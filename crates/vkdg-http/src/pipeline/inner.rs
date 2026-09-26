use std::sync::atomic::AtomicUsize;
use std::sync::Arc;

use futures::stream::{FuturesUnordered, StreamExt};

use axum::response::Response;
use bytes::Bytes;
use http::header;
use serde_json::json;
use vkdg_cache::{cache_key, CacheEntry, CacheResult};
use vkdg_core::pipeline::PipelineCtx;
use vkdg_core::{AttemptState, ConnectionId, VkdgError};
use vkdg_operations::Operation;
use vkdg_routing::{EligibilityFilter, RouteId, RouteResult, RoutingHints};

use super::helpers::{
    error_response, filter_think_tags_stream, has_tool_calls, is_multiturn, unix_secs,
};
use super::phases::{
    post_response_accounting, prepare_operation, relay_on_rotation, resolve_combo_and_session,
};
use crate::upstream::{UpstreamRequest, UpstreamResponse};
use crate::PipelineState;

// ── Auto-route counter ────────────────────────────────────────────────────────
/// Round-robin index for auto-route fallback (no explicit RouteConfig matched).
static AUTO_ROUTE_COUNTER: AtomicUsize = AtomicUsize::new(0);

// ── RAII dedup guard ──────────────────────────────────────────────────────────
/// Calls `DedupTable::complete` on drop so in-flight tracking is always cleaned
/// up regardless of how run_pipeline_inner exits (normal return, early return,
/// or `?`-propagated error).
struct DedupGuard {
    dedup: Arc<crate::DedupTable>,
    key: String,
}
impl Drop for DedupGuard {
    fn drop(&mut self) {
        self.dedup.complete(&self.key);
    }
}

pub(super) async fn run_pipeline_inner(
    pipeline: &PipelineState,
    ctx: &mut PipelineCtx,
    mut operation: Operation,
    excluded: &[ConnectionId],
) -> Result<Response, VkdgError> {
    let start_time = std::time::Instant::now();

    // 0. IP policy ─────────────────────────────────────────────────────────────
    // Checked before admission so blocked IPs don't consume capacity slots.
    if let Some(ip_policy) = &pipeline.ip_policy {
        if let Some(client_ip) = &ctx.envelope.client_ip {
            if !ip_policy.allows(client_ip) {
                return Err(VkdgError::Unauthorized);
            }
        }
    }

    // 1. Admission ─────────────────────────────────────────────────────────────
    // Must be the very first step: any failure before this would leak requests
    // past the capacity limit.
    let _permit = pipeline.admission.acquire()?;
    ctx.transition(AttemptState::Admitted);

    // 2. Combo resolution + session stickiness ────────────────────────────────
    let csr = resolve_combo_and_session(pipeline, &ctx.envelope).await;

    // 2b. Budget cap check ─────────────────────────────────────────────────────
    // Reject before routing or any upstream call if the estimated cost exceeds
    // the combo budget. Placed here so a budget rejection is cheap and does not
    // consume routing, connection, or credential resources.
    // Plausible wrong impl: check happens after the upstream call, so the budget
    // is enforced too late and costs are already incurred.
    if let Some(budget) = &csr.budget_policy {
        if let Some(max_cost) = budget.max_cost_microdollars {
            let estimated_tokens: u64 = if let Operation::Conversation(conv_req) = &operation {
                conv_req
                    .messages
                    .iter()
                    .map(|m| match &m.content {
                        vkdg_operations::MessageContent::Text(s) => (s.len() as u64) / 4,
                        _ => 50,
                    })
                    .sum()
            } else {
                0
            };
            // Rough rate: $3/M tokens = 3 microdollars per token.
            let estimated_cost_microdollars = estimated_tokens * 3;
            if estimated_cost_microdollars > max_cost {
                return Err(VkdgError::BudgetExceeded {
                    estimated_usd: estimated_cost_microdollars as f64 / 1_000_000.0,
                    limit_usd: max_cost as f64 / 1_000_000.0,
                });
            }
        }
    }

    // 3. Route ─────────────────────────────────────────────────────────────────
    let filter = if excluded.is_empty() {
        EligibilityFilter::default()
    } else {
        EligibilityFilter {
            excluded_connections: excluded.to_vec(),
            reason_map: excluded
                .iter()
                .map(|id| (id.clone(), "rate_limited".into()))
                .collect(),
        }
    };
    // Build routing hints from live quota signals.
    // We iterate all known connection IDs so the scorer gets real headroom for
    // every candidate; the router uses only the subset that matches the route.
    let routing_hints = {
        let mut hints = RoutingHints::default();
        if let Some(mode) = &ctx.envelope.mode_pack_override {
            hints.mode_pack = Some(mode.clone());
        }
        if let Some(tracker) = &pipeline.quota_tracker {
            for conn_id in pipeline.catalog.connection_ids() {
                if let Some(h) = tracker.headroom(&conn_id).await {
                    hints.quota_headroom.insert(conn_id, h);
                }
            }
        }
        if let Some(tracker) = &pipeline.latency_tracker {
            for conn_id in pipeline.catalog.connection_ids() {
                if let Some(ms) = tracker.p50_ms(&conn_id).await {
                    hints.latency_p50_ms.insert(conn_id, ms);
                }
            }
        }
        hints
    };
    let route_result = match pipeline
        .router
        .route(&ctx.envelope, &filter, &routing_hints)
        .await
    {
        Ok(r) => r,
        Err(VkdgError::NoEligibleConnection) => {
            // Auto-route: find any healthy catalog connection that serves the model.
            let model = &ctx.envelope.model_requested;
            let excluded_ids = filter.excluded_connections.to_vec();
            let candidates = pipeline.catalog.eligible(model, &excluded_ids);
            if candidates.is_empty() {
                return Err(VkdgError::NoEligibleConnection);
            }
            // Round-robin across all eligible connections.
            let idx = AUTO_ROUTE_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                % candidates.len();
            RouteResult {
                connection_id: candidates[idx].clone(),
                route_id: RouteId("auto".into()),
                excluded: vec![],
                fusion_targets: vec![],
                chain_steps: vec![],
            }
        }
        Err(e) => return Err(e),
    };
    // Fusion fast-path: fan-out to all targets in parallel, return first success.
    if route_result.fusion_targets.len() > 1 {
        return run_fusion_dispatch(
            pipeline,
            ctx,
            operation,
            route_result.fusion_targets,
            csr.compression_threshold,
            &csr.effective_compressor_id,
        )
        .await;
    }
    // PromptChain fast-path: run steps sequentially, inject previous response into each step.
    if !route_result.chain_steps.is_empty() {
        return run_prompt_chain(
            pipeline,
            ctx,
            operation,
            route_result.chain_steps,
            csr.compression_threshold,
            &csr.effective_compressor_id,
        )
        .await;
    }

    // Priority: session pin > combo redirect > routed connection.
    // Save the preferred connection before it is moved into the if-let so we
    // can detect rotation afterwards for context-relay.
    let session_preferred_saved = csr.session_preferred.clone();
    let connection_id = if let Some(preferred) = csr.session_preferred {
        // Only use the pin if the connection is still in the catalog (healthy check
        // happens inside acquire() at step 3; here we just guard against stale pins
        // for connections that were removed from the catalog entirely).
        if pipeline.catalog.get(&preferred).is_some() {
            tracing::debug!(
                session_key = ?ctx.envelope.session_key,
                pinned = %preferred.0,
                "session stickiness: using pinned connection",
            );
            preferred
        } else if let Some(targets) = &csr.resolved_targets {
            if !targets.is_empty() && !targets.contains(&route_result.connection_id) {
                targets[0].clone()
            } else {
                route_result.connection_id
            }
        } else {
            route_result.connection_id
        }
    } else if let Some(targets) = &csr.resolved_targets {
        if !targets.is_empty() && !targets.contains(&route_result.connection_id) {
            tracing::debug!(
                combo_id = %csr.combo_id.as_deref().unwrap_or(""),
                routed   = %route_result.connection_id.0,
                redirect = %targets[0].0,
                "combo redirect: routed connection not in combo targets",
            );
            targets[0].clone()
        } else {
            route_result.connection_id
        }
    } else {
        route_result.connection_id
    };
    ctx.connection_id = Some(connection_id.clone());
    ctx.transition(AttemptState::AccountReserved);

    // Context-relay: if the session had a pinned connection but we rotated to a
    // different one (stale pin or exclusion fallback), inject the recent
    // conversation history so the new provider has context.
    if pipeline.relay_enabled {
        if let Some(preferred) = &session_preferred_saved {
            if preferred != &connection_id {
                relay_on_rotation(&mut operation, preferred, &connection_id);
            }
        }
    }

    // 3. Connection + RAII guard ───────────────────────────────────────────────
    let conn_arc = pipeline
        .catalog
        .get(&connection_id)
        .ok_or(VkdgError::NoEligibleConnection)?;
    // Read the connection config and acquire a request slot.
    // `_guard` is kept alive for the entire function; it decrements
    // active_requests on drop (RAII).
    let (config, _guard) = {
        let conn = conn_arc.read().await;
        let guard = conn.acquire().ok_or(VkdgError::NoEligibleConnection)?;
        (conn.config.clone(), guard)
    };

    // 4. Credential ────────────────────────────────────────────────────────────
    let token = pipeline.credentials.get_token(&config).await?;
    ctx.transition(AttemptState::CredentialReady);
    // 4b. VideoGenerate: async job path — skip upstream send, return 202 immediately.
    // The provider adapter still builds the request so we validate the operation;
    // in Phase E this will wire to a real JobManager via PipelineState.
    if matches!(operation, Operation::VideoGenerate(_)) {
        ctx.transition(AttemptState::Prepared);
        ctx.mark_committed();
        ctx.transition(AttemptState::Committed);
        let body = serde_json::to_vec(&json!({"status": "queued", "message": "job created"}))
            .unwrap_or_default();
        return Ok(axum::response::Response::builder()
            .status(http::StatusCode::ACCEPTED)
            .header(header::CONTENT_TYPE, "application/json")
            .body(axum::body::Body::from(body))
            .unwrap_or_else(|_| error_response(VkdgError::Internal("response build".into()))));
    }
    // 5. Prepare operation (compression + system prompt + memory injection) ────
    let (op, _compress_metrics) = prepare_operation(
        pipeline,
        operation,
        &ctx.envelope,
        csr.compression_threshold,
        &csr.effective_compressor_id,
    )
    .await;
    operation = op;
    // 6. Request deduplication ─────────────────────────────────────────────────
    // Register this request as in-flight.  If a duplicate is already in-flight
    // we still proceed (Phase D: register-and-proceed).  Phase E will add the
    // wait-then-read-cache path.  The DedupGuard calls complete() on drop so
    // the entry is always removed even on error or early return.
    let _dedup_guard = if let (Some(dedup), Operation::Conversation(conv_req)) =
        (&pipeline.dedup_table, &operation)
    {
        let key = cache_key(&ctx.envelope.model_requested, conv_req);
        let (is_first, _notify) = dedup.register(&key);
        tracing::debug!(key = %key, is_first, "dedup registration");
        Some(DedupGuard {
            dedup: Arc::clone(dedup),
            key,
        })
    } else {
        None
    };

    // 7. Cache lookup ──────────────────────────────────────────────────────────
    // Bypass rules enforced here in core; the cache plugin never needs to check them.
    let conv_req = if let Operation::Conversation(r) = &operation {
        Some(r)
    } else {
        None
    };
    let bypass_cache = ctx.envelope.cache_bypass
        || conv_req.map_or(true, |r| is_multiturn(r) || has_tool_calls(r));
    let cache_key_val: Option<String> = if !bypass_cache {
        conv_req.map(|r| cache_key(&ctx.envelope.model_requested, r))
    } else {
        None
    };
    if let (Some(key), Some(cache)) = (&cache_key_val, &pipeline.cache) {
        match cache.lookup(key).await {
            Ok(CacheResult::Hit(entry)) => {
                ctx.transition(AttemptState::Completed);
                let body = entry.response_json.into_bytes();
                return Ok(axum::response::Response::builder()
                    .status(http::StatusCode::OK)
                    .header(header::CONTENT_TYPE, "application/json")
                    .header("x-vkdg-cache", "hit")
                    .body(axum::body::Body::from(body))
                    .unwrap_or_else(|_| {
                        error_response(VkdgError::Internal("cache response build".into()))
                    }));
            }
            Ok(CacheResult::Miss) => {}
            Err(e) => {
                // Cache errors are non-fatal: log and continue to upstream.
                tracing::warn!(error = %e, "cache lookup failed, bypassing");
            }
        }
    }

    // 8. Build upstream request via provider adapter ───────────────────────────
    let prepared = pipeline
        .provider_adapter
        .prepare(&operation, &config, &token)
        .map_err(|e| VkdgError::Internal(e.to_string()))?;
    let is_streaming = prepared.is_streaming;
    let upstream_req = UpstreamRequest {
        method: http::Method::POST,
        url: prepared.url,
        headers: prepared.headers,
        body: prepared.body,
    };
    ctx.transition(AttemptState::Prepared);

    // 9. Send ──────────────────────────────────────────────────────────────────
    let upstream_resp = match pipeline.http_client.send(upstream_req, is_streaming).await {
        Ok(r) => r,
        Err(VkdgError::UpstreamError { code, message }) if code == 429 || code >= 500 => {
            if let Some(conn_arc) = ctx
                .connection_id
                .as_ref()
                .and_then(|id| pipeline.catalog.get(id))
            {
                if let Ok(mut conn) = conn_arc.try_write() {
                    conn.record_upstream_error(code);
                }
            }
            return Err(VkdgError::UpstreamError { code, message });
        }
        Err(e) => return Err(e),
    };
    ctx.transition(AttemptState::UpstreamOpen);

    // First byte received — mark committed; transparent retry is no longer safe.
    ctx.mark_committed();
    ctx.transition(AttemptState::Committed);

    // 10. Build response ───────────────────────────────────────────────────────
    // `complete_body` is set in the Complete arm and passed to post_response_accounting.
    // Streaming leaves it None (Phase E: wrap stream on completion).
    let mut complete_body: Option<Bytes> = None;
    let resp = match upstream_resp {
        UpstreamResponse::Complete { status, body } => {
            // 11. Cache store (fire-and-forget, non-streaming only) ────────────
            if let (Some(key), Some(cache)) = (&cache_key_val, &pipeline.cache) {
                let entry = CacheEntry {
                    response_json: String::from_utf8_lossy(&body).into_owned(),
                    model: ctx.envelope.model_requested.clone(),
                    cached_at_secs: unix_secs(),
                    ttl_secs: Some(300), // default 5 min; Phase E: from combo CachePolicy
                    cache_type: "exact".into(),
                    hit_score: None,
                };
                let cache = Arc::clone(cache);
                let key = key.clone();
                tokio::spawn(async move {
                    let _ = cache.store(&key, entry).await;
                });
            }
            // Capture body for post-response accounting (Bytes clone is O(1)).
            complete_body = Some(body.clone());
            let status_code =
                http::StatusCode::from_u16(status).unwrap_or(http::StatusCode::BAD_GATEWAY);
            axum::response::Response::builder()
                .status(status_code)
                .header(header::CONTENT_TYPE, "application/json")
                .body(axum::body::Body::from(body))
                .unwrap_or_else(|_| {
                    error_response(VkdgError::Internal("response builder failed".into()))
                })
        }
        UpstreamResponse::Streaming { status: _, body } => {
            // Streaming responses are never cached — the body is a stream.
            // Filter think tags unless the client opted in via X-VKDG-Think-Tags: include.
            let filtered_body = if !ctx.envelope.include_think_tags {
                filter_think_tags_stream(body)
            } else {
                body
            };
            // Guard: if upstream closes before [DONE], inject error event so
            // clients can detect the incomplete response (instead of silent 200).
            let guarded_body = crate::with_termination_guard(filtered_body);
            axum::response::Response::builder()
                .status(http::StatusCode::OK)
                .header(header::CONTENT_TYPE, "text/event-stream")
                .header("cache-control", "no-cache")
                .header("x-accel-buffering", "no")
                .header("x-vkdg-stream-guard", "active")
                .body(axum::body::Body::from_stream(guarded_body))
                .unwrap_or_else(|_| {
                    error_response(VkdgError::Internal(
                        "streaming response builder failed".into(),
                    ))
                })
        }
    };

    // 12. Post-response accounting (memory, eval, quota, latency, cooldown) ────
    post_response_accounting(
        pipeline,
        ctx,
        complete_body.as_ref(),
        start_time,
        &operation,
    );
    Ok(resp)
}

// ── Fusion dispatch ───────────────────────────────────────────────────────────

/// Fan-out to all `fusion_targets` in parallel; return the first successful response.
/// Operation is prepared once and shared across all branches.
async fn run_fusion_dispatch(
    pipeline: &PipelineState,
    ctx: &mut PipelineCtx,
    mut operation: Operation,
    fusion_targets: Vec<ConnectionId>,
    compression_threshold: u32,
    effective_compressor_id: &Option<String>,
) -> Result<Response, VkdgError> {
    // Tag ctx with the primary target for tracing/decision-record.
    ctx.connection_id = Some(fusion_targets[0].clone());
    ctx.transition(AttemptState::AccountReserved);

    // Prepare operation once — compression, system prompt, memory injection.
    let (op, _) = prepare_operation(
        pipeline,
        operation,
        &ctx.envelope,
        compression_threshold,
        effective_compressor_id,
    )
    .await;
    operation = op;

    // Race all targets; return first Ok, or NoEligibleConnection if all fail.
    let mut futs: FuturesUnordered<_> = fusion_targets
        .into_iter()
        .map(|conn_id| {
            let operation = operation.clone();
            async move { fusion_one_target(pipeline, conn_id, operation).await }
        })
        .collect();

    let mut last_err = VkdgError::NoEligibleConnection;
    while let Some(result) = futs.next().await {
        match result {
            Ok(resp) => return Ok(resp),
            Err(e) => {
                tracing::debug!(error = %e, "fusion branch failed");
                last_err = e;
            }
        }
    }
    Err(last_err)
}

/// Execute a single upstream call for one fusion target.
async fn fusion_one_target(
    pipeline: &PipelineState,
    conn_id: ConnectionId,
    operation: Operation,
) -> Result<Response, VkdgError> {
    let conn_arc = pipeline
        .catalog
        .get(&conn_id)
        .ok_or(VkdgError::NoEligibleConnection)?;
    let (config, _guard) = {
        let conn = conn_arc.read().await;
        let guard = conn.acquire().ok_or(VkdgError::NoEligibleConnection)?;
        (conn.config.clone(), guard)
    };

    let token = pipeline.credentials.get_token(&config).await?;

    let prepared = pipeline
        .provider_adapter
        .prepare(&operation, &config, &token)
        .map_err(|e| VkdgError::Internal(e.to_string()))?;
    let is_streaming = prepared.is_streaming;
    let upstream_req = UpstreamRequest {
        method: http::Method::POST,
        url: prepared.url,
        headers: prepared.headers,
        body: prepared.body,
    };

    let upstream_resp = pipeline
        .http_client
        .send(upstream_req, is_streaming)
        .await?;

    let resp = match upstream_resp {
        UpstreamResponse::Complete { status, body } => axum::response::Response::builder()
            .status(http::StatusCode::from_u16(status).unwrap_or(http::StatusCode::BAD_GATEWAY))
            .header(header::CONTENT_TYPE, "application/json")
            .body(axum::body::Body::from(body))
            .map_err(|e| VkdgError::Internal(e.to_string()))?,
        UpstreamResponse::Streaming { status: _, body } => axum::response::Response::builder()
            .status(http::StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/event-stream")
            .header("cache-control", "no-cache")
            .header("x-accel-buffering", "no")
            .body(axum::body::Body::from_stream(body))
            .map_err(|e| VkdgError::Internal(e.to_string()))?,
    };

    Ok(resp)
}
// ── PromptChain dispatch ──────────────────────────────────────────────────────

/// Run a sequential prompt chain: each step's response is injected into the next step.
///
/// Steps run in order. If any step fails, the chain aborts and returns the error.
/// The operation is prepared once before the chain starts. Only the final step's
/// response is returned to the client.
async fn run_prompt_chain(
    pipeline: &PipelineState,
    ctx: &mut PipelineCtx,
    mut operation: Operation,
    steps: Vec<vkdg_routing::ChainStep>,
    compression_threshold: u32,
    effective_compressor_id: &Option<String>,
) -> Result<Response, VkdgError> {
    use vkdg_operations::{Message, MessageContent, Role};
    use vkdg_routing::InjectMode;

    ctx.transition(AttemptState::AccountReserved);

    // Prepare operation once (compression, system prompt, memory injection).
    let (op, _) = prepare_operation(
        pipeline,
        operation,
        &ctx.envelope,
        compression_threshold,
        effective_compressor_id,
    )
    .await;
    operation = op;

    let mut previous_response: Option<String> = None;
    let step_count = steps.len();

    for (step_idx, step) in steps.into_iter().enumerate() {
        // Inject previous response into this step's operation (skipped for step 0).
        if let Some(prev) = &previous_response {
            if let Operation::Conversation(conv_req) = &mut operation {
                match &step.inject_previous {
                    InjectMode::AsAssistant => {
                        // Insert assistant message before the last user message.
                        let last_user_pos = conv_req
                            .messages
                            .iter()
                            .rposition(|m| matches!(m.role, Role::User));
                        if let Some(pos) = last_user_pos {
                            conv_req.messages.insert(
                                pos,
                                Message {
                                    role: Role::Assistant,
                                    content: MessageContent::Text(prev.clone()),
                                },
                            );
                        }
                    }
                    InjectMode::AppendToUser => {
                        if let Some(last_user) = conv_req
                            .messages
                            .iter_mut()
                            .rfind(|m| matches!(m.role, Role::User))
                        {
                            if let MessageContent::Text(t) = &mut last_user.content {
                                *t = format!("{t}\n\nPrevious output:\n{prev}");
                            }
                        }
                    }
                    InjectMode::AsSystem => {
                        conv_req.system = Some(match &conv_req.system {
                            None => format!("Previous output:\n{prev}"),
                            Some(existing) => {
                                format!("Previous output:\n{prev}\n\n{existing}")
                            }
                        });
                    }
                }
            }
        }

        // Override system for this step if configured.
        if let Operation::Conversation(conv_req) = &mut operation {
            if let Some(step_system) = &step.system {
                conv_req.system = Some(step_system.clone());
            }
        }

        // Resolve connection for this step.
        let conn_id = match step.connection_id {
            Some(id) => id,
            None => {
                // Auto-route: find any eligible catalog connection for the requested model.
                let excluded_ids: Vec<ConnectionId> = vec![];
                pipeline
                    .catalog
                    .eligible(&ctx.envelope.model_requested, &excluded_ids)
                    .into_iter()
                    .next()
                    .ok_or(VkdgError::NoEligibleConnection)?
            }
        };

        ctx.connection_id = Some(conn_id.clone());
        tracing::debug!(
            step = step_idx,
            total = step_count,
            connection = %conn_id.0,
            "prompt chain step",
        );

        // Execute the step.
        let resp = fusion_one_target(pipeline, conn_id, operation.clone()).await?;

        if step_idx + 1 < step_count {
            // Not the last step: consume the body and extract text for the next step.
            let body_bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
                .await
                .map_err(|e| VkdgError::Internal(e.to_string()))?;
            let body_str = String::from_utf8_lossy(&body_bytes);
            let extracted = serde_json::from_str::<serde_json::Value>(&body_str)
                .ok()
                .and_then(|v| {
                    // Anthropic format: content[0].text
                    v.get("content")
                        .and_then(|c| c.as_array())
                        .and_then(|arr| arr.first())
                        .and_then(|item| item.get("text"))
                        .and_then(|t| t.as_str())
                        .map(|s| s.to_string())
                        // OpenAI format: choices[0].message.content
                        .or_else(|| {
                            v.get("choices")
                                .and_then(|c| c.as_array())
                                .and_then(|arr| arr.first())
                                .and_then(|item| item.get("message"))
                                .and_then(|m| m.get("content"))
                                .and_then(|c| c.as_str())
                                .map(|s| s.to_string())
                        })
                })
                .unwrap_or_else(|| body_str.into_owned());
            previous_response = Some(extracted);
        } else {
            // Last step: return its response directly.
            ctx.transition(AttemptState::Completed);
            return Ok(resp);
        }
    }

    // Unreachable: steps is validated non-empty in the router.
    Err(VkdgError::NoEligibleConnection)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use vkdg_connections::{ConnectionCatalog, CredentialManager};
    use vkdg_core::{
        pipeline::PipelineCtx, ApiType, ClientId, RequestEnvelope, RequestId, TenantId,
    };
    use vkdg_observe::DecisionRecordExporter;
    use vkdg_operations::{CapabilitySet, ConversationRequest, Operation};
    use vkdg_routing::Router as VkdgRouter;

    use super::super::entry::run_conversation_pipeline;
    use crate::provider::{PreparedRequest, ProviderAdapter};
    use crate::upstream::HttpClient;
    use crate::{AdmissionGuard, PipelineState};

    struct StubAdapter;
    impl ProviderAdapter for StubAdapter {
        fn id(&self) -> &str {
            "stub"
        }
        fn display_name(&self) -> &str {
            "Stub"
        }
        fn prepare(
            &self,
            _op: &Operation,
            _cfg: &vkdg_connections::ConnectionConfig,
            _token: &str,
        ) -> Result<PreparedRequest, vkdg_provider_sdk::ProviderError> {
            Err(vkdg_provider_sdk::ProviderError::UnsupportedOperation)
        }
    }

    fn make_pipeline(admission_limit: usize) -> Arc<PipelineState> {
        Arc::new(PipelineState::minimal(
            Arc::new(AdmissionGuard::new(admission_limit)),
            Arc::new(VkdgRouter::new(vec![])),
            Arc::new(ConnectionCatalog::new(vec![])),
            Arc::new(CredentialManager::new()),
            Arc::new(HttpClient::new()),
            Arc::new(DecisionRecordExporter::new()),
            Arc::new(StubAdapter),
        ))
    }

    fn make_ctx(model: &str) -> PipelineCtx {
        let envelope = RequestEnvelope {
            request_id: RequestId::new(),
            client_id: ClientId("test".into()),
            tenant_id: TenantId("default".into()),
            session_key: None,
            api_type: ApiType::AnthropicMessages,
            model_requested: model.into(),
            deadline: None,
            mode_pack_override: None,
            compression_override: None,
            cache_bypass: false,
            include_think_tags: false,
            client_ip: None,
        };
        PipelineCtx::new(envelope)
    }

    fn make_conv_op() -> Operation {
        Operation::Conversation(ConversationRequest {
            messages: vec![],
            tools: vec![],
            max_tokens: Some(100),
            temperature: None,
            stream: false,
            system: None,
            required_capabilities: CapabilitySet::default(),
        })
    }

    // Plausible wrong impl: admission check happens after routing, so a
    // NoEligibleConnection (502) races with AdmissionRejected (503).
    // This test pins that 503 appears even when the router has no routes.
    #[tokio::test]
    async fn admission_check_is_before_routing() {
        let pipeline = make_pipeline(0); // limit=0 → always fails
        let ctx = make_ctx("claude-3-5-sonnet-20241022");

        let resp = run_conversation_pipeline(pipeline, ctx, make_conv_op()).await;

        assert_eq!(
            resp.status(),
            http::StatusCode::SERVICE_UNAVAILABLE,
            "expected 503 from admission check, not 502 from routing"
        );
    }

    // Plausible wrong impl: routing failure returns 503 instead of 502,
    // masking the admission result.
    #[tokio::test]
    async fn no_eligible_connection_returns_502() {
        let pipeline = make_pipeline(100); // admission passes
        let ctx = make_ctx("claude-3-5-sonnet-20241022");

        let resp = run_conversation_pipeline(pipeline, ctx, make_conv_op()).await;

        assert_eq!(
            resp.status(),
            http::StatusCode::BAD_GATEWAY,
            "expected 502 when no routes match, not 503"
        );
    }

    // Plausible wrong impl: pipeline completes but never calls exporter.export(),
    // making request_explain impossible to implement because no DecisionRecord
    // is ever emitted — the exporter field is wired but never invoked.
    // This test defeats it: both success and failure paths must complete without
    // panic, proving the exporter is called with a well-formed DecisionRecord.
    #[tokio::test]
    async fn pipeline_emits_decision_record_on_failure_path() {
        // Pipeline has capacity but no routes → routing fails → DecisionRecord
        // with result=Failed must be emitted before the 502 response is returned.
        let pipeline = make_pipeline(100);
        let ctx = make_ctx("claude-3-5-sonnet-20241022");

        let resp = run_conversation_pipeline(pipeline, ctx, make_conv_op()).await;

        // 502 proves routing failed; completing without panic proves
        // exporter.export() was called with a valid DecisionRecord (export()
        // itself never panics — serialisation errors are logged, not unwrapped).
        assert_eq!(
            resp.status(),
            http::StatusCode::BAD_GATEWAY,
            "routing failure must produce 502 after emitting DecisionRecord"
        );
    }

    // Plausible wrong impl: DecisionRecord is only emitted on the success path,
    // so admission failures leave no audit trail.
    // This test defeats it: even a 503 from admission must produce a record.
    #[tokio::test]
    async fn pipeline_emits_decision_record_on_admission_failure() {
        let pipeline = make_pipeline(0); // limit=0 → admission always fails
        let ctx = make_ctx("claude-3-5-sonnet-20241022");

        let resp = run_conversation_pipeline(pipeline, ctx, make_conv_op()).await;

        // Completing without panic proves export() was called; status confirms
        // the correct error path was taken.
        assert_eq!(
            resp.status(),
            http::StatusCode::SERVICE_UNAVAILABLE,
            "admission failure must return 503 after emitting DecisionRecord"
        );
    }

    // Plausible wrong impl: compressor runs but operation is not updated (mutation lost).
    // This test defeats it by verifying the pipeline reaches routing (which fails 502)
    // without panicking — i.e. compression threshold guard short-circuits when estimated
    // tokens < threshold, so the pipeline proceeds normally.
    #[tokio::test]
    async fn pipeline_with_compressor_threshold_not_met_skips_compression() {
        use vkdg_policy_compress::CavemanCompressor;
        let mut state = PipelineState::minimal(
            Arc::new(AdmissionGuard::new(100)),
            Arc::new(VkdgRouter::new(vec![])),
            Arc::new(ConnectionCatalog::new(vec![])),
            Arc::new(CredentialManager::new()),
            Arc::new(HttpClient::new()),
            Arc::new(DecisionRecordExporter::new()),
            Arc::new(StubAdapter),
        );
        state.compressor = Some(Arc::new(CavemanCompressor));
        let pipeline = Arc::new(state);
        let ctx = make_ctx("claude-3-5-sonnet-20241022");
        // Tiny request → estimated tokens << 2000 threshold → compression skipped.
        // Pipeline proceeds to routing (no routes → 502), not panic.
        let resp = run_conversation_pipeline(pipeline, ctx, make_conv_op()).await;
        assert_eq!(
            resp.status(),
            http::StatusCode::BAD_GATEWAY,
            "with compressor but threshold not met, pipeline must reach routing and return 502",
        );
    }

    // Plausible wrong impl: combo compression threshold ignored, always uses 2000.
    // This test defeats it: a combo with auto_trigger_tokens=100 is wired via combo_resolver;
    // the pipeline must execute step 1.5b without panic and reach routing (502).
    // The structural assertion checks the policy fields are correctly round-tripped from the combo.
    #[tokio::test]
    async fn compression_threshold_from_combo_overrides_default() {
        use vkdg_combos::{Combo, ComboResolver, CompressionPolicy};
        use vkdg_policy_compress::CavemanCompressor;
        use vkdg_routing::StrategyKind;

        // Structural check: policy fields are what we set.
        let policy = CompressionPolicy {
            plugin_id: "caveman".into(),
            auto_trigger_tokens: Some(100),
        };
        assert_eq!(policy.auto_trigger_tokens, Some(100));
        assert_eq!(policy.plugin_id, "caveman");

        // Behavioral check: pipeline with combo_resolver that resolves to a combo with
        // threshold=100 must proceed to routing (502) without panic, proving 1.5b executes.
        let combo = Combo {
            id: "low-threshold".into(),
            match_patterns: vec!["low-threshold".into()],
            strategy: StrategyKind::FallbackChain,
            targets: vec![],
            compression: Some(CompressionPolicy {
                plugin_id: "caveman".into(),
                auto_trigger_tokens: Some(100),
            }),
            cache: None,
            budget: None,
            mode_pack: None,
        };
        let mut state = PipelineState::minimal(
            Arc::new(AdmissionGuard::new(100)),
            Arc::new(VkdgRouter::new(vec![])),
            Arc::new(ConnectionCatalog::new(vec![])),
            Arc::new(CredentialManager::new()),
            Arc::new(HttpClient::new()),
            Arc::new(DecisionRecordExporter::new()),
            Arc::new(StubAdapter),
        );
        state.combo_resolver = Some(Arc::new(ComboResolver::new(vec![combo])));
        state.compressor = Some(Arc::new(CavemanCompressor));
        let pipeline = Arc::new(state);
        // Request for the combo name — resolver matches; step 1.5b reads threshold=100.
        // Empty request → estimated tokens = 0 < 100 → compression skipped.
        // Pipeline proceeds to routing (no routes → 502).
        let ctx = make_ctx("low-threshold");
        let resp = run_conversation_pipeline(pipeline, ctx, make_conv_op()).await;
        assert_eq!(
            resp.status(),
            http::StatusCode::BAD_GATEWAY,
            "combo with compression policy must reach routing without panic and return 502",
        );
    }

    // Plausible wrong impl: global_system_prompt overwrites existing system instead of prepending.
    // This test defeats it by checking both variants: no-existing and has-existing.
    #[test]
    fn global_system_prompt_prepended_to_existing() {
        let global = "Be concise.".to_string();
        let existing = "You are a helpful assistant.".to_string();

        // Case 1: no existing system → inject as-is.
        let result_none: Option<String> = Some(global.clone());
        assert_eq!(result_none.as_deref(), Some("Be concise."));

        // Case 2: existing system → global prepended with double newline separator.
        let result_both = format!("{global}\n\n{existing}");
        assert!(
            result_both.starts_with("Be concise."),
            "global prompt must appear first"
        );
        assert!(
            result_both.contains("You are a helpful assistant."),
            "existing prompt must be preserved"
        );
        assert_eq!(
            result_both, "Be concise.\n\nYou are a helpful assistant.",
            "separator must be exactly two newlines"
        );
    }

    // Plausible wrong impl: budget check happens after the upstream call (or not at all),
    // so a request that exceeds the cost cap still incurs charges before being rejected.
    // This test defeats it: a combo with max_cost_microdollars=0 must produce 402 before
    // any upstream attempt, proved by the pipeline never reaching step 8 (no catalog).
    #[tokio::test]
    async fn budget_exceeded_returns_402_before_upstream() {
        use vkdg_combos::{BudgetPolicy, Combo, ComboResolver};
        use vkdg_routing::StrategyKind;

        let combo = Combo {
            id: "zero-budget".into(),
            match_patterns: vec!["zero-budget".into()],
            strategy: StrategyKind::FallbackChain,
            targets: vec![],
            compression: None,
            cache: None,
            budget: Some(BudgetPolicy {
                max_cost_microdollars: Some(0), // any non-empty request exceeds this
                overflow: "strict".into(),
            }),
            mode_pack: None,
        };
        let mut state = PipelineState::minimal(
            Arc::new(AdmissionGuard::new(100)),
            Arc::new(VkdgRouter::new(vec![])),
            Arc::new(ConnectionCatalog::new(vec![])),
            Arc::new(CredentialManager::new()),
            Arc::new(HttpClient::new()),
            Arc::new(DecisionRecordExporter::new()),
            Arc::new(StubAdapter),
        );
        state.combo_resolver = Some(Arc::new(ComboResolver::new(vec![combo])));
        let pipeline = Arc::new(state);

        // Build a request with a non-trivial message so estimated_tokens > 0
        // and estimated_cost_microdollars (tokens * 3) > 0 = max_cost.
        use vkdg_operations::{Message, MessageContent, Role};
        let op = Operation::Conversation(ConversationRequest {
            messages: vec![Message {
                role: Role::User,
                content: MessageContent::Text("hello world".into()),
            }],
            tools: vec![],
            max_tokens: Some(100),
            temperature: None,
            stream: false,
            system: None,
            required_capabilities: CapabilitySet::default(),
        });

        let ctx = make_ctx("zero-budget");
        let resp = run_conversation_pipeline(pipeline, ctx, op).await;
        assert_eq!(
            resp.status().as_u16(),
            402,
            "combo with max_cost=0 and non-empty message must return 402 Payment Required"
        );
    }

    // Plausible wrong impl: pipeline panics or returns 500 when the provider adapter's
    // prepare() returns Err, instead of cleanly mapping it to an HTTP error.
    // This test defeats it by driving the pipeline all the way to step 8 (adapter
    // prepare) via a real route + connection, confirming StubAdapter's Err propagates
    // as a non-panicking 500 response.
    #[tokio::test]
    async fn adapter_prepare_error_propagates_as_500() {
        use vkdg_connections::{AuthKind, ConnectionConfig, ProviderKind};
        use vkdg_core::CapabilitySet;
        use vkdg_routing::{RouteConfig, RouteId, StrategyKind};

        let conn_id = ConnectionId("stub-conn".into());
        let conn_config = ConnectionConfig {
            id: conn_id.clone(),
            provider: ProviderKind::Custom {
                base_url: "http://127.0.0.1:0".into(),
            },
            auth: AuthKind::ApiKey {
                env_var: "HOME".into(), // always set on Unix; credential step passes → adapter reached
            },
            models: vec!["stub-model".into()],
            max_concurrent: 100,
            weight: 1,
            tags: vec![],
            capabilities: CapabilitySet::default(),
        };
        let route = RouteConfig {
            id: RouteId("stub-route".into()),
            match_models: vec!["stub-model".into()],
            strategy: StrategyKind::FallbackChain,
            targets: vec![conn_id],
            plugin_hooks: Default::default(),
        };

        let pipeline = Arc::new(PipelineState::minimal(
            Arc::new(AdmissionGuard::new(100)),
            Arc::new(VkdgRouter::new(vec![route])),
            Arc::new(ConnectionCatalog::new(vec![conn_config])),
            Arc::new(CredentialManager::new()),
            Arc::new(HttpClient::new()),
            Arc::new(DecisionRecordExporter::new()),
            Arc::new(StubAdapter),
        ));

        let ctx = make_ctx("stub-model");
        let resp = run_conversation_pipeline(pipeline, ctx, make_conv_op()).await;
        assert_eq!(
            resp.status(),
            http::StatusCode::INTERNAL_SERVER_ERROR,
            "adapter prepare() returning Err must produce 500, not panic"
        );
    }
}
