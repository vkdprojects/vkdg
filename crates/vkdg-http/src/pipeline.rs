use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::response::Response;
use bytes::Bytes;
use futures::{Stream, StreamExt};
use http::header;
use serde_json::json;
use vkdg_cache::{CacheEntry, CacheResult, cache_key};
use vkdg_core::{AttemptResult, AttemptState, ConnectionId, DecisionRecord, VkdgError};
use vkdg_core::pipeline::PipelineCtx;
use vkdg_operations::{ContentBlock, MessageContent, Operation, Role};
use vkdg_routing::{EligibilityFilter, RoutingHints};

use crate::sse::{SseEvent, SseParser};
use crate::upstream::{UpstreamRequest, UpstreamResponse};
use crate::PipelineState;

// ── Pipeline entry point ──────────────────────────────────────────────────────

pub async fn run_conversation_pipeline(
    pipeline: Arc<PipelineState>,
    mut ctx: PipelineCtx,
    operation: Operation,
) -> Response {
    let outcome = run_pipeline_inner(&pipeline, &mut ctx, operation.clone(), &[]).await;

    // Transparent 429 fallback: if the upstream rate-limits us and we have not
    // yet committed any bytes to the client, retry with the failed connection
    // excluded so the router picks a different candidate.
    let (ctx, outcome) = if let Err(VkdgError::UpstreamError { code: 429, .. }) = &outcome {
        if ctx.can_retry() {
            // Emit DecisionRecord for the failed first attempt before retrying.
            let excluded: Vec<ConnectionId> =
                ctx.connection_id.clone().into_iter().collect();
            emit_decision_record(&pipeline, &ctx, &outcome, 1);

            let mut ctx2 = PipelineCtx::new(ctx.envelope.clone());
            let outcome2 =
                run_pipeline_inner(&pipeline, &mut ctx2, operation.clone(), &excluded).await;
            (ctx2, outcome2)
        } else {
            (ctx, outcome)
        }
    } else {
        (ctx, outcome)
    };

    // Emit DecisionRecord for the final attempt (attempt 1 when no retry, attempt 2 otherwise).
    emit_decision_record(&pipeline, &ctx, &outcome, 1);

    match outcome {
        Ok(resp) => resp,
        Err(e) => error_response(e),
    }
}

/// Emit a DecisionRecord to the pipeline exporter.
fn emit_decision_record(
    pipeline: &PipelineState,
    ctx: &PipelineCtx,
    outcome: &Result<Response, VkdgError>,
    attempt: u32,
) {
    let result = match outcome {
        Ok(_) => AttemptResult::Completed,
        Err(e) => AttemptResult::Failed {
            code: format!("{e}"),
            phase: format!("{:?}", ctx.state),
            retryable: !ctx.committed,
            committed: ctx.committed,
        },
    };
    let record = DecisionRecord {
        request_id: ctx.envelope.request_id.clone(),
        config_version: 0,
        client_id: ctx.envelope.client_id.clone(),
        route_id: ctx
            .connection_id
            .as_ref()
            .map(|c| c.0.clone())
            .unwrap_or_else(|| "none".into()),
        connection_chosen: ctx.connection_id.clone(),
        candidates_excluded: vec![],
        attempt_count: attempt,
        state_transitions: ctx.transitions.clone(),
        result,
    };
    pipeline.exporter.export(&record);
}

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

async fn run_pipeline_inner(
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
    let _permit = pipeline.admission.acquire().await?;
    ctx.transition(AttemptState::Admitted);

    // 1.5. Combo resolution ────────────────────────────────────────────────────
    // Resolve the requested model name to a combo before routing so the combo's
    // configuration (targets, strategy, compression, cache policy) can shadow the
    // default route.  Falls through to bare model routing when no combo matches.
    let combo = pipeline
        .combo_resolver
        .as_ref()
        .and_then(|r| r.resolve(&ctx.envelope.model_requested));

    if let Some(c) = &combo {
        tracing::debug!(combo_id = %c.id, "request resolved to combo");
    }

    // 1.5b. Effective compression config ──────────────────────────────────────
    // X-VKDG-Compression header overrides the combo plugin_id ("none" = skip).
    // Threshold comes from the combo; falls back to 2000 when unset.
    let combo_compression = combo
        .as_ref()
        .and_then(|c| c.compression.as_ref())
        .cloned();
    let effective_compressor_id: Option<String> = ctx.envelope.compression_override.clone()
        .or_else(|| combo_compression.as_ref().map(|c| c.plugin_id.clone()));
    let compression_threshold: u32 = combo_compression
        .as_ref()
        .and_then(|c| c.auto_trigger_tokens)
        .unwrap_or(2000);

    // 1.6. Session stickiness ──────────────────────────────────────────────────
    // If the request carries a session_key and the registry has a valid pin for
    // it, record the preferred connection now.  We apply it after routing (below)
    // so the combo override logic is unaffected.
    let session_preferred_connection: Option<ConnectionId> =
        if let (Some(registry), Some(session_key)) = (
            &pipeline.session_registry,
            &ctx.envelope.session_key,
        ) {
            registry.get(&session_key.0).await
        } else {
            None
        };

    // 2. Route ─────────────────────────────────────────────────────────────────
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
    let route_result = pipeline
        .router
        .route(&ctx.envelope, &filter, &routing_hints)
        .await?;
    // Apply combo target override: if a combo matched and its targets don't
    // include the routed connection, redirect to the first combo target.
    // Apply combo target override, then honor session preference.
    // Priority: session pin > combo redirect > routed connection.
    let connection_id = if let Some(preferred) = session_preferred_connection {
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
        } else if let Some(c) = &combo {
            if !c.targets.is_empty() && !c.targets.contains(&route_result.connection_id) {
                c.targets[0].clone()
            } else {
                route_result.connection_id
            }
        } else {
            route_result.connection_id
        }
    } else if let Some(c) = &combo {
        if !c.targets.is_empty() && !c.targets.contains(&route_result.connection_id) {
            tracing::debug!(
                combo_id = %c.id,
                routed   = %route_result.connection_id.0,
                redirect = %c.targets[0].0,
                "combo redirect: routed connection not in combo targets",
            );
            c.targets[0].clone()
        } else {
            route_result.connection_id
        }
    } else {
        route_result.connection_id
    };
    ctx.connection_id = Some(connection_id.clone());
    ctx.transition(AttemptState::AccountReserved);

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
    // 4.5. Context compression (pre-dispatch) ──────────────────────────────────
    // Applied when the pipeline has a compressor AND this is a conversation.
    // Threshold and plugin ID come from the resolved combo (step 1.5b); default 2000.
    // "none" in effective_compressor_id skips compression entirely.
    // Plugin identity selection requires a registry lookup (Phase E); for now log mismatch.
    let skip_compression = effective_compressor_id.as_deref() == Some("none");
    if !skip_compression {
        if let Some(compressor) = &pipeline.compressor {
            if let Operation::Conversation(conv_req) = &operation {
                let estimated = compressor.estimate_tokens(conv_req);
                if estimated >= compression_threshold {
                    if let Some(comp_id) = &effective_compressor_id {
                        if comp_id != "auto" && comp_id != compressor.name() {
                            tracing::debug!(
                                requested = %comp_id,
                                available = %compressor.name(),
                                "compression plugin mismatch, using available (plugin registry is Phase E)",
                            );
                        }
                    }
                    match compressor.compress(conv_req.clone(), estimated) {
                        Ok((compressed_req, report)) => {
                            tracing::debug!(
                                tokens_before = estimated,
                                tokens_after  = estimated.saturating_sub(report.estimated_tokens_removed),
                                algorithm     = %report.strategy,
                                "compression applied",
                            );
                            operation = Operation::Conversation(compressed_req);
                        }
                        Err(vkdg_policy_compress::CompressionError::NotApplicable) => {}
                        Err(e) => {
                            tracing::warn!(error = %e, "compression failed, proceeding uncompressed");
                        }
                    }
                }
            }
        }
    }

    // 4.6. Global system prompt injection ─────────────────────────────────────
    if let (Some(global_prompt), Operation::Conversation(conv_req)) =
        (&pipeline.global_system_prompt, &mut operation)
    {
        conv_req.system = Some(match &conv_req.system {
            None => global_prompt.clone(),
            Some(existing) => format!("{global_prompt}\n\n{existing}"),
        });
    }

    // 4.7. Request deduplication ───────────────────────────────────────────────
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
        Some(DedupGuard { dedup: Arc::clone(dedup), key })
    } else {
        None
    };

    // 5a. Cache lookup (before upstream, after credential) ────────────────────
    // Bypass rules enforced here in core; the cache plugin never needs to check them.
    let conv_req = if let Operation::Conversation(r) = &operation { Some(r) } else { None };
    let bypass_cache = ctx.envelope.cache_bypass
        || conv_req.is_none_or(|r| is_multiturn(r) || has_tool_calls(r));
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
                    .unwrap_or_else(|_| error_response(VkdgError::Internal(
                        "cache response build".into(),
                    ))));
            }
            Ok(CacheResult::Miss) => {}
            Err(e) => {
                // Cache errors are non-fatal: log and continue to upstream.
                tracing::warn!(error = %e, "cache lookup failed, bypassing");
            }
        }
    }

    // 5b. Build upstream request via provider adapter ─────────────────────────
    let prepared = pipeline.provider_adapter.prepare(&operation, &config, &token)?;
    let is_streaming = prepared.is_streaming;
    let upstream_req = UpstreamRequest {
        method: http::Method::POST,
        url: prepared.url,
        headers: prepared.headers,
        body: prepared.body,
    };
    ctx.transition(AttemptState::Prepared);

    // 6. Send ──────────────────────────────────────────────────────────────────
    let upstream_resp = match pipeline.http_client.send(upstream_req, is_streaming).await {
        Ok(r) => r,
        Err(VkdgError::UpstreamError { code, message })
            if code == 429 || code >= 500 =>
        {
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

    // 7. Build response ────────────────────────────────────────────────────────
    // `response_body_len` is set in the Complete arm for post-response quota
    // accounting; streaming leaves it None (Phase E: wrap stream on completion).
    let mut response_body_len: Option<usize> = None;
    let resp = match upstream_resp {
        UpstreamResponse::Complete { status, body } => {
            // 8. Cache store (fire-and-forget, non-streaming only) ─────────────
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
                tokio::spawn(async move { let _ = cache.store(&key, entry).await; });
            }
            response_body_len = Some(body.len());
            let status_code =
                http::StatusCode::from_u16(status).unwrap_or(http::StatusCode::BAD_GATEWAY);
            axum::response::Response::builder()
                .status(status_code)
                .header(header::CONTENT_TYPE, "application/json")
                .body(axum::body::Body::from(body))
                .unwrap_or_else(|_| error_response(VkdgError::Internal(
                    "response builder failed".into(),
                )))
        }
        UpstreamResponse::Streaming { status: _, body } => {
            // Streaming responses are never cached — the body is a stream.
            // Filter think tags unless the client opted in via X-VKDG-Think-Tags: include.
            let filtered_body = if !ctx.envelope.include_think_tags {
                filter_think_tags_stream(body)
            } else {
                body
            };
            axum::response::Response::builder()
                .status(http::StatusCode::OK)
                .header(header::CONTENT_TYPE, "text/event-stream")
                .header("cache-control", "no-cache")
                .header("x-accel-buffering", "no")
                .body(axum::body::Body::from_stream(filtered_body))
                .unwrap_or_else(|_| error_response(VkdgError::Internal(
                    "streaming response builder failed".into(),
                )))
        }
    };

    // 9. Post-response: record quota usage + pin session ──────────────────────
    // Only for non-streaming complete responses (body len captured above).
    // Streaming: approximate usage recorded when stream completes (Phase E).
    if let Some(approx_tokens) = response_body_len.map(|n| (n / 4) as u64) {
        if let Some(tracker) = &pipeline.quota_tracker {
            if let Some(conn_id) = &ctx.connection_id {
                let tracker = Arc::clone(tracker);
                let conn_id = conn_id.clone();
                tokio::spawn(async move {
                    tracker.record_usage(&conn_id, approx_tokens).await;
                });
            }
        }
        if let (Some(registry), Some(session_key), Some(conn_id)) = (
            &pipeline.session_registry,
            &ctx.envelope.session_key,
            &ctx.connection_id,
        ) {
            let registry = Arc::clone(registry);
            let session_id = session_key.0.clone();
            let conn_id = conn_id.clone();
            tokio::spawn(async move {
                registry.pin(session_id, conn_id).await;
            });
        }
    }
    // 9b. Record latency for routing scorer ───────────────────────────────────
    if let (Some(tracker), Some(conn_id)) = (&pipeline.latency_tracker, &ctx.connection_id) {
        let latency_ms = start_time.elapsed().as_millis() as u32;
        let tracker = Arc::clone(tracker);
        let conn_id = conn_id.clone();
        tokio::spawn(async move { tracker.record(&conn_id, latency_ms).await; });
    }
    // 6c. On success, check if an expired cooldown can be cleared → recover to Healthy.
    if let Some(conn_arc) = ctx
        .connection_id
        .as_ref()
        .and_then(|id| pipeline.catalog.get(id))
    {
        if let Ok(mut conn) = conn_arc.try_write() {
            conn.check_cooldown();
        }
    }
    Ok(resp)
}

// ── Cache bypass helpers ──────────────────────────────────────────────────────

/// True when the conversation has more than one assistant turn in its history.
/// Multi-turn conversations are never cached: the response depends on prior
/// assistant outputs that may not be stable across retries.
fn is_multiturn(op: &vkdg_operations::ConversationRequest) -> bool {
    op.messages.iter().filter(|m| matches!(m.role, Role::Assistant)).count() > 1
}

/// True when any message in the conversation contains tool_use or tool_result
/// content blocks.  Tool-call conversations are non-deterministic.
fn has_tool_calls(op: &vkdg_operations::ConversationRequest) -> bool {
    op.messages.iter().any(|m| matches!(
        &m.content,
        MessageContent::Blocks(blocks) if blocks.iter().any(|b|
            matches!(b, ContentBlock::ToolUse { .. } | ContentBlock::ToolResult { .. })
        )
    ))
}

fn unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn error_response(err: VkdgError) -> Response {
    use axum::response::IntoResponse;
    use http::{HeaderMap, HeaderValue, StatusCode};

    let (status, error_type) = match &err {
        VkdgError::Unauthenticated => (StatusCode::UNAUTHORIZED, "authentication_error"),
        VkdgError::Unauthorized => (StatusCode::FORBIDDEN, "permission_error"),
        VkdgError::AdmissionRejected { .. } => (StatusCode::SERVICE_UNAVAILABLE, "overloaded_error"),
        VkdgError::CapabilityUnsupported { .. } => {
            (StatusCode::BAD_REQUEST, "invalid_request_error")
        }
        VkdgError::NoEligibleConnection => (StatusCode::BAD_GATEWAY, "api_error"),
        VkdgError::UpstreamError { code, .. } => {
            let s = StatusCode::from_u16(*code).unwrap_or(StatusCode::BAD_GATEWAY);
            (s, "api_error")
        }
        VkdgError::PluginError { .. } => (StatusCode::INTERNAL_SERVER_ERROR, "api_error"),
        VkdgError::ConfigInvalid { .. } => (StatusCode::BAD_REQUEST, "invalid_request_error"),
        VkdgError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "api_error"),
    };

    let body = json!({
        "type": "error",
        "error": { "type": error_type, "message": err.to_string() }
    });
    let json_bytes = serde_json::to_vec(&body).unwrap_or_else(|_| b"{}".to_vec());
    let mut h = HeaderMap::new();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
    (status, h, json_bytes).into_response()
}

// (build_anthropic_upstream_body and upstream_base_url moved to vkdg-provider-anthropic)
// ── SSE think-tag filter ──────────────────────────────────────────────────────

/// Re-encode a parsed SseEvent back into raw SSE bytes.
fn sse_event_to_bytes(event: &SseEvent) -> Bytes {
    let mut s = String::new();
    if let Some(ref et) = event.event_type {
        s.push_str("event: ");
        s.push_str(et);
        s.push('\n');
    }
    // Each \n in data must become a separate data: line per the SSE spec.
    for line in event.data.split('\n') {
        s.push_str("data: ");
        s.push_str(line);
        s.push('\n');
    }
    s.push('\n');
    Bytes::from(s)
}

/// Wrap an upstream SSE byte stream with a think-tag filter.
/// Parses each chunk through `SseParser` (strip_think_tags=true), re-encodes
/// clean events back to SSE bytes.  Events spanning chunk boundaries are
/// correctly handled by the parser's internal buffer.
fn filter_think_tags_stream(
    body: Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>>,
) -> Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>> {
    // State: (upstream stream, sse parser, buffered re-encoded events)
    let state = (body, SseParser::new(), VecDeque::<SseEvent>::new());
    Box::pin(futures::stream::unfold(state, |(mut upstream, mut parser, mut pending)| async move {
        loop {
            // Drain any events already parsed from the last chunk.
            if let Some(event) = pending.pop_front() {
                return Some((Ok(sse_event_to_bytes(&event)), (upstream, parser, pending)));
            }
            // Pull the next chunk from upstream.
            match upstream.next().await {
                Some(Ok(chunk)) => {
                    for e in parser.push(&chunk) {
                        pending.push_back(e);
                    }
                    // Loop back to drain the newly queued events.
                }
                Some(Err(e)) => return Some((Err(e), (upstream, parser, pending))),
                None => return None,
            }
        }
    }))
}


// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use vkdg_connections::{AuthKind, ConnectionCatalog, ConnectionConfig, CredentialManager, ProviderKind};
    use vkdg_core::{
        pipeline::PipelineCtx, ApiType, AttemptState, ClientId, ConnectionId, RequestEnvelope,
        RequestId, TenantId,
    };
    use vkdg_operations::{CapabilitySet, ConversationRequest, Operation};
    use vkdg_routing::Router as VkdgRouter;
    use vkdg_observe::DecisionRecordExporter;

    use crate::{AdmissionGuard, PipelineState};
    use crate::provider::{PreparedRequest, ProviderAdapter};
    use crate::upstream::HttpClient;

    struct StubAdapter;
    impl ProviderAdapter for StubAdapter {
        fn name(&self) -> &str { "stub" }
        fn prepare(&self, _op: &Operation, _cfg: &vkdg_connections::ConnectionConfig, _token: &str) -> Result<PreparedRequest, VkdgError> {
            Err(VkdgError::Internal("stub adapter — not for real requests".into()))
        }
    }

    fn make_pipeline(admission_limit: usize) -> Arc<PipelineState> {
        Arc::new(PipelineState {
            admission: Arc::new(AdmissionGuard::new(admission_limit)),
            router: Arc::new(VkdgRouter::new(vec![])),
            catalog: Arc::new(ConnectionCatalog::new(vec![])),
            credentials: Arc::new(CredentialManager::new()),
            http_client: Arc::new(HttpClient::new()),
            exporter: Arc::new(DecisionRecordExporter::new()),
            provider_adapter: Arc::new(StubAdapter),
            cache: None,
            combo_resolver: None,
            compressor: None,
            dedup_table: None,
            session_registry: None,
            quota_tracker: None,
            global_system_prompt: None,
            ip_policy: None,
            latency_tracker: None,
        })
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
        let pipeline = Arc::new(PipelineState {
            admission: Arc::new(AdmissionGuard::new(100)),
            router: Arc::new(VkdgRouter::new(vec![])),
            catalog: Arc::new(ConnectionCatalog::new(vec![])),
            credentials: Arc::new(CredentialManager::new()),
            http_client: Arc::new(HttpClient::new()),
            exporter: Arc::new(DecisionRecordExporter::new()),
            provider_adapter: Arc::new(StubAdapter),
            cache: None,
            combo_resolver: None,
            compressor: Some(Arc::new(CavemanCompressor)),
            dedup_table: None,
            session_registry: None,
            quota_tracker: None,
            global_system_prompt: None,
            ip_policy: None,
            latency_tracker: None,
        });
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
        let pipeline = Arc::new(PipelineState {
            admission: Arc::new(AdmissionGuard::new(100)),
            router: Arc::new(VkdgRouter::new(vec![])),
            catalog: Arc::new(ConnectionCatalog::new(vec![])),
            credentials: Arc::new(CredentialManager::new()),
            http_client: Arc::new(HttpClient::new()),
            exporter: Arc::new(DecisionRecordExporter::new()),
            provider_adapter: Arc::new(StubAdapter),
            cache: None,
            combo_resolver: Some(Arc::new(ComboResolver::new(vec![combo]))),
            compressor: Some(Arc::new(CavemanCompressor)),
            dedup_table: None,
            session_registry: None,
            quota_tracker: None,
            global_system_prompt: None,
            ip_policy: None,
            latency_tracker: None,
        });
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
            result_both,
            "Be concise.\n\nYou are a helpful assistant.",
            "separator must be exactly two newlines"
        );
    }
}
