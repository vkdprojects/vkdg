//! Encode internal `ConversationEvent` values into `OpenAI` Chat Completions SSE chunks.

use std::convert::Infallible;
use std::fmt::Write;

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

/// Encode a single `ConversationEvent` as an `OpenAI` SSE `data:` line.
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
        // OpenAI exposes model reasoning as a separate `reasoning_content` delta.
        ConversationEvent::ReasoningDelta { delta, index } => json!({
            "id": request_id,
            "object": "chat.completion.chunk",
            "created": 0,
            "model": "",
            "choices": [{"index": index, "delta": {"reasoning_content": delta}, "finish_reason": null}]
        }),

        // OpenAI has no per-tool-call terminator: arguments simply stop arriving.
        ConversationEvent::ToolCallEnd { .. } => return None,

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
            ..
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

// ── Responses API encoding ────────────────────────────────────────────────────

fn stop_reason_to_responses_stop_reason(reason: &StopReason) -> &'static str {
    match reason {
        StopReason::EndTurn | StopReason::StopSequence | StopReason::Cancelled => "end_turn",
        StopReason::MaxTokens => "max_output_tokens",
        StopReason::ToolUse => "tool_calls",
    }
}

/// Encode a single `ConversationEvent` as `OpenAI` Responses API SSE event string(s).
///
/// Returns `None` for events that should not produce an SSE frame (e.g. `Failed`).
/// `Started` emits three concatenated events; `Completed` emits three concatenated events.
pub fn encode_event_to_responses_chunk(
    event: &ConversationEvent,
    response_id: &str,
    item_id: &str,
) -> Option<String> {
    match event {
        ConversationEvent::Started { .. } => {
            let created = serde_json::json!({
                "type": "response.created",
                "response": {"id": response_id, "status": "in_progress"}
            });
            let item_added = serde_json::json!({
                "type": "response.output_item.added",
                "output_index": 0,
                "item": {"id": item_id, "type": "message", "role": "assistant", "content": []}
            });
            let part_added = serde_json::json!({
                "type": "response.content_part.added",
                "output_index": 0,
                "content_index": 0,
                "part": {"type": "output_text", "text": ""}
            });
            let s1 = serde_json::to_string(&created).unwrap_or_else(|_| "{}".to_string());
            let s2 = serde_json::to_string(&item_added).unwrap_or_else(|_| "{}".to_string());
            let s3 = serde_json::to_string(&part_added).unwrap_or_else(|_| "{}".to_string());
            Some(format!("data: {s1}\n\ndata: {s2}\n\ndata: {s3}\n\n"))
        }

        ConversationEvent::OutputDelta { delta, .. } => {
            let ev = serde_json::json!({
                "type": "response.output_text.delta",
                "output_index": 0,
                "content_index": 0,
                "item_id": item_id,
                "response_id": response_id,
                "delta": delta,
            });
            let s = serde_json::to_string(&ev).unwrap_or_else(|_| "{}".to_string());
            Some(format!("data: {s}\n\n"))
        }

        ConversationEvent::ReasoningDelta { delta, .. } => {
            let ev = serde_json::json!({
                "type": "response.reasoning_summary_text.delta",
                "delta": delta,
            });
            let s = serde_json::to_string(&ev).unwrap_or_else(|_| "{}".to_string());
            Some(format!("data: {s}\n\n"))
        }

        ConversationEvent::ToolCallDelta {
            tool_use_id,
            name,
            input_delta,
            index,
        } => {
            let mut out = String::new();
            if !name.is_empty() {
                let item_ev = serde_json::json!({
                    "type": "response.output_item.added",
                    "output_index": index,
                    "item": {
                        "id": tool_use_id,
                        "type": "function_call",
                        "call_id": tool_use_id,
                        "name": name,
                        "arguments": ""
                    }
                });
                let s = serde_json::to_string(&item_ev).unwrap_or_else(|_| "{}".to_string());
                write!(out, "data: {s}\n\n").unwrap();
            }
            let args_ev = serde_json::json!({
                "type": "response.function_call_arguments.delta",
                "output_index": index,
                "item_id": tool_use_id,
                "delta": input_delta,
            });
            let s = serde_json::to_string(&args_ev).unwrap_or_else(|_| "{}".to_string());
            write!(out, "data: {s}\n\n").unwrap();
            Some(out)
        }

        ConversationEvent::ToolCallEnd { .. } => {
            let ev = serde_json::json!({
                "type": "response.function_call_arguments.done",
                "arguments": "",
            });
            let s = serde_json::to_string(&ev).unwrap_or_else(|_| "{}".to_string());
            Some(format!("data: {s}\n\n"))
        }

        // Usage is rolled into response.completed; skip separate event.
        ConversationEvent::Usage { .. } => None,

        ConversationEvent::Completed { stop_reason } => {
            let sr = stop_reason_to_responses_stop_reason(stop_reason);
            let text_done = serde_json::json!({
                "type": "response.output_text.done",
                "output_index": 0,
                "content_index": 0,
                "text": "",
            });
            let item_done = serde_json::json!({
                "type": "response.output_item.done",
                "output_index": 0,
                "item": {"id": item_id, "type": "message", "role": "assistant"},
            });
            let completed = serde_json::json!({
                "type": "response.completed",
                "response": {
                    "id": response_id,
                    "status": "completed",
                    "stop_reason": sr,
                }
            });
            let s1 = serde_json::to_string(&text_done).unwrap_or_else(|_| "{}".to_string());
            let s2 = serde_json::to_string(&item_done).unwrap_or_else(|_| "{}".to_string());
            let s3 = serde_json::to_string(&completed).unwrap_or_else(|_| "{}".to_string());
            Some(format!("data: {s1}\n\ndata: {s2}\n\ndata: {s3}\n\n"))
        }

        ConversationEvent::Failed { .. } => None,
    }
}

/// Wrap a `ConversationEvent` stream into an SSE `axum::response::Response` using `OpenAI` format.
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

    // ── Responses API tests ───────────────────────────────────────────────────

    const RESP_ID: &str = "resp-test";
    const ITEM_ID: &str = "item-test";

    // Defeat: Started event not emitting all three required SSE events.
    #[test]
    fn encode_responses_started_emits_created_and_item() {
        let event = ConversationEvent::Started {
            request_id: RequestId::new(),
        };
        let chunk = encode_event_to_responses_chunk(&event, RESP_ID, ITEM_ID).unwrap();
        // Must contain all three event types.
        assert!(
            chunk.contains("\"type\":\"response.created\""),
            "Missing response.created, got: {chunk}"
        );
        assert!(
            chunk.contains("\"type\":\"response.output_item.added\""),
            "Missing response.output_item.added, got: {chunk}"
        );
        assert!(
            chunk.contains("\"type\":\"response.content_part.added\""),
            "Missing response.content_part.added, got: {chunk}"
        );
        // All three must be valid JSON objects in separate data: lines.
        let data_lines: Vec<&str> = chunk
            .split("\n\n")
            .filter(|l| l.starts_with("data: "))
            .collect();
        assert_eq!(
            data_lines.len(),
            3,
            "Expected 3 data: lines, got: {data_lines:?}"
        );
        for line in &data_lines {
            let json_str = line.strip_prefix("data: ").unwrap();
            let v: serde_json::Value = serde_json::from_str(json_str)
                .unwrap_or_else(|e| panic!("Invalid JSON in line '{line}': {e}"));
            assert!(v.get("type").is_some(), "Missing 'type' field in: {v}");
        }
    }

    // Defeat: OutputDelta not mapping to response.output_text.delta with correct delta field.
    #[test]
    fn encode_responses_output_delta() {
        let event = ConversationEvent::OutputDelta {
            delta: "world".to_string(),
            index: 0,
        };
        let chunk = encode_event_to_responses_chunk(&event, RESP_ID, ITEM_ID).unwrap();
        assert!(
            chunk.contains("\"type\":\"response.output_text.delta\""),
            "Wrong event type, got: {chunk}"
        );
        assert!(
            chunk.contains("\"delta\":\"world\""),
            "Missing delta text, got: {chunk}"
        );
    }

    // Defeat: Completed not emitting response.completed with status=completed.
    #[test]
    fn encode_responses_completed() {
        let event = ConversationEvent::Completed {
            stop_reason: StopReason::EndTurn,
        };
        let chunk = encode_event_to_responses_chunk(&event, RESP_ID, ITEM_ID).unwrap();
        assert!(
            chunk.contains("\"type\":\"response.completed\""),
            "Missing response.completed, got: {chunk}"
        );
        assert!(
            chunk.contains("\"status\":\"completed\""),
            "Missing status:completed, got: {chunk}"
        );
        // Last data: line must be response.completed.
        let last_data = chunk
            .split("\n\n")
            .filter(|l| l.starts_with("data: "))
            .last()
            .unwrap();
        let json_str = last_data.strip_prefix("data: ").unwrap();
        let v: serde_json::Value = serde_json::from_str(json_str).unwrap();
        assert_eq!(
            v["type"].as_str(),
            Some("response.completed"),
            "Last event must be response.completed, got: {v}"
        );
    }
}
