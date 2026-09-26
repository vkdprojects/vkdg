use std::collections::VecDeque;
use std::pin::Pin;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::response::Response;
use bytes::Bytes;
use futures::{Stream, StreamExt};
use http::header;
use serde_json::json;
use vkdg_core::VkdgError;
use vkdg_operations::{ContentBlock, MessageContent, Role};

use crate::sse::{SseEvent, SseParser};

// ── Cache bypass helpers ──────────────────────────────────────────────────────

/// True when the conversation has more than one assistant turn in its history.
/// Multi-turn conversations are never cached: the response depends on prior
/// assistant outputs that may not be stable across retries.
pub(super) fn is_multiturn(op: &vkdg_operations::ConversationRequest) -> bool {
    op.messages.iter().filter(|m| matches!(m.role, Role::Assistant)).count() > 1
}

/// True when any message in the conversation contains tool_use or tool_result
/// content blocks.  Tool-call conversations are non-deterministic.
pub(super) fn has_tool_calls(op: &vkdg_operations::ConversationRequest) -> bool {
    op.messages.iter().any(|m| matches!(
        &m.content,
        MessageContent::Blocks(blocks) if blocks.iter().any(|b|
            matches!(b, ContentBlock::ToolUse { .. } | ContentBlock::ToolResult { .. })
        )
    ))
}

pub(super) fn unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ── Helpers ───────────────────────────────────────────────────────────────────

pub(super) fn error_response(err: VkdgError) -> Response {
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

// ── SSE think-tag filter ──────────────────────────────────────────────────────

/// Re-encode a parsed SseEvent back into raw SSE bytes.
pub(super) fn sse_event_to_bytes(event: &SseEvent) -> Bytes {
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
pub(super) fn filter_think_tags_stream(
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
