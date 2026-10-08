//! A complete `OpenAI` Chat Completions response to canonical events.
//!
//! Same vocabulary as [`OpenAiSseDecoder`](super::OpenAiSseDecoder). Only choice 0 is
//! decoded. Tool call arguments are already a JSON string and pass through as is.

use serde_json::Value;
use vkdg_core::{RequestId, VkdgError};
use vkdg_operations::{parse_openai_finish_reason, ConversationEvent, StopReason};

use super::openai_sse::error_status;
use super::token_counts::TokenCounts;
use super::upstream_failure::capped;

const NOT_A_COMPLETION: &str = "upstream returned a response that is not an openai chat completion";

fn upstream(code: u16, message: &str) -> VkdgError {
    VkdgError::UpstreamError {
        code,
        message: message.to_owned(),
        retry_after: None,
    }
}

/// `content` as text: a string, or the text parts of an array (some compatible providers).
fn content_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect(),
        _ => String::new(),
    }
}

fn str_of<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// Decodes one `chat.completion` object. An `error` object, a body of another dialect
/// or invalid JSON is an error; the failure never echoes the body.
pub fn decode_openai_json(body: &[u8]) -> Result<Vec<ConversationEvent>, VkdgError> {
    let completion: Value =
        serde_json::from_slice(body).map_err(|_| upstream(502, NOT_A_COMPLETION))?;
    if let Some(error) = completion.get("error").filter(|e| e.is_object()) {
        return Err(upstream(
            error_status(str_of(error, "type")),
            &capped(str_of(error, "message")),
        ));
    }
    let choice = completion
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| {
            choices
                .iter()
                .find(|c| c.get("index").and_then(Value::as_u64).unwrap_or(0) == 0)
        })
        .ok_or_else(|| upstream(502, NOT_A_COMPLETION))?;
    let message = choice.get("message").unwrap_or(&Value::Null);

    let mut events = vec![ConversationEvent::Started {
        request_id: RequestId::new(),
    }];
    let reasoning = message
        .get("reasoning_content")
        .or_else(|| message.get("reasoning"))
        .map(content_text)
        .unwrap_or_default();
    if !reasoning.is_empty() {
        events.push(ConversationEvent::ReasoningDelta {
            delta: reasoning,
            index: 0,
        });
    }
    let text = message.get("content").map(content_text).unwrap_or_default();
    if !text.is_empty() {
        events.push(ConversationEvent::OutputDelta {
            delta: text,
            index: 0,
        });
    }
    let calls = message
        .get("tool_calls")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    for (position, call) in calls.iter().enumerate() {
        let index = u32::try_from(position).unwrap_or(u32::MAX);
        let function = call.get("function").unwrap_or(&Value::Null);
        events.push(ConversationEvent::ToolCallDelta {
            tool_use_id: str_of(call, "id").to_owned(),
            name: str_of(function, "name").to_owned(),
            input_delta: str_of(function, "arguments").to_owned(),
            index,
        });
        events.push(ConversationEvent::ToolCallEnd { index });
    }
    let mut counts = TokenCounts::default();
    if completion
        .get("usage")
        .is_some_and(|u| counts.merge_openai(u))
    {
        events.push(counts.event());
    }
    let reason = match choice.get("finish_reason").and_then(Value::as_str) {
        Some(reason) => parse_openai_finish_reason(reason),
        None if !calls.is_empty() => StopReason::ToolUse,
        None => StopReason::EndTurn,
    };
    events.push(ConversationEvent::Completed {
        stop_reason: reason,
    });
    Ok(events)
}
