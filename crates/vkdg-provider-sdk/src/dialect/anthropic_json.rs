//! A complete Anthropic Messages response to canonical events.
//!
//! The same vocabulary and the same losses as [`AnthropicSseDecoder`](super::AnthropicSseDecoder):
//! signatures, redacted thinking and server-side tool blocks are dropped; source block
//! indices are the position in `content`.

use serde_json::Value;
use vkdg_core::{RequestId, VkdgError};
use vkdg_operations::{parse_anthropic_stop_reason, ConversationEvent};

use super::anthropic_sse::error_status;
use super::token_counts::TokenCounts;
use super::upstream_failure::capped;

const NOT_A_MESSAGE: &str = "upstream returned a response that is not an anthropic message";

fn upstream(code: u16, message: &str) -> VkdgError {
    VkdgError::UpstreamError {
        code,
        message: message.to_owned(),
        retry_after: None,
    }
}

fn str_of<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// Decodes one Anthropic `message` object. An `error` object, a body of another
/// dialect or invalid JSON is an error; the failure never echoes the body.
pub fn decode_anthropic_json(body: &[u8]) -> Result<Vec<ConversationEvent>, VkdgError> {
    let message: Value = serde_json::from_slice(body).map_err(|_| upstream(502, NOT_A_MESSAGE))?;
    if str_of(&message, "type") == "error" {
        let error = message.get("error").unwrap_or(&message);
        return Err(upstream(
            error_status(str_of(error, "type")),
            &capped(str_of(error, "message")),
        ));
    }
    let content = message
        .get("content")
        .and_then(Value::as_array)
        .filter(|_| str_of(&message, "type") == "message")
        .ok_or_else(|| upstream(502, NOT_A_MESSAGE))?;

    let mut events = vec![ConversationEvent::Started {
        request_id: RequestId::new(),
    }];
    let mut tool_called = false;
    for (position, block) in content.iter().enumerate() {
        let index = u32::try_from(position).unwrap_or(u32::MAX);
        match str_of(block, "type") {
            "text" => {
                let text = str_of(block, "text");
                if !text.is_empty() {
                    events.push(ConversationEvent::OutputDelta {
                        delta: text.to_owned(),
                        index,
                    });
                }
            }
            "thinking" => {
                let text = str_of(block, "thinking");
                if !text.is_empty() {
                    events.push(ConversationEvent::ReasoningDelta {
                        delta: text.to_owned(),
                        index,
                    });
                }
            }
            "tool_use" => {
                tool_called = true;
                let input = block.get("input").unwrap_or(&Value::Null);
                events.push(ConversationEvent::ToolCallDelta {
                    tool_use_id: str_of(block, "id").to_owned(),
                    name: str_of(block, "name").to_owned(),
                    input_delta: if input.is_null() {
                        "{}".to_owned()
                    } else {
                        input.to_string()
                    },
                    index,
                });
                events.push(ConversationEvent::ToolCallEnd { index });
            }
            // redacted_thinking, server_tool_use, *_tool_result and future blocks.
            _ => {}
        }
    }
    let mut counts = TokenCounts::default();
    if message
        .get("usage")
        .is_some_and(|u| counts.merge_anthropic(u))
    {
        events.push(counts.event());
    }
    let reason = match message.get("stop_reason").and_then(Value::as_str) {
        Some(reason) => parse_anthropic_stop_reason(reason),
        None if tool_called => vkdg_operations::StopReason::ToolUse,
        None => vkdg_operations::StopReason::EndTurn,
    };
    events.push(ConversationEvent::Completed {
        stop_reason: reason,
    });
    Ok(events)
}
