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
    op.messages
        .iter()
        .filter(|m| matches!(m.role, Role::Assistant))
        .count()
        > 1
}

/// True when any message in the conversation contains tool_use or tool_result
/// content blocks.  Tool-call conversations are non-deterministic.
pub(super) fn has_tool_calls(op: &vkdg_operations::ConversationRequest) -> bool {
    op.messages.iter().any(|m| {
        matches!(
            &m.content,
            MessageContent::Blocks(blocks) if blocks.iter().any(|b|
                matches!(b, ContentBlock::ToolUse { .. } | ContentBlock::ToolResult { .. })
            )
        )
    })
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
        VkdgError::AdmissionRejected { .. } => {
            (StatusCode::SERVICE_UNAVAILABLE, "overloaded_error")
        }
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
        VkdgError::BudgetExceeded { .. } => (StatusCode::PAYMENT_REQUIRED, "budget_exceeded_error"),
    };

    let body = json!({
        "type": "error",
        "error": { "type": error_type, "message": err.to_string() }
    });
    let json_bytes = serde_json::to_vec(&body).unwrap_or_else(|_| b"{}".to_vec());
    let mut h = HeaderMap::new();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
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
    Box::pin(futures::stream::unfold(
        state,
        |(mut upstream, mut parser, mut pending)| async move {
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
        },
    ))
}

/// Decodes a protocol-specific stream (e.g., AWS EventStream) to SSE events
/// encoded in the client's API dialect (Anthropic Messages or OpenAI ChatCompletions).
pub(super) fn decode_stream_to_sse(
    body: Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>>,
    decoder: Box<dyn vkdg_provider_sdk::ConversationStreamDecoder>,
    api_type: vkdg_core::ApiType,
) -> Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>> {
    // State: (upstream stream, decoder, buffered events, api_type for encode)
    // Use ApiType that implements Clone
    let state = (body, decoder, VecDeque::<vkdg_operations::ConversationEvent>::new(), api_type.clone());
    Box::pin(futures::stream::unfold(
        state,
        |(mut upstream, mut decoder, mut pending, api_type)| async move {
            loop {
                // Drain any events already decoded from the last chunk.
                if let Some(event) = pending.pop_front() {
                    let encoded = encode_event_for_api_type(&event, api_type.clone());
                    return Some((Ok(Bytes::from(encoded)), (upstream, decoder, pending, api_type)));
                }
                // Pull the next chunk from upstream.
                match upstream.next().await {
                    Some(Ok(chunk)) => {
                        for e in decoder.feed(chunk) {
                            pending.push_back(e);
                        }
                    }
                    Some(Err(e)) => return Some((Err(e), (upstream, decoder, pending, api_type))),
                    None => return None,
                }
            }
        },
    ))
}

/// Encodes a single ConversationEvent to SSE format for the given API type.
fn encode_event_for_api_type(event: &vkdg_operations::ConversationEvent, api_type: vkdg_core::ApiType) -> String {
    match api_type {
        vkdg_core::ApiType::AnthropicMessages => encode_event_anthropic(event),
        vkdg_core::ApiType::OpenAiChatCompletions => encode_event_openai(event),
        _ => {
            // Default: JSON serialize as SSE data
            let json = serde_json::to_string(event).unwrap_or_else(|_| "{}".to_string());
            format!("data: {}\n\n", json)
        }
    }
}

/// Encode a single ConversationEvent to Anthropic Messages streaming format.
fn encode_event_anthropic(event: &vkdg_operations::ConversationEvent) -> String {
    use vkdg_operations::ConversationEvent::*;
    match event {
        Started { .. } => {
            "event: message_start\ndata: {}\n\n".to_string()
        }
        OutputDelta { delta, index: _ } => {
            let json = serde_json::json!({
                "index": 0,
                "type": "content_block_delta",
                "delta": { "type": "text_delta", "text": delta }
            });
            format!("event: content_block_delta\ndata: {}\n\n", serde_json::to_string(&json).unwrap_or_default())
        }
        Completed { stop_reason } => {
            let reason = match stop_reason {
                vkdg_operations::StopReason::EndTurn => "end_turn",
                vkdg_operations::StopReason::MaxTokens => "max_tokens",
                vkdg_operations::StopReason::StopSequence => "stop_sequence",
                vkdg_operations::StopReason::ToolUse => "tool_use",
                vkdg_operations::StopReason::Cancelled => "end_turn",
            };
            let json = serde_json::json!({
                "type": "message_delta",
                "delta": { "stop_reason": reason },
                "usage": { "output_tokens": 0 }
            });
            format!(
                "event: content_block_stop\ndata: {{}}\n\nevent: message_delta\ndata: {}\n\nevent: message_stop\ndata: {{}}\n\n",
                serde_json::to_string(&json).unwrap_or_default()
            )
        }
        Failed { error } => {
            let json = serde_json::json!({ "type": "error", "error": { "type": "server_error", "message": error.to_string() } });
            format!("event: error\ndata: {}\n\n", serde_json::to_string(&json).unwrap_or_default())
        }
        Usage { input_tokens, output_tokens } => {
            let input = match input_tokens { vkdg_operations::UsageCount::Reported(n) => *n, vkdg_operations::UsageCount::Estimated(n) => *n, vkdg_operations::UsageCount::Unknown => 0 };
            let output = match output_tokens { vkdg_operations::UsageCount::Reported(n) => *n, vkdg_operations::UsageCount::Estimated(n) => *n, vkdg_operations::UsageCount::Unknown => 0 };
            let json = serde_json::json!({
                "type": "message_delta",
                "usage": { "input_tokens": input, "output_tokens": output }
            });
            format!("event: message_delta\ndata: {}\n\n", serde_json::to_string(&json).unwrap_or_default())
        }
        _ => String::new(),
    }
}

/// Encode a single ConversationEvent to OpenAI ChatCompletions streaming format.
fn encode_event_openai(event: &vkdg_operations::ConversationEvent) -> String {
    use vkdg_operations::ConversationEvent::*;
    match event {
        OutputDelta { delta, index: _ } => {
            let json = serde_json::json!({
                "id": "chatcmpl",
                "object": "chat.completion.chunk",
                "created": 0,
                "model": "",
                "choices": [{ "index": 0, "delta": { "content": delta }, "finish_reason": serde_json::Value::Null }]
            });
            format!("data: {}\n\n", serde_json::to_string(&json).unwrap_or_default())
        }
        Completed { stop_reason } => {
            let reason = match stop_reason {
                vkdg_operations::StopReason::EndTurn => "stop",
                vkdg_operations::StopReason::MaxTokens => "length",
                vkdg_operations::StopReason::StopSequence => "stop",
                vkdg_operations::StopReason::ToolUse => "tool_calls",
                vkdg_operations::StopReason::Cancelled => "stop",
            };
            let json = serde_json::json!({
                "id": "chatcmpl",
                "object": "chat.completion.chunk",
                "created": 0,
                "model": "",
                "choices": [{ "index": 0, "delta": {}, "finish_reason": reason }]
            });
            format!("data: {}\n\ndata: [DONE]\n\n", serde_json::to_string(&json).unwrap_or_default())
        }
        Failed { error } => {
            let json = serde_json::json!({ "error": { "message": error.to_string(), "type": "server_error" } });
            format!("data: {}\n\n", serde_json::to_string(&json).unwrap_or_default())
        }
        _ => String::new(),
    }
}
