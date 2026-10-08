//! Anthropic Messages SSE stream to canonical events.
//!
//! Mapping and the losses it accepts (`docs/sdk/dialect-translation.md`):
//!
//! | upstream | event |
//! |---|---|
//! | `message_start` | `Started`, then `Usage` from `message.usage` |
//! | text / `thinking` deltas | `OutputDelta` / `ReasoningDelta`, source block index kept |
//! | `tool_use` start, `input_json_delta` | `ToolCallDelta` (id and name once), `ToolCallEnd` at block stop |
//! | `message_delta` | `Usage` (only the fields it reports), stop reason remembered |
//! | `message_stop` or EOF after a stop reason | `Completed` |
//! | `error` | `Failed` with the status the error type implies |
//!
//! Dropped: `signature_delta` and `redacted_thinking` (no client dialect can use them
//! without the provider's own session), server-side tool blocks and their results,
//! citations, unknown events. A stream that ends without a stop reason is a failure,
//! never a clean `Completed`.

use std::collections::HashMap;

use bytes::Bytes;
use serde_json::Value;
use vkdg_core::RequestId;
use vkdg_operations::{parse_anthropic_stop_reason, ConversationEvent, StopReason};

use super::sse_frame::{FrameError, SseFrame, SseFramer, DEFAULT_MAX_EVENT_BYTES};
use super::token_counts::TokenCounts;
use super::upstream_failure::{
    capped, failed, DEFAULT_MAX_TOOL_ARGUMENT_BYTES, EMPTY_STREAM, EVENT_TOO_LARGE,
    MALFORMED_ANTHROPIC, TOOL_ARGUMENTS_TOO_LARGE, TRUNCATED_STREAM,
};
use crate::ConversationStreamDecoder;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockKind {
    Text,
    Thinking,
    Tool,
    /// Redacted thinking, server-side tools and anything unknown: dropped whole.
    Dropped,
}

/// Decodes one Anthropic Messages SSE response. One instance per stream.
#[derive(Debug)]
pub struct AnthropicSseDecoder {
    framer: SseFramer,
    state: StreamState,
}

#[derive(Debug)]
#[allow(clippy::struct_excessive_bools)] // independent one-way stream flags
struct StreamState {
    max_tool_argument_bytes: usize,
    started: bool,
    /// `Completed` or `Failed` was emitted; nothing follows it.
    done: bool,
    /// Something other than a ping arrived.
    saw_event: bool,
    stop: Option<StopReason>,
    counts: TokenCounts,
    blocks: HashMap<u32, BlockKind>,
    tool_argument_bytes: usize,
    tool_called: bool,
}

impl AnthropicSseDecoder {
    pub fn new() -> Self {
        Self::with_limits(DEFAULT_MAX_EVENT_BYTES, DEFAULT_MAX_TOOL_ARGUMENT_BYTES)
    }

    /// Bounds one SSE event and one tool call's streamed arguments; either limit
    /// exceeded fails the stream instead of growing memory.
    pub fn with_limits(max_event_bytes: usize, max_tool_argument_bytes: usize) -> Self {
        Self {
            framer: SseFramer::with_max_event_bytes(max_event_bytes),
            state: StreamState {
                max_tool_argument_bytes,
                started: false,
                done: false,
                saw_event: false,
                stop: None,
                counts: TokenCounts::default(),
                blocks: HashMap::new(),
                tool_argument_bytes: 0,
                tool_called: false,
            },
        }
    }
}

impl Default for AnthropicSseDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl ConversationStreamDecoder for AnthropicSseDecoder {
    fn feed(&mut self, chunk: Bytes) -> Vec<ConversationEvent> {
        let mut out = Vec::new();
        if self.state.done {
            return out;
        }
        let Self { framer, state } = self;
        framer.push(&chunk, |frame| state.on_frame(frame, &mut out, false));
        out
    }

    fn finish(&mut self) -> Vec<ConversationEvent> {
        let mut out = Vec::new();
        if self.state.done {
            return out;
        }
        let Self { framer, state } = self;
        // A cut inside the last event reads as truncation, not as a malformed event.
        framer.finish(|frame| state.on_frame(frame, &mut out, true));
        state.end_of_stream(&mut out);
        out
    }
}

fn index_of(event: &Value) -> u32 {
    event
        .get("index")
        .and_then(Value::as_u64)
        .map_or(0, |n| u32::try_from(n).unwrap_or(u32::MAX))
}

fn str_of<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// The status an Anthropic error type implies.
pub(super) fn error_status(kind: &str) -> u16 {
    match kind {
        "invalid_request_error" => 400,
        "authentication_error" => 401,
        "billing_error" => 402,
        "permission_error" => 403,
        "not_found_error" => 404,
        "request_too_large" => 413,
        "rate_limit_error" => 429,
        "api_error" => 500,
        "timeout_error" => 504,
        "overloaded_error" => 529,
        _ => 502,
    }
}

impl StreamState {
    fn fail(&mut self, out: &mut Vec<ConversationEvent>, code: u16, message: &str) {
        out.push(failed(code, message));
        self.done = true;
    }

    fn ensure_started(&mut self, out: &mut Vec<ConversationEvent>) {
        if !self.started {
            self.started = true;
            out.push(ConversationEvent::Started {
                request_id: RequestId::new(),
            });
        }
    }

    fn merge_usage(&mut self, usage: Option<&Value>, out: &mut Vec<ConversationEvent>) {
        if usage.is_some_and(|u| self.counts.merge_anthropic(u)) {
            out.push(self.counts.event());
        }
    }

    fn on_frame(
        &mut self,
        frame: Result<SseFrame<'_>, FrameError>,
        out: &mut Vec<ConversationEvent>,
        at_eof: bool,
    ) {
        if self.done {
            return;
        }
        let Ok(frame) = frame else {
            self.fail(out, 502, EVENT_TOO_LARGE);
            return;
        };
        // Some Anthropic-compatible proxies close with the OpenAI sentinel.
        if frame.data.trim() == "[DONE]" {
            return;
        }
        let Ok(event) = serde_json::from_str::<Value>(&frame.data) else {
            if !at_eof {
                self.fail(out, 502, MALFORMED_ANTHROPIC);
            }
            return;
        };
        let kind = event
            .get("type")
            .and_then(Value::as_str)
            .or(frame.event)
            .unwrap_or_default();
        match kind {
            "ping" => {}
            "error" => {
                let error = event.get("error").unwrap_or(&event);
                let message = capped(str_of(error, "message"));
                self.fail(out, error_status(str_of(error, "type")), &message);
            }
            "message_start" => {
                self.begin(out);
                self.merge_usage(event.get("message").and_then(|m| m.get("usage")), out);
            }
            "content_block_start" => {
                self.begin(out);
                self.block_start(&event, out);
            }
            "content_block_delta" => {
                self.begin(out);
                self.block_delta(&event, out);
            }
            "content_block_stop" => {
                self.begin(out);
                let index = index_of(&event);
                if self.blocks.remove(&index) == Some(BlockKind::Tool) {
                    out.push(ConversationEvent::ToolCallEnd { index });
                }
            }
            "message_delta" => {
                self.begin(out);
                if let Some(reason) = event
                    .get("delta")
                    .and_then(|d| d.get("stop_reason"))
                    .and_then(Value::as_str)
                {
                    self.stop = Some(parse_anthropic_stop_reason(reason));
                }
                self.merge_usage(event.get("usage"), out);
            }
            "message_stop" => {
                self.begin(out);
                let reason = self.stop.take().unwrap_or(if self.tool_called {
                    StopReason::ToolUse
                } else {
                    StopReason::EndTurn
                });
                out.push(ConversationEvent::Completed {
                    stop_reason: reason,
                });
                self.done = true;
            }
            // Unknown event types are ignored: new Anthropic events must not break streams.
            _ => {}
        }
    }

    /// A real event arrived: the response has started.
    fn begin(&mut self, out: &mut Vec<ConversationEvent>) {
        self.saw_event = true;
        self.ensure_started(out);
    }

    fn block_start(&mut self, event: &Value, out: &mut Vec<ConversationEvent>) {
        let index = index_of(event);
        let block = event.get("content_block").unwrap_or(&Value::Null);
        match str_of(block, "type") {
            "text" => {
                self.blocks.insert(index, BlockKind::Text);
                let text = str_of(block, "text");
                if !text.is_empty() {
                    out.push(ConversationEvent::OutputDelta {
                        delta: text.to_owned(),
                        index,
                    });
                }
            }
            "thinking" => {
                self.blocks.insert(index, BlockKind::Thinking);
                let text = str_of(block, "thinking");
                if !text.is_empty() {
                    out.push(ConversationEvent::ReasoningDelta {
                        delta: text.to_owned(),
                        index,
                    });
                }
            }
            "tool_use" => {
                self.blocks.insert(index, BlockKind::Tool);
                self.tool_called = true;
                self.tool_argument_bytes = 0;
                out.push(ConversationEvent::ToolCallDelta {
                    tool_use_id: str_of(block, "id").to_owned(),
                    name: str_of(block, "name").to_owned(),
                    input_delta: String::new(),
                    index,
                });
            }
            _ => {
                self.blocks.insert(index, BlockKind::Dropped);
            }
        }
    }

    fn block_delta(&mut self, event: &Value, out: &mut Vec<ConversationEvent>) {
        let index = index_of(event);
        let delta = event.get("delta").unwrap_or(&Value::Null);
        let known = self.blocks.get(&index).copied();
        if known == Some(BlockKind::Dropped) {
            return;
        }
        match str_of(delta, "type") {
            "text_delta" => {
                let text = str_of(delta, "text");
                if !text.is_empty() {
                    out.push(ConversationEvent::OutputDelta {
                        delta: text.to_owned(),
                        index,
                    });
                }
            }
            "thinking_delta" => {
                let text = str_of(delta, "thinking");
                if !text.is_empty() {
                    out.push(ConversationEvent::ReasoningDelta {
                        delta: text.to_owned(),
                        index,
                    });
                }
            }
            "input_json_delta" if known == Some(BlockKind::Tool) => {
                let fragment = str_of(delta, "partial_json");
                if fragment.is_empty() {
                    return;
                }
                self.tool_argument_bytes = self.tool_argument_bytes.saturating_add(fragment.len());
                if self.tool_argument_bytes > self.max_tool_argument_bytes {
                    self.fail(out, 502, TOOL_ARGUMENTS_TOO_LARGE);
                    return;
                }
                out.push(ConversationEvent::ToolCallDelta {
                    tool_use_id: String::new(),
                    name: String::new(),
                    input_delta: fragment.to_owned(),
                    index,
                });
            }
            // `signature_delta`, `citations_delta` and future delta types.
            _ => {}
        }
    }

    fn end_of_stream(&mut self, out: &mut Vec<ConversationEvent>) {
        if self.done {
            return;
        }
        if !self.saw_event {
            self.fail(out, 502, EMPTY_STREAM);
        } else if let Some(reason) = self.stop.take() {
            // A gateway dropped the final `message_stop`; the stream was complete.
            out.push(ConversationEvent::Completed {
                stop_reason: reason,
            });
            self.done = true;
        } else {
            self.fail(out, 502, TRUNCATED_STREAM);
        }
    }
}
