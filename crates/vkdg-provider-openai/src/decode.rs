//! Parse OpenAI Chat Completions SSE chunks and Images API responses.
//!
//! `parse_sse_line` returns a `Vec` rather than `Option` to handle parallel
//! tool calls in a single chunk (where `delta.tool_calls` may contain multiple
//! entries).

use serde::Deserialize;
use vkdg_core::RequestId;
use vkdg_operations::{ConversationEvent, ImageOutput, ImageResponse, StopReason, UsageCount};

// ── Wire types ────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct OaiChunk {
    #[serde(default)]
    choices: Vec<OaiChoice>,
    usage: Option<OaiUsage>,
}

#[derive(Debug, Deserialize)]
struct OaiChoice {
    delta: OaiDelta,
    finish_reason: Option<String>,
    #[allow(dead_code)]
    index: u32,
}

#[derive(Debug, Deserialize, Default)]
struct OaiDelta {
    role: Option<String>,
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<OaiToolCallChunk>>,
}

#[derive(Debug, Deserialize)]
struct OaiToolCallChunk {
    index: u32,
    id: Option<String>,
    function: Option<OaiToolCallFunctionChunk>,
}

#[derive(Debug, Deserialize)]
struct OaiToolCallFunctionChunk {
    name: Option<String>,
    arguments: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OaiUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Parse one SSE data line from an OpenAI Chat Completions stream.
///
/// Returns a `Vec` to support parallel tool calls where a single chunk may
/// contain multiple `tool_calls` entries, each producing its own event.
/// Returns an empty vec for `[DONE]` and unrecognised/no-op chunks.
pub fn parse_sse_line(data: &str, request_id: &RequestId) -> Vec<ConversationEvent> {
    if data == "[DONE]" {
        return vec![];
    }

    let chunk: OaiChunk = match serde_json::from_str(data) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(err = %e, "openai: failed to parse SSE chunk");
            return vec![];
        }
    };

    // Usage-only chunk (sent by stream_options:{include_usage:true}).
    if let Some(usage) = chunk.usage {
        if chunk.choices.is_empty() {
            return vec![ConversationEvent::Usage {
                input_tokens: UsageCount::Reported(usage.prompt_tokens),
                output_tokens: UsageCount::Reported(usage.completion_tokens),
            }];
        }
    }

    let choice = match chunk.choices.into_iter().next() {
        Some(c) => c,
        None => return vec![],
    };

    // finish_reason chunk — must check before delta to avoid emitting spurious events.
    if let Some(reason) = &choice.finish_reason {
        return vec![ConversationEvent::Completed {
            stop_reason: parse_stop_reason(reason),
        }];
    }

    let delta = choice.delta;

    // Started: role:assistant with no content or tool_calls.
    if delta.role.as_deref() == Some("assistant")
        && delta.content.is_none()
        && delta.tool_calls.is_none()
    {
        return vec![ConversationEvent::Started {
            request_id: request_id.clone(),
        }];
    }

    // Content delta.
    if let Some(text) = delta.content {
        if !text.is_empty() {
            return vec![ConversationEvent::OutputDelta {
                delta: text,
                index: 0,
            }];
        }
    }

    // Tool call deltas — one event per entry in the array (handles parallel calls).
    if let Some(tool_calls) = delta.tool_calls {
        let events: Vec<ConversationEvent> = tool_calls
            .into_iter()
            .filter_map(|tc| {
                let func = tc.function?;
                if let Some(name) = func.name {
                    // First chunk for this tool call: carries the name.
                    Some(ConversationEvent::ToolCallDelta {
                        tool_use_id: tc.id.unwrap_or_default(),
                        name,
                        input_delta: String::new(),
                        index: tc.index,
                    })
                } else if let Some(args) = func.arguments {
                    // Subsequent chunks carry argument fragments.
                    Some(ConversationEvent::ToolCallDelta {
                        tool_use_id: tc.id.unwrap_or_default(),
                        name: String::new(),
                        input_delta: args,
                        index: tc.index,
                    })
                } else {
                    None
                }
            })
            .collect();
        if !events.is_empty() {
            return events;
        }
    }

    vec![]
}

/// Map an OpenAI `finish_reason` string to a [`StopReason`].
fn parse_stop_reason(reason: &str) -> StopReason {
    match reason {
        "stop" => StopReason::EndTurn,
        "tool_calls" => StopReason::ToolUse,
        "length" => StopReason::MaxTokens,
        other => {
            tracing::warn!(
                reason = other,
                "openai: unknown finish_reason; treating as EndTurn"
            );
            StopReason::EndTurn
        }
    }
}

// ── Images API wire types ─────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct OaiImagesResponse {
    created: u64,
    data: Vec<OaiImageDatum>,
}

#[derive(Debug, Deserialize)]
struct OaiImageDatum {
    url: Option<String>,
    b64_json: Option<String>,
    revised_prompt: Option<String>,
}

// ── Images API public API ─────────────────────────────────────────────────────

/// Parse an OpenAI Images API response body into an [`ImageResponse`].
///
/// OpenAI format: `{"created": 1234567890, "data": [{"url": "...", "revised_prompt": "..."}]}`
pub fn parse_image_response(
    body: &[u8],
    request_id: &RequestId,
) -> Result<ImageResponse, vkdg_core::VkdgError> {
    let raw: OaiImagesResponse = serde_json::from_slice(body)
        .map_err(|e| vkdg_core::VkdgError::Internal(format!("parse_image_response: {e}")))?;

    let data = raw
        .data
        .into_iter()
        .map(|d| ImageOutput {
            url: d.url,
            b64_json: d.b64_json,
            revised_prompt: d.revised_prompt,
        })
        .collect();

    Ok(ImageResponse {
        request_id: request_id.clone(),
        created: raw.created,
        data,
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use vkdg_core::RequestId;

    fn rid() -> RequestId {
        RequestId::default()
    }

    /// role:assistant chunk with no content should produce a Started event.
    #[test]
    fn parse_started_from_role_chunk() {
        let data = r#"{"choices":[{"index":0,"delta":{"role":"assistant"},"finish_reason":null}]}"#;
        let events = parse_sse_line(data, &rid());
        assert!(
            matches!(events.as_slice(), [ConversationEvent::Started { .. }]),
            "{events:?}"
        );
    }

    /// delta.content should produce an OutputDelta event with the text.
    #[test]
    fn parse_output_delta() {
        let data = r#"{"choices":[{"index":0,"delta":{"content":"hello"},"finish_reason":null}]}"#;
        let events = parse_sse_line(data, &rid());
        assert!(
            matches!(
                events.as_slice(),
                [ConversationEvent::OutputDelta { delta, .. }] if delta == "hello"
            ),
            "{events:?}"
        );
    }

    /// First tool_call chunk (with name) should yield a ToolCallDelta carrying the name.
    #[test]
    fn parse_tool_call_name_chunk() {
        let data = r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"get_weather","arguments":""}}]},"finish_reason":null}]}"#;
        let events = parse_sse_line(data, &rid());
        assert!(
            matches!(
                events.as_slice(),
                [ConversationEvent::ToolCallDelta { tool_use_id, name, .. }]
                    if tool_use_id == "call_1" && name == "get_weather"
            ),
            "{events:?}"
        );
    }

    /// Subsequent tool_call chunks carry argument fragments, not name.
    #[test]
    fn parse_tool_call_args_chunk() {
        let data = r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"loc"}}]},"finish_reason":null}]}"#;
        let events = parse_sse_line(data, &rid());
        assert!(
            matches!(
                events.as_slice(),
                [ConversationEvent::ToolCallDelta { input_delta, name, .. }]
                    if input_delta.contains("{\"loc") && name.is_empty()
            ),
            "{events:?}"
        );
    }

    /// Usage chunk with empty choices must produce a Usage event.
    #[test]
    fn parse_usage_chunk() {
        let data = r#"{"choices":[],"usage":{"prompt_tokens":10,"completion_tokens":20}}"#;
        let events = parse_sse_line(data, &rid());
        assert!(
            matches!(
                events.as_slice(),
                [ConversationEvent::Usage {
                    input_tokens: UsageCount::Reported(10),
                    output_tokens: UsageCount::Reported(20),
                }]
            ),
            "{events:?}"
        );
    }

    /// finish_reason:stop must produce Completed{EndTurn}.
    #[test]
    fn parse_finish_reason_stop() {
        let data = r#"{"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#;
        let events = parse_sse_line(data, &rid());
        assert!(
            matches!(
                events.as_slice(),
                [ConversationEvent::Completed {
                    stop_reason: StopReason::EndTurn
                }]
            ),
            "{events:?}"
        );
    }

    /// finish_reason:tool_calls must produce Completed{ToolUse}.
    #[test]
    fn parse_finish_reason_tool_calls() {
        let data = r#"{"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}"#;
        let events = parse_sse_line(data, &rid());
        assert!(
            matches!(
                events.as_slice(),
                [ConversationEvent::Completed {
                    stop_reason: StopReason::ToolUse
                }]
            ),
            "{events:?}"
        );
    }

    /// [DONE] sentinel must produce an empty vec.
    #[test]
    fn parse_done_sentinel() {
        let events = parse_sse_line("[DONE]", &rid());
        assert!(events.is_empty(), "{events:?}");
    }

    /// A chunk with two parallel tool_calls must produce two ToolCallDelta events.
    #[test]
    fn parse_parallel_tool_calls() {
        let data = r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_a","function":{"name":"tool_a","arguments":""}},{"index":1,"id":"call_b","function":{"name":"tool_b","arguments":""}}]},"finish_reason":null}]}"#;
        let events = parse_sse_line(data, &rid());
        assert_eq!(events.len(), 2, "{events:?}");
        assert!(
            matches!(&events[0], ConversationEvent::ToolCallDelta { tool_use_id, name, index: 0, .. } if tool_use_id == "call_a" && name == "tool_a")
        );
        assert!(
            matches!(&events[1], ConversationEvent::ToolCallDelta { tool_use_id, name, index: 1, .. } if tool_use_id == "call_b" && name == "tool_b")
        );
    }

    // Defeat: parse_image_response drops the url field or maps it to the wrong
    // ImageOutput field, so callers get an ImageResponse with url=None even when
    // the upstream returned a valid URL.
    #[test]
    fn parse_image_response_url() {
        let body = br#"{"created":1234567890,"data":[{"url":"https://example.com/img.png","revised_prompt":"a fluffy cat"}]}"#;
        let resp = parse_image_response(body, &rid()).unwrap();
        assert_eq!(resp.created, 1234567890);
        assert_eq!(resp.data.len(), 1);
        assert_eq!(
            resp.data[0].url.as_deref(),
            Some("https://example.com/img.png")
        );
        assert_eq!(resp.data[0].revised_prompt.as_deref(), Some("a fluffy cat"));
    }
}
