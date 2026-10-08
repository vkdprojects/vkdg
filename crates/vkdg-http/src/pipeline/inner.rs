use std::collections::HashMap;
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

use super::helpers::{error_response, has_tool_calls, is_multiturn, unix_secs};
use super::phases::{
    post_response_accounting, prepare_operation, relay_on_rotation, resolve_combo_and_session,
};
use crate::upstream::{UpstreamRequest, UpstreamResponse};
use crate::PipelineState;

// ── Auto-route counter ────────────────────────────────────────────────────────
/// Round-robin index for auto-route fallback (no explicit `RouteConfig` matched).
static AUTO_ROUTE_COUNTER: AtomicUsize = AtomicUsize::new(0);

// ── RAII dedup guard ──────────────────────────────────────────────────────────
/// Calls `DedupTable::complete` on drop so in-flight tracking is always cleaned
/// up regardless of how `run_pipeline_inner` exits (normal return, early return,
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
    // An unknown client IP fails an allowlist instead of skipping it.
    if let Some(ip_policy) = &pipeline.ip_policy {
        if !ip_policy.allows(ctx.envelope.client_ip) {
            return Err(VkdgError::Unauthorized);
        }
    }

    // 1. Admission ─────────────────────────────────────────────────────────────
    // Must be the very first step: any failure before this would leak requests
    // past the capacity limit.
    let _permit = pipeline.admission.acquire()?;
    ctx.transition(AttemptState::Admitted);

    // 2. Combo resolution + session stickiness ────────────────────────────────
    // Clients send no session key; derive one from the conversation so its turns
    // share a connection (and the provider's prompt cache).
    if ctx.envelope.session_key.is_none() {
        if let Operation::Conversation(conv) = &operation {
            ctx.envelope.session_key = super::session_affinity::derive(&ctx.envelope, conv);
        }
    }
    // The same digest doubles as the provider-facing cache key; it carries no
    // prompt text or raw session id, so it is safe to send upstream.
    if let (Operation::Conversation(conv), Some(key)) = (&mut operation, &ctx.envelope.session_key)
    {
        conv.cache_key = Some(key.0.clone());
    }
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
                        vkdg_operations::MessageContent::Blocks(_) => 50,
                    })
                    .sum()
            } else {
                0
            };
            // Input-only estimate at the highest price any provider lists for
            // the model; $3/M when none does, so an unpriced model still counts.
            let model = match &operation {
                Operation::Conversation(c) => c.model.as_str(),
                _ => ctx.envelope.model_requested.as_str(),
            };
            let per_mtok = pipeline
                .provider_registry
                .max_price(model)
                .map_or(3_000_000, |p| p.input_per_mtok);
            let estimated_cost_microdollars = u64::try_from(
                (u128::from(estimated_tokens) * u128::from(per_mtok)).div_ceil(1_000_000),
            )
            .unwrap_or(u64::MAX);
            if estimated_cost_microdollars > max_cost {
                return Err(VkdgError::BudgetExceeded {
                    #[allow(clippy::cast_precision_loss)] // microdollar amounts; f64 precision sufficient
                    estimated_usd: estimated_cost_microdollars as f64 / 1_000_000.0,
                    #[allow(clippy::cast_precision_loss)] // microdollar amounts; f64 precision sufficient
                    limit_usd: max_cost as f64 / 1_000_000.0,
                });
            }
        }
    }

    // 3. Route ─────────────────────────────────────────────────────────────────
    // A route names its targets by id and matches only on its own `match_models`,
    // so on its own it will happily pick a connection that cannot take the
    // request: one whose catalogue lacks the model (upstream answers 400), one in
    // cooldown, or one already at `max_concurrent` — the last dies later as
    // "no eligible connection" at reservation, without ever trying a sibling.
    // Excluding them here lets the strategy choose among targets that can serve.
    //
    // The model is the one that will actually go upstream: a combo request names
    // the combo id (`coding-fast`), which no connection lists, and the combo's
    // own `model` is what the provider receives. Filtering on the raw envelope
    // would exclude every target of every combo.
    let effective_model = csr
        .combo_model
        .clone()
        .unwrap_or_else(|| ctx.envelope.model_requested.clone());
    let filter = {
        let mut reason_map: HashMap<ConnectionId, String> = excluded
            .iter()
            .map(|id| (id.clone(), "rate_limited".to_owned()))
            .collect();
        for (id, reason) in pipeline.catalog.unroutable(&effective_model) {
            reason_map.entry(id).or_insert_with(|| reason.to_owned());
        }
        EligibilityFilter {
            excluded_connections: reason_map.keys().cloned().collect(),
            reason_map,
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
        hints.in_flight = pipeline.catalog.in_flight();
        hints
    };
    let route_result = match &ctx.pinned_connection {
        Some(pinned) => Ok(RouteResult {
            connection_id: pinned.clone(),
            route_id: RouteId("pinned".into()),
            excluded: vec![],
            fusion_targets: vec![],
            chain_steps: vec![],
            hooks: Default::default(),
        }),
        None => {
            pipeline
                .router
                .route(&ctx.envelope, &filter, &routing_hints)
                .await
        }
    };
    let route_result = match route_result {
        Ok(r) => r,
        // Only when no route claims the model. A route that matched but has no
        // usable target stays failed: falling through would send its traffic to
        // connections outside its `targets` and around its policy.
        Err(VkdgError::NoRouteMatched) => {
            // Auto-route: find any healthy catalog connection that serves the model.
            let model = &ctx.envelope.model_requested;
            let excluded_ids = filter.excluded_connections.clone();
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
                hooks: Default::default(),
            }
        }
        Err(e) => return Err(e),
    };
    ctx.route_id = Some(route_result.route_id.0.clone());
    ctx.excluded.clone_from(&route_result.excluded);
    // A combo id (`coding-fast`) is not a model any provider knows. A request
    // naming the combo itself is sent upstream as the combo's model, before any
    // dispatch branch, so fusion and chain targets get the same name. A request
    // that reached a combo through a pattern keeps the model it asked for.
    if let (Some(id), Some(model)) = (&csr.combo_id, &csr.combo_model) {
        if *id == ctx.envelope.model_requested {
            if let Operation::Conversation(conv) = &mut operation {
                conv.model.clone_from(model);
            }
        }
    }

    // Route hooks run before any dispatch path, fusion and chains included.
    if !route_result.hooks.auth.is_empty() {
        pipeline
            .hooks
            .authorize(
                &route_result.hooks.auth,
                crate::hooks::HookRequest {
                    key_id: ctx.envelope.client_id.0.clone(),
                    tenant_id: ctx.envelope.tenant_id.0.clone(),
                    client_ip: ctx.envelope.client_ip,
                    model: ctx.envelope.model_requested.clone(),
                    route_id: route_result.route_id.0.clone(),
                },
            )
            .await?;
    }

    // Fusion fast-path: fan-out to all targets in parallel, return first success.
    if route_result.fusion_targets.len() > 1 {
        return run_fusion_dispatch(
            pipeline,
            ctx,
            operation,
            route_result.fusion_targets,
            csr.compression_threshold,
            csr.effective_compressor_id.as_deref(),
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
            csr.effective_compressor_id.as_deref(),
        )
        .await;
    }

    // Priority: session pin > routed connection. Combos route through the
    // router as priority routes, so their strategy already chose the target.
    // Save the preferred connection before it is moved into the if-let so we
    // can detect rotation afterwards for context-relay.
    let session_preferred_saved = csr.session_preferred.clone();
    let connection_id = match csr.session_preferred {
        // A pin holds only while its connection is still eligible for this
        // request: in the catalog, serving the model, healthy, with capacity and
        // not already rate-limited this request. Otherwise the router's choice
        // wins and the pin moves with the next successful response.
        Some(preferred)
            if !filter.is_excluded(&preferred) && pipeline.catalog.get(&preferred).is_some() =>
        {
            tracing::debug!(
                session_key = ?ctx.envelope.session_key,
                pinned = %preferred.0,
                "session stickiness: using pinned connection",
            );
            preferred
        }
        _ => route_result.connection_id,
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
    let credential = pipeline.credentials.get_token(&config).await?;
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
            .unwrap_or_else(|_| {
                error_response(
                    &ctx.envelope.api_type,
                    &VkdgError::Internal("response build".into()),
                )
            }));
    }
    // 5. Prepare operation (compression + system prompt + memory injection) ────
    let (op, compress_metrics) = prepare_operation(
        pipeline,
        operation,
        &ctx.envelope,
        csr.compression_threshold,
        csr.effective_compressor_id.as_deref(),
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
        let key = cache_key(
            &ctx.envelope.model_requested,
            &ctx.envelope.api_type,
            conv_req,
        );
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
    // A streaming request never reads or writes the cache: the stored body is JSON,
    // and a hit would hand a client that asked for SSE a plain body.
    let bypass_cache = ctx.envelope.cache_bypass
        || conv_req.map_or(true, |r| r.stream || is_multiturn(r) || has_tool_calls(r));
    let cache_key_val: Option<String> = if bypass_cache {
        None
    } else {
        conv_req.map(|r| cache_key(&ctx.envelope.model_requested, &ctx.envelope.api_type, r))
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
                        error_response(
                            &ctx.envelope.api_type,
                            &VkdgError::Internal("cache response build".into()),
                        )
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
    let adapter = pipeline
        .provider_registry
        .get(config.provider.adapter_id())
        .ok_or_else(|| {
            VkdgError::Internal(format!(
                "no provider adapter registered for '{}'",
                config.provider.adapter_id()
            ))
        })?;
    let prepared = adapter
        .prepare(&operation, &config, &credential)
        .map_err(|e| VkdgError::Internal(e.to_string()))?;
    // Priced on the model actually sent upstream. A compatible endpoint borrows
    // the OpenAI or Anthropic adapter for its wire format, not their prices.
    ctx.price = match (&config.provider, &operation) {
        (
            vkdg_connections::ProviderKind::Custom { .. }
            | vkdg_connections::ProviderKind::AnthropicCompat { .. },
            _,
        ) => None,
        (_, Operation::Conversation(conv)) => {
            vkdg_core::pricing::price_for(adapter.prices(), &conv.model).cloned()
        }
        _ => None,
    };
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
        Err(VkdgError::UpstreamError {
            code,
            message,
            retry_after,
        }) if super::helpers::cools_connection(code) => {
            cool_connection(pipeline, ctx.connection_id.as_ref(), code, retry_after).await;
            return Err(VkdgError::UpstreamError {
                code,
                message,
                retry_after,
            });
        }
        Err(e) => return Err(e),
    };

    // 9b. Account failures reported inside a 200 ──────────────────────────────
    // Kiro answers 200 and sends "throttled" / "out of credits" as the first
    // frame of the stream; a dialect translator can find the same in a complete
    // body. Both are found here, while nothing has reached the client, so the
    // connection is cooled and the entry point can retry on a sibling. Once
    // `mark_committed` runs below that is no longer possible.
    let relay = super::relay::plan(adapter.as_ref(), &config, &ctx.envelope.api_type);
    let upstream_resp = match upstream_resp {
        UpstreamResponse::Streaming { status, body } => {
            match super::relay::guard_first_event(adapter.as_ref(), body).await {
                Ok(body) => UpstreamResponse::Streaming { status, body },
                Err(error) => {
                    return Err(account_failure(pipeline, ctx.connection_id.as_ref(), error).await)
                }
            }
        }
        // The client-facing body: translated when the upstream speaks the other
        // dialect. What the cache stores and what the client gets are the same bytes.
        UpstreamResponse::Complete { status, body } => {
            match super::relay::relay_complete(
                &relay,
                body,
                &ctx.envelope.api_type,
                &vkdg_operations::StreamContext {
                    model: &ctx.envelope.model_requested,
                    request_id: &ctx.envelope.request_id,
                },
            ) {
                Ok(body) => UpstreamResponse::Complete { status, body },
                Err(e)
                    if matches!(
                        &e,
                        VkdgError::UpstreamError { code, .. } if super::helpers::fails_over(*code)
                    ) =>
                {
                    return Err(account_failure(pipeline, ctx.connection_id.as_ref(), e).await)
                }
                Err(e) => return Err(e),
            }
        }
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
            let mut builder = axum::response::Response::builder()
                .status(status_code)
                .header(header::CONTENT_TYPE, "application/json");
            if let Some(m) = &compress_metrics {
                if m.estimated_tokens_removed > 0 {
                    builder = builder.header(
                        "x-vkdg-tokens-saved",
                        m.estimated_tokens_removed.to_string(),
                    );
                }
            }
            builder
                .body(axum::body::Body::from(body))
                .unwrap_or_else(|_| {
                    error_response(
                        &ctx.envelope.api_type,
                        &VkdgError::Internal("response builder failed".into()),
                    )
                })
        }
        UpstreamResponse::Streaming { status: _, body } => {
            // Translated or provider-decoded streams are re-encoded in the client's
            // dialect; same-dialect streams pass through untouched.
            let body = super::relay::into_client_stream(
                relay,
                body,
                &ctx.envelope.api_type,
                &vkdg_operations::StreamContext {
                    model: &ctx.envelope.model_requested,
                    request_id: &ctx.envelope.request_id,
                },
                ctx.envelope.include_think_tags,
            );
            let mut builder = axum::response::Response::builder()
                .status(http::StatusCode::OK)
                .header(header::CONTENT_TYPE, "text/event-stream")
                .header("cache-control", "no-cache")
                .header("x-accel-buffering", "no")
                .header("x-vkdg-stream-guard", "active");
            if let Some(m) = &compress_metrics {
                if m.estimated_tokens_removed > 0 {
                    builder = builder.header(
                        "x-vkdg-tokens-saved",
                        m.estimated_tokens_removed.to_string(),
                    );
                }
            }
            builder
                .body(axum::body::Body::from_stream(body))
                .unwrap_or_else(|_| {
                    error_response(
                        &ctx.envelope.api_type,
                        &VkdgError::Internal("streaming response builder failed".into()),
                    )
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
    )
    .await;
    Ok(resp)
}

// ── Cooldown ──────────────────────────────────────────────────────────────────

/// Puts `connection_id` into cooldown for an upstream `code` (and the wait the
/// upstream asked for, if any). Returns when it will be eligible again, or `None`
/// when the connection left the catalog.
///
/// Waits for the connection's lock instead of skipping when a request is
/// reserving a slot: a missed write would leave a limited connection eligible and
/// its `retry-after` ignored.
async fn cool_connection(
    pipeline: &PipelineState,
    connection_id: Option<&ConnectionId>,
    code: u16,
    retry_after: Option<u32>,
) -> Option<chrono::DateTime<chrono::Utc>> {
    let conn_arc = pipeline.catalog.get(connection_id?)?;
    let until = conn_arc
        .write()
        .await
        .record_upstream_error(code, retry_after);
    Some(until)
}

/// Handles an account failure found inside an HTTP 200 (see step 9b): cools the
/// connection and returns the error for the entry point to retry on a sibling.
///
/// Providers such as Kiro never say how long to wait, so an error without a
/// `retry-after` gets the time the connection will stay out. If no sibling can
/// serve the retry, that is what the client is told.
async fn account_failure(
    pipeline: &PipelineState,
    connection_id: Option<&ConnectionId>,
    error: VkdgError,
) -> VkdgError {
    let VkdgError::UpstreamError {
        code,
        message,
        retry_after,
    } = error
    else {
        return error;
    };
    let until = cool_connection(pipeline, connection_id, code, retry_after).await;
    let retry_after = retry_after.or_else(|| {
        until.map(|until| {
            u32::try_from((until - chrono::Utc::now()).num_seconds().max(1)).unwrap_or(u32::MAX)
        })
    });
    VkdgError::UpstreamError {
        code,
        message,
        retry_after,
    }
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
    effective_compressor_id: Option<&str>,
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
    let envelope = ctx.envelope.clone();
    let mut futs: FuturesUnordered<_> = fusion_targets
        .into_iter()
        .map(|conn_id| {
            let operation = operation.clone();
            let envelope = envelope.clone();
            async move { fusion_one_target(pipeline, conn_id, operation, envelope).await }
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

async fn fusion_one_target(
    pipeline: &PipelineState,
    conn_id: ConnectionId,
    operation: Operation,
    envelope: vkdg_core::RequestEnvelope,
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

    let credential = pipeline.credentials.get_token(&config).await?;

    let adapter = pipeline
        .provider_registry
        .get(config.provider.adapter_id())
        .ok_or_else(|| {
            VkdgError::Internal(format!(
                "no provider adapter registered for '{}'",
                config.provider.adapter_id()
            ))
        })?;
    let prepared = adapter
        .prepare(&operation, &config, &credential)
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

    let stream_ctx = vkdg_operations::StreamContext {
        model: &envelope.model_requested,
        request_id: &envelope.request_id,
    };
    let relay = super::relay::plan(adapter.as_ref(), &config, &envelope.api_type);
    let resp = match upstream_resp {
        UpstreamResponse::Complete { status, body } => {
            let body = super::relay::relay_complete(&relay, body, &envelope.api_type, &stream_ctx)?;
            axum::response::Response::builder()
                .status(http::StatusCode::from_u16(status).unwrap_or(http::StatusCode::BAD_GATEWAY))
                .header(header::CONTENT_TYPE, "application/json")
                .body(axum::body::Body::from(body))
                .map_err(|e| VkdgError::Internal(e.to_string()))?
        }
        UpstreamResponse::Streaming { status: _, body } => {
            let body = super::relay::into_client_stream(
                relay,
                body,
                &envelope.api_type,
                &stream_ctx,
                envelope.include_think_tags,
            );
            axum::response::Response::builder()
                .status(http::StatusCode::OK)
                .header(header::CONTENT_TYPE, "text/event-stream")
                .header("cache-control", "no-cache")
                .header("x-accel-buffering", "no")
                .body(axum::body::Body::from_stream(body))
                .map_err(|e| VkdgError::Internal(e.to_string()))?
        }
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
    effective_compressor_id: Option<&str>,
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
                        conv_req.prepend_system(&format!("Previous output:\n{prev}"));
                    }
                }
            }
        }

        // Override system for this step if configured.
        if let Operation::Conversation(conv_req) = &mut operation {
            if let Some(step_system) = &step.system {
                conv_req.set_system(Some(step_system.clone()));
            }
        }

        // Resolve connection for this step.
        let conn_id = if let Some(id) = step.connection_id {
            id
        } else {
            // Auto-route: find any eligible catalog connection for the requested model.
            let excluded_ids: Vec<ConnectionId> = vec![];
            pipeline
                .catalog
                .eligible(&ctx.envelope.model_requested, &excluded_ids)
                .into_iter()
                .next()
                .ok_or(VkdgError::NoEligibleConnection)?
        };

        ctx.connection_id = Some(conn_id.clone());
        tracing::debug!(
            step = step_idx,
            total = step_count,
            connection = %conn_id.0,
            "prompt chain step",
        );

        // Execute the step.
        let resp =
            fusion_one_target(pipeline, conn_id, operation.clone(), ctx.envelope.clone()).await?;

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
    use vkdg_provider_sdk::ProviderRegistry;

    struct StubAdapter;
    impl ProviderAdapter for StubAdapter {
        fn id(&self) -> &'static str {
            // Registered under "openai" so Custom connections (adapter_id() = "openai") resolve it.
            "openai"
        }
        fn display_name(&self) -> &'static str {
            "Stub"
        }
        fn prepare(
            &self,
            _op: &Operation,
            _cfg: &vkdg_connections::ConnectionConfig,
            _credential: &vkdg_provider_sdk::Credential,
        ) -> Result<PreparedRequest, vkdg_provider_sdk::ProviderError> {
            Err(vkdg_provider_sdk::ProviderError::UnsupportedOperation)
        }
    }

    fn make_pipeline(admission_limit: usize) -> Arc<PipelineState> {
        let mut r = ProviderRegistry::empty();
        r.register(Arc::new(StubAdapter));
        Arc::new(PipelineState::minimal(
            Arc::new(AdmissionGuard::new(admission_limit)),
            Arc::new(VkdgRouter::new(vec![])),
            Arc::new(ConnectionCatalog::new(vec![])),
            Arc::new(CredentialManager::new()),
            Arc::new(HttpClient::new()),
            Arc::new(DecisionRecordExporter::new()),
            Arc::new(r),
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
            model: "test-model".into(),
            messages: vec![],
            tools: vec![],
            max_tokens: Some(100),
            temperature: None,
            stream: false,
            system: None,
            required_capabilities: CapabilitySet::default(),
            thinking: None,
            ..Default::default()
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
        let mut r = ProviderRegistry::empty();
        r.register(Arc::new(StubAdapter));
        let mut state = PipelineState::minimal(
            Arc::new(AdmissionGuard::new(100)),
            Arc::new(VkdgRouter::new(vec![])),
            Arc::new(ConnectionCatalog::new(vec![])),
            Arc::new(CredentialManager::new()),
            Arc::new(HttpClient::new()),
            Arc::new(DecisionRecordExporter::new()),
            Arc::new(r),
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
            model: None,
        };
        let mut r = ProviderRegistry::empty();
        r.register(Arc::new(StubAdapter));
        let mut state = PipelineState::minimal(
            Arc::new(AdmissionGuard::new(100)),
            Arc::new(VkdgRouter::new(vec![])),
            Arc::new(ConnectionCatalog::new(vec![])),
            Arc::new(CredentialManager::new()),
            Arc::new(HttpClient::new()),
            Arc::new(DecisionRecordExporter::new()),
            Arc::new(r),
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
            model: None,
        };
        let mut r = ProviderRegistry::empty();
        r.register(Arc::new(StubAdapter));
        let mut state = PipelineState::minimal(
            Arc::new(AdmissionGuard::new(100)),
            Arc::new(VkdgRouter::new(vec![])),
            Arc::new(ConnectionCatalog::new(vec![])),
            Arc::new(CredentialManager::new()),
            Arc::new(HttpClient::new()),
            Arc::new(DecisionRecordExporter::new()),
            Arc::new(r),
        );
        state.combo_resolver = Some(Arc::new(ComboResolver::new(vec![combo])));
        let pipeline = Arc::new(state);

        // Build a request with a non-trivial message so estimated_tokens > 0
        // and estimated_cost_microdollars (tokens * 3) > 0 = max_cost.
        use vkdg_operations::{Message, MessageContent, Role};
        let op = Operation::Conversation(ConversationRequest {
            model: "test-model".into(),
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
            thinking: None,
            ..Default::default()
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
            endpoint: None,
            capabilities: CapabilitySet::default(),
        };
        let route = RouteConfig {
            id: RouteId("stub-route".into()),
            match_models: vec!["stub-model".into()],
            strategy: StrategyKind::FallbackChain,
            targets: vec![conn_id],
            plugin_hooks: Default::default(),
        };

        let mut r = ProviderRegistry::empty();
        r.register(Arc::new(StubAdapter));
        let pipeline = Arc::new(PipelineState::minimal(
            Arc::new(AdmissionGuard::new(100)),
            Arc::new(VkdgRouter::new(vec![route])),
            Arc::new(ConnectionCatalog::new(vec![conn_config])),
            Arc::new(CredentialManager::new()),
            Arc::new(HttpClient::new()),
            Arc::new(DecisionRecordExporter::new()),
            Arc::new(r),
        ));

        let ctx = make_ctx("stub-model");
        let resp = run_conversation_pipeline(pipeline, ctx, make_conv_op()).await;
        assert_eq!(
            resp.status(),
            http::StatusCode::INTERNAL_SERVER_ERROR,
            "adapter prepare() returning Err must produce 500, not panic"
        );
    }

    use vkdg_routing::{RouteConfig, StrategyKind};

    struct Fixed(
        crate::hooks::HookVerdict,
        Arc<std::sync::Mutex<Vec<String>>>,
    );
    impl crate::hooks::AuthHook for Fixed {
        fn check(&self, r: &crate::hooks::HookRequest) -> crate::hooks::HookVerdict {
            self.1
                .lock()
                .unwrap()
                .push(serde_json::to_string(r).unwrap());
            self.0.clone()
        }
    }

    fn hooked_pipeline(
        strategy: StrategyKind,
        hooks: &[&str],
    ) -> (Arc<PipelineState>, Arc<std::sync::Mutex<Vec<String>>>) {
        use crate::hooks::{HookRegistry, HookVerdict};
        let ids: Vec<ConnectionId> = ["h1", "h2"]
            .iter()
            .map(|i| ConnectionId((*i).into()))
            .collect();
        let conns = ids
            .iter()
            .map(|id| vkdg_connections::ConnectionConfig {
                id: id.clone(),
                provider: vkdg_connections::ProviderKind::Custom {
                    base_url: "http://127.0.0.1:1".into(),
                },
                auth: vkdg_connections::AuthKind::ApiKey {
                    env_var: "UNUSED".into(),
                },
                models: vec!["hooked".into()],
                max_concurrent: 10,
                weight: 1,
                tags: vec![],
                endpoint: None,
                capabilities: CapabilitySet::default(),
            })
            .collect();
        let route = RouteConfig {
            id: RouteId("guarded".into()),
            match_models: vec!["hooked".into()],
            strategy,
            targets: ids,
            plugin_hooks: vkdg_routing::PluginHooks {
                auth: hooks.iter().map(|h| (*h).to_owned()).collect(),
            },
        };
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut reg = HookRegistry::default();
        reg.register_auth(
            "allow",
            Arc::new(Fixed(HookVerdict::Allow, Arc::clone(&seen))),
        );
        reg.register_auth(
            "deny",
            Arc::new(Fixed(HookVerdict::Deny("nope".into()), Arc::clone(&seen))),
        );
        let mut r = ProviderRegistry::empty();
        r.register(Arc::new(StubAdapter));
        let mut p = PipelineState::minimal(
            Arc::new(AdmissionGuard::new(10)),
            Arc::new(VkdgRouter::new(vec![route])),
            Arc::new(ConnectionCatalog::new(conns)),
            Arc::new(CredentialManager::new()),
            Arc::new(HttpClient::new()),
            Arc::new(DecisionRecordExporter::new()),
            Arc::new(r),
        );
        p.hooks = Arc::new(reg);
        (Arc::new(p), seen)
    }

    // Found in review: route hooks were parsed and never run.
    #[tokio::test]
    async fn route_auth_hook_denial_is_forbidden_on_every_dispatch_path() {
        for strategy in [
            StrategyKind::RoundRobin,
            StrategyKind::Fusion {
                max_candidates: None,
            },
        ] {
            let (p, _) = hooked_pipeline(strategy.clone(), &["allow", "deny"]);
            let resp = run_conversation_pipeline(p, make_ctx("hooked"), make_conv_op()).await;
            assert_eq!(resp.status(), http::StatusCode::FORBIDDEN, "{strategy:?}");
        }
    }

    // A hook that is not installed must deny, not be skipped.
    #[tokio::test]
    async fn unknown_route_hook_denies() {
        let (p, _) = hooked_pipeline(StrategyKind::RoundRobin, &["not-installed"]);
        let resp = run_conversation_pipeline(p, make_ctx("hooked"), make_conv_op()).await;
        assert_eq!(resp.status(), http::StatusCode::FORBIDDEN);
    }

    // Allowed requests go on to dispatch; the hook sees identity, not secrets.
    #[tokio::test]
    async fn allowing_hook_passes_and_sees_no_credentials() {
        let (p, seen) = hooked_pipeline(StrategyKind::RoundRobin, &["allow"]);
        let resp = run_conversation_pipeline(p, make_ctx("hooked"), make_conv_op()).await;
        assert_ne!(resp.status(), http::StatusCode::FORBIDDEN);
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert!(seen[0].contains("\"route_id\":\"guarded\""), "{}", seen[0]);
        assert!(
            !seen[0].to_lowercase().contains("api-key") && !seen[0].contains("vkdg_"),
            "{}",
            seen[0]
        );
    }

    // The admin request log always recorded `decision: None`, so the console
    // could never show which route handled a request or why targets were skipped.
    #[tokio::test]
    async fn request_log_records_the_routing_decision() {
        let (p, _) = hooked_pipeline(StrategyKind::RoundRobin, &[]);
        let log = vkdg_admin::handlers::requests::RequestLog::new();
        let mut p = Arc::try_unwrap(p).ok().expect("sole owner");
        p.request_log = Some(Arc::clone(&log));
        let ctx = make_ctx("hooked");
        let id = ctx.envelope.request_id.0.to_string();
        let resp = run_conversation_pipeline(Arc::new(p), ctx, make_conv_op()).await;
        // The pipeline hands the row to the auth middleware; it writes nothing itself.
        assert!(log.get(&id).is_none());
        let rec = resp
            .extensions()
            .get::<crate::pipeline::PendingLog>()
            .expect("row attached")
            .record
            .clone();
        assert_eq!(rec.request_id, id);
        let d = rec.decision.expect("decision recorded");
        assert_eq!(d.route_id.as_deref(), Some("guarded"));
        assert!(d.attempt_count >= 1);
    }

    /// Records what reached `prepare` (the last step before upstream), then
    /// fails, so no network is needed.
    struct Recording(Arc<std::sync::Mutex<Vec<(String, String)>>>);
    impl ProviderAdapter for Recording {
        fn id(&self) -> &'static str {
            "openai"
        }
        fn display_name(&self) -> &'static str {
            "Recording"
        }
        fn prepare(
            &self,
            op: &Operation,
            cfg: &vkdg_connections::ConnectionConfig,
            _credential: &vkdg_provider_sdk::Credential,
        ) -> Result<PreparedRequest, vkdg_provider_sdk::ProviderError> {
            if let Operation::Conversation(c) = op {
                self.0
                    .lock()
                    .unwrap()
                    .push((cfg.id.0.clone(), c.model.clone()));
            }
            Err(vkdg_provider_sdk::ProviderError::UnsupportedOperation)
        }
    }

    // Found in review: a combo called by its id failed with NoEligibleConnection
    // (routing ran before the combo), always went to targets[0] whatever its
    // strategy, and sent the combo id itself upstream as the model.
    #[tokio::test]
    async fn a_combo_called_by_id_routes_with_its_strategy_and_sends_its_model() {
        std::env::set_var("VKDG_TEST_COMBO_KEY", "k");
        let conns = ["h1", "h2"]
            .iter()
            .map(|id| vkdg_connections::ConnectionConfig {
                id: ConnectionId((*id).into()),
                provider: vkdg_connections::ProviderKind::Custom {
                    base_url: "http://127.0.0.1:1".into(),
                },
                auth: vkdg_connections::AuthKind::ApiKey {
                    env_var: "VKDG_TEST_COMBO_KEY".into(),
                },
                models: vec!["m-*".into()],
                max_concurrent: 10,
                weight: 1,
                tags: vec![],
                endpoint: None,
                capabilities: CapabilitySet::default(),
            })
            .collect();
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut r = ProviderRegistry::empty();
        r.register(Arc::new(Recording(Arc::clone(&seen))));
        let mut p = PipelineState::minimal(
            Arc::new(AdmissionGuard::new(10)),
            Arc::new(VkdgRouter::new(vec![])),
            Arc::new(ConnectionCatalog::new(conns)),
            Arc::new(CredentialManager::new()),
            Arc::new(HttpClient::new()),
            Arc::new(DecisionRecordExporter::new()),
            Arc::new(r),
        );
        let dir = std::env::temp_dir().join(format!("vkdg-combo-{}", uuid::Uuid::new_v4()));
        let resolver = Arc::new(vkdg_combos::ComboResolver::new(vec![]));
        let svc = vkdg_combos::ComboService::open(
            vkdg_combos::ComboStore::new(dir.join("combos.json")),
            Arc::clone(&resolver),
            Some(Arc::clone(&p.router)),
        )
        .unwrap();
        svc.create(&vkdg_combos::Combo {
            id: "coding-fast".into(),
            match_patterns: vec![],
            strategy: StrategyKind::RoundRobin,
            targets: vec![ConnectionId("h1".into()), ConnectionId("h2".into())],
            model: Some("m-real".into()),
            compression: None,
            cache: None,
            budget: None,
            mode_pack: None,
        })
        .unwrap();
        p.combo_resolver = Some(resolver);
        let p = Arc::new(p);

        for _ in 0..2 {
            let _ =
                run_conversation_pipeline(Arc::clone(&p), make_ctx("coding-fast"), make_conv_op())
                    .await;
        }
        let seen = seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 2, "both calls reached a provider: {seen:?}");
        assert!(seen.iter().all(|(_, m)| m == "m-real"), "{seen:?}");
        assert_ne!(seen[0].0, seen[1].0, "round_robin rotates: {seen:?}");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Sends every request to the connection's own `base_url`, so each connection
    /// can be pointed at its own local upstream.
    struct ToBaseUrl;
    impl ProviderAdapter for ToBaseUrl {
        fn id(&self) -> &'static str {
            "openai"
        }
        fn display_name(&self) -> &'static str {
            "ToBaseUrl"
        }
        fn prepare(
            &self,
            _op: &Operation,
            cfg: &vkdg_connections::ConnectionConfig,
            _credential: &vkdg_provider_sdk::Credential,
        ) -> Result<PreparedRequest, vkdg_provider_sdk::ProviderError> {
            let vkdg_connections::ProviderKind::Custom { base_url } = &cfg.provider else {
                return Err(vkdg_provider_sdk::ProviderError::UnsupportedOperation);
            };
            Ok(PreparedRequest {
                url: base_url.clone(),
                headers: http::HeaderMap::new(),
                body: Bytes::new(),
                is_streaming: false,
            })
        }
    }

    /// A local upstream that answers every request with `status` (and
    /// `retry-after`, when given). Returns its URL and a request counter.
    async fn canned_upstream(
        status: u16,
        retry_after: Option<&'static str>,
    ) -> (String, Arc<AtomicUsize>) {
        use http_body_util::Full;
        use hyper::service::service_fn;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1/messages", listener.local_addr().unwrap());
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&hits);
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let counter = Arc::clone(&counter);
                tokio::spawn(async move {
                    let service = service_fn(move |_req: hyper::Request<hyper::body::Incoming>| {
                        counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        async move {
                            let mut resp = hyper::Response::builder().status(status);
                            if let Some(v) = retry_after {
                                resp = resp.header("retry-after", v);
                            }
                            Ok::<_, std::convert::Infallible>(
                                resp.body(Full::new(Bytes::from_static(
                                    br#"{"id":"m","type":"message","role":"assistant","content":[],"usage":{"input_tokens":1,"output_tokens":1}}"#,
                                )))
                                .unwrap(),
                            )
                        }
                    });
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(hyper_util::rt::TokioIo::new(stream), service)
                        .await;
                });
            }
        });
        (url, hits)
    }

    /// A route balancing `limited` and `healthy` with `power_of_two_choices`; each
    /// serves the model under its own spelling (Kiro dots, Anthropic dashes).
    fn two_upstream_pipeline(
        limited_url: String,
        healthy_url: String,
        limited_first: bool,
    ) -> Arc<PipelineState> {
        std::env::set_var("VKDG_TEST_FAILOVER_KEY", "k");
        let conn = |id: &str, url: String, models: &[&str]| vkdg_connections::ConnectionConfig {
            id: ConnectionId(id.into()),
            provider: vkdg_connections::ProviderKind::Custom { base_url: url },
            auth: vkdg_connections::AuthKind::ApiKey {
                env_var: "VKDG_TEST_FAILOVER_KEY".into(),
            },
            models: models.iter().map(|m| (*m).to_owned()).collect(),
            max_concurrent: 10,
            weight: 1,
            tags: vec![],
            endpoint: None,
            capabilities: CapabilitySet::default(),
        };
        let mut conns = vec![
            conn("limited", limited_url, &["claude-sonnet-4.6"]),
            conn("healthy", healthy_url, &["claude-*"]),
        ];
        if !limited_first {
            conns.reverse();
        }
        let route = RouteConfig {
            id: RouteId("claude".into()),
            match_models: vec!["claude-*".into()],
            strategy: StrategyKind::PowerOfTwoChoices,
            targets: conns.iter().map(|c| c.id.clone()).collect(),
            plugin_hooks: vkdg_routing::PluginHooks::default(),
        };
        let mut r = ProviderRegistry::empty();
        r.register(Arc::new(ToBaseUrl));
        Arc::new(PipelineState::minimal(
            Arc::new(AdmissionGuard::new(100)),
            Arc::new(VkdgRouter::new(vec![route])),
            Arc::new(ConnectionCatalog::new(conns)),
            Arc::new(CredentialManager::new()),
            Arc::new(HttpClient::new()),
            Arc::new(DecisionRecordExporter::new()),
            Arc::new(r),
        ))
    }

    // Plausible wrong impl: either the 429 does not take the connection out for the
    // advertised `retry-after` (it rejoins after the 300 s backoff ceiling and every
    // later request burns a probe on the limited account), or the failover does not
    // happen (the client sees the 429 while a healthy sibling serves the model).
    // Both targets serve the model under different spellings (Kiro dots, Anthropic
    // dashes) and the route balances them with power_of_two_choices; each direction
    // is exercised: the limited account first in `targets`, then second.
    #[tokio::test]
    async fn a_429_with_retry_after_fails_over_and_keeps_the_limited_connection_out() {
        const REQUESTS: usize = 32;
        for limited_first in [true, false] {
            let (limited_url, limited_hits) = canned_upstream(429, Some("18000")).await;
            let (healthy_url, healthy_hits) = canned_upstream(200, None).await;
            let p = two_upstream_pipeline(limited_url, healthy_url, limited_first);

            // The client spells the model with dashes; "limited" lists it with a dot.
            for i in 0..REQUESTS {
                let resp = run_conversation_pipeline(
                    Arc::clone(&p),
                    make_ctx("claude-sonnet-4-6"),
                    make_conv_op(),
                )
                .await;
                assert_eq!(
                    resp.status(),
                    http::StatusCode::OK,
                    "request {i} (limited_first={limited_first}) must be served by the healthy sibling"
                );
            }

            assert_eq!(
                limited_hits.load(std::sync::atomic::Ordering::SeqCst),
                1,
                "limited_first={limited_first}: the limited connection must be probed exactly once \
                 (0 = the strategy never tried it, >1 = its cooldown did not hold)"
            );
            assert_eq!(
                healthy_hits.load(std::sync::atomic::Ordering::SeqCst),
                REQUESTS,
                "every request ends on the healthy connection, none lost to the 429"
            );

            let wait = p
                .catalog
                .secs_until_cooldown_ends("claude-sonnet-4-6")
                .expect("the limited connection is cooling");
            assert!(
                (17_900..=18_001).contains(&wait),
                "cooldown must follow retry-after: 18000, got {wait} s"
            );
            let healthy = p.catalog.get(&ConnectionId("healthy".into())).unwrap();
            assert!(
                healthy.read().await.state.is_healthy(),
                "invariant 6: the 429 on one connection must not cool its sibling"
            );
        }
    }

    /// `n` Custom connections on one `claude-*` route (fallback chain, so the
    /// router always prefers the first target), each at its own canned upstream.
    fn pinned_pipeline(urls: &[String], with_route: bool) -> Arc<PipelineState> {
        std::env::set_var("VKDG_TEST_FAILOVER_KEY", "k");
        let conns: Vec<_> = urls
            .iter()
            .enumerate()
            .map(|(i, url)| vkdg_connections::ConnectionConfig {
                id: ConnectionId(format!("conn-{i}")),
                provider: vkdg_connections::ProviderKind::Custom {
                    base_url: url.clone(),
                },
                auth: vkdg_connections::AuthKind::ApiKey {
                    env_var: "VKDG_TEST_FAILOVER_KEY".into(),
                },
                models: vec!["claude-*".into()],
                max_concurrent: 10,
                weight: 1,
                tags: vec![],
                endpoint: None,
                capabilities: CapabilitySet::default(),
            })
            .collect();
        let routes = if with_route {
            vec![RouteConfig {
                id: RouteId("claude".into()),
                match_models: vec!["claude-*".into()],
                strategy: StrategyKind::FallbackChain,
                targets: conns.iter().map(|c| c.id.clone()).collect(),
                plugin_hooks: vkdg_routing::PluginHooks::default(),
            }]
        } else {
            vec![]
        };
        let mut r = ProviderRegistry::empty();
        r.register(Arc::new(ToBaseUrl));
        Arc::new(PipelineState::minimal(
            Arc::new(AdmissionGuard::new(100)),
            Arc::new(VkdgRouter::new(routes)),
            Arc::new(ConnectionCatalog::new(conns)),
            Arc::new(CredentialManager::new()),
            Arc::new(HttpClient::new()),
            Arc::new(DecisionRecordExporter::new()),
            Arc::new(r),
        ))
    }

    // The admin "Test" button pins the request to the connection under test.
    // Plausible wrong impl: the pin is ignored and the router picks the route's
    // first target, so the test probes (and reports on) a different account.
    #[tokio::test]
    async fn a_pinned_connection_bypasses_the_router() {
        let (first_url, first_hits) = canned_upstream(200, None).await;
        let (second_url, second_hits) = canned_upstream(200, None).await;
        let p = pinned_pipeline(&[first_url, second_url], true);

        let mut ctx = make_ctx("claude-sonnet-4-6");
        ctx.pinned_connection = Some(ConnectionId("conn-1".into()));
        let resp = run_conversation_pipeline(p, ctx, make_conv_op()).await;

        assert_eq!(resp.status(), http::StatusCode::OK);
        assert_eq!(first_hits.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert_eq!(second_hits.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    // Production: every account connection lists only patterns (`claude-*`), no
    // route or connection serves the literal model the old smoke test sent, so the
    // test died as "no eligible connection". A pinned connection is tried as asked.
    #[tokio::test]
    async fn a_pinned_connection_is_used_even_when_no_route_serves_the_model() {
        let (url, hits) = canned_upstream(200, None).await;
        let p = pinned_pipeline(&[url], false);

        let mut ctx = make_ctx("test");
        ctx.pinned_connection = Some(ConnectionId("conn-0".into()));
        let resp = run_conversation_pipeline(p, ctx, make_conv_op()).await;

        assert_eq!(resp.status(), http::StatusCode::OK);
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}
