//! Encode internal `ConversationEvent` values into OpenAI Chat Completions SSE chunks.

use std::convert::Infallible;

use axum::response::sse::Event;
use axum::response::{IntoResponse, Response, Sse};
use futures::Stream;
use serde_json::json;

use vkdg_operations::{ConversationEvent, StopReason, UsageCount};

// ── Encoding helpers ──────────────────────────────────────────────────────────

fn stop_reason_to_finish_reason(reason: &StopReason) -> &'static str {
    match reason {
        StopReason::EndTurn | StopReason::StopSequence | StopReason::Cancelled => "stop",
        StopReason::MaxTokens => "length",
        StopReason::ToolUse => "tool_calls",
    }
}

fn usage_count_to_u32(count: &UsageCount) -> u32 {
    match count {
        UsageCount::Reported(n) | UsageCount::Estimated(n) => *n,
        UsageCount::Unknown => 0,
    }
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Encode a single `ConversationEvent` as an OpenAI SSE `data:` line.
///
/// Returns `None` for events that should not produce an SSE frame (e.g. `Failed`).
/// `Completed` appends a `data: [DONE]` sentinel after the finish-reason chunk.
pub fn encode_event_to_oai_chunk(event: &ConversationEvent, request_id: &str) -> Option<String> {
    let chunk = match event {
        ConversationEvent::Started { .. } => json!({
            "id": request_id,
            "object": "chat.completion.chunk",
            "created": 0,
            "model": "",
            "choices": [{"index": 0, "delta": {"role": "assistant", "content": ""}, "finish_reason": null}]
        }),

        ConversationEvent::OutputDelta { delta, index } => json!({
            "id": request_id,
            "object": "chat.completion.chunk",
            "created": 0,
            "model": "",
            "choices": [{"index": index, "delta": {"content": delta}, "finish_reason": null}]
        }),

        ConversationEvent::ToolCallDelta {
            tool_use_id,
            name,
            input_delta,
            index,
        } => json!({
            "id": request_id,
            "object": "chat.completion.chunk",
            "created": 0,
            "model": "",
            "choices": [{
                "index": 0,
                "delta": {
                    "tool_calls": [{
                        "index": index,
                        "id": tool_use_id,
                        "type": "function",
                        "function": {"name": name, "arguments": input_delta}
                    }]
                },
                "finish_reason": null
            }]
        }),

        ConversationEvent::Usage {
            input_tokens,
            output_tokens,
        } => json!({
            "id": request_id,
            "object": "chat.completion.chunk",
            "created": 0,
            "model": "",
            "choices": [],
            "usage": {
                "prompt_tokens": usage_count_to_u32(input_tokens),
                "completion_tokens": usage_count_to_u32(output_tokens)
            }
        }),

        ConversationEvent::Completed { stop_reason } => {
            let finish = stop_reason_to_finish_reason(stop_reason);
            let chunk_json = json!({
                "id": request_id,
                "object": "chat.completion.chunk",
                "created": 0,
                "model": "",
                "choices": [{"index": 0, "delta": {}, "finish_reason": finish}]
            });
            let chunk_str = serde_json::to_string(&chunk_json).unwrap_or_else(|_| "{}".to_string());
            return Some(format!("data: {chunk_str}\n\ndata: [DONE]\n\n"));
        }

        // Failed events are handled at the pipeline/handler level; no SSE frame emitted.
        ConversationEvent::Failed { .. } => return None,
    };

    let s = serde_json::to_string(&chunk).unwrap_or_else(|_| "{}".to_string());
    Some(format!("data: {s}\n\n"))
}

/// Wrap a `ConversationEvent` stream into an SSE `axum::response::Response` using OpenAI format.
pub fn events_to_sse_stream(
    events: impl Stream<Item = ConversationEvent> + Send + 'static,
) -> Response {
    use futures::StreamExt;

    let sse_stream = events.filter_map(|event| async move {
        let request_id = "chatcmpl-passthrough";
        let chunk_text = encode_event_to_oai_chunk(&event, request_id)?;
        // Strip leading "data: " and trailing "\n\n" — axum's Sse adds them.
        let data = chunk_text
            .strip_prefix("data: ")
            .unwrap_or(&chunk_text)
            .trim_end_matches('\n')
            .to_string();
        Some(Ok::<Event, Infallible>(Event::default().data(data)))
    });

    Sse::new(sse_stream)
        .keep_alive(axum::response::sse::KeepAlive::default())
        .into_response()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use vkdg_core::RequestId;
    use vkdg_operations::StopReason;

    const REQ_ID: &str = "chatcmpl-test";

    // Defeat: omitting role:assistant from the Started chunk, breaking streaming clients.
    #[test]
    fn encode_started_has_assistant_role() {
        let event = ConversationEvent::Started {
            request_id: RequestId::new(),
        };
        let chunk = encode_event_to_oai_chunk(&event, REQ_ID).unwrap();
        assert!(
            chunk.contains("\"role\":\"assistant\""),
            "Started chunk must contain role:assistant, got: {chunk}"
        );
    }

    // Defeat: emitting delta.content with wrong key or empty string loss.
    #[test]
    fn encode_output_delta_has_content() {
        let event = ConversationEvent::OutputDelta {
            delta: "hello".to_string(),
            index: 0,
        };
        let chunk = encode_event_to_oai_chunk(&event, REQ_ID).unwrap();
        assert!(
            chunk.contains("\"content\":\"hello\""),
            "OutputDelta chunk must carry content text, got: {chunk}"
        );
    }

    // Defeat: missing finish_reason or [DONE] sentinel, breaking streaming clients.
    #[test]
    fn encode_completed_stop_has_finish_reason_and_done() {
        let event = ConversationEvent::Completed {
            stop_reason: StopReason::EndTurn,
        };
        let chunk = encode_event_to_oai_chunk(&event, REQ_ID).unwrap();
        assert!(
            chunk.contains("\"finish_reason\":\"stop\""),
            "Completed/EndTurn must have finish_reason:stop, got: {chunk}"
        );
        assert!(
            chunk.contains("[DONE]"),
            "Completed chunk must end with [DONE], got: {chunk}"
        );
    }

    // Defeat: mapping ToolUse stop reason to "stop" instead of "tool_calls".
    #[test]
    fn encode_completed_tool_calls_finish_reason() {
        let event = ConversationEvent::Completed {
            stop_reason: StopReason::ToolUse,
        };
        let chunk = encode_event_to_oai_chunk(&event, REQ_ID).unwrap();
        assert!(
            chunk.contains("\"finish_reason\":\"tool_calls\""),
            "Completed/ToolUse must have finish_reason:tool_calls, got: {chunk}"
        );
    }
}
