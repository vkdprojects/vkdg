//! `OpenAI` Chat Completions SSE stream to canonical events.
//!
//! | upstream | event |
//! |---|---|
//! | first chunk | `Started` |
//! | `delta.content` / `delta.reasoning_content` or `delta.reasoning` | `OutputDelta` / `ReasoningDelta` (index 0) |
//! | `delta.tool_calls[]` | `ToolCallDelta` (id and name once per call), `ToolCallEnd` when the next call or the finish arrives |
//! | `usage` (usually a trailing chunk with empty `choices`) | `Usage`, with cached tokens split out of `prompt_tokens` |
//! | `finish_reason`, then `[DONE]` or EOF | `Completed` |
//! | `{"error": ...}` | `Failed` with the status the error type implies |
//!
//! `Completed` waits for `[DONE]` or EOF because the usage chunk arrives after the
//! `finish_reason` chunk. Only choice 0 is decoded (`n > 1` is rejected at ingress).
//! A stream that ends without a `finish_reason` is a failure, never a clean `Completed`.

use bytes::Bytes;
use serde_json::Value;
use vkdg_core::RequestId;
use vkdg_operations::{parse_openai_finish_reason, ConversationEvent, StopReason};

use super::sse_frame::{FrameError, SseFrame, SseFramer, DEFAULT_MAX_EVENT_BYTES};
use super::token_counts::TokenCounts;
use super::upstream_failure::{
    capped, failed, DEFAULT_MAX_TOOL_ARGUMENT_BYTES, EMPTY_STREAM, EVENT_TOO_LARGE,
    MALFORMED_OPENAI, TOOL_ARGUMENTS_TOO_LARGE, TRUNCATED_STREAM,
};
use crate::ConversationStreamDecoder;

/// Decodes one `OpenAI` Chat Completions SSE response. One instance per stream.
#[derive(Debug)]
pub struct OpenAiSseDecoder {
    framer: SseFramer,
    state: StreamState,
}

#[derive(Debug)]
struct ToolCall {
    index: u32,
    argument_bytes: usize,
    open: bool,
}

#[derive(Debug)]
struct StreamState {
    max_tool_argument_bytes: usize,
    started: bool,
    /// `Completed` or `Failed` was emitted; nothing follows it.
    done: bool,
    /// A chunk with content, usage or a finish reason arrived.
    saw_event: bool,
    stop: Option<StopReason>,
    counts: TokenCounts,
    tools: Vec<ToolCall>,
}

impl OpenAiSseDecoder {
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
                tools: Vec::new(),
            },
        }
    }
}

impl Default for OpenAiSseDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl ConversationStreamDecoder for OpenAiSseDecoder {
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

/// The status an `OpenAI` error type implies.
pub(super) fn error_status(kind: &str) -> u16 {
    match kind {
        "invalid_request_error" => 400,
        "authentication_error" => 401,
        "permission_error" => 403,
        "not_found_error" => 404,
        "rate_limit_error" | "insufficient_quota" => 429,
        "server_error" => 500,
        _ => 502,
    }
}

fn index_of(value: &Value, fallback: u32) -> u32 {
    value
        .get("index")
        .and_then(Value::as_u64)
        .map_or(fallback, |n| u32::try_from(n).unwrap_or(u32::MAX))
}

fn non_empty_str<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

impl StreamState {
    fn fail(&mut self, out: &mut Vec<ConversationEvent>, code: u16, message: &str) {
        out.push(failed(code, message));
        self.done = true;
    }

    fn begin(&mut self, out: &mut Vec<ConversationEvent>) {
        self.saw_event = true;
        if !self.started {
            self.started = true;
            out.push(ConversationEvent::Started {
                request_id: RequestId::new(),
            });
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
        if frame.data.trim() == "[DONE]" {
            self.complete(out);
            return;
        }
        let Ok(chunk) = serde_json::from_str::<Value>(&frame.data) else {
            if !at_eof {
                self.fail(out, 502, MALFORMED_OPENAI);
            }
            return;
        };
        if let Some(error) = chunk.get("error").filter(|e| e.is_object()) {
            let kind = error
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let message = capped(
                error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            );
            self.fail(out, error_status(kind), &message);
            return;
        }
        self.on_chunk(&chunk, out);
    }

    fn on_chunk(&mut self, chunk: &Value, out: &mut Vec<ConversationEvent>) {
        let first_choice = chunk
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.iter().find(|c| index_of(c, 0) == 0));
        if first_choice.is_some() {
            self.begin(out);
        }
        if let Some(choice) = first_choice {
            if let Some(delta) = choice.get("delta") {
                self.on_delta(delta, out);
            }
            if self.done {
                return;
            }
            if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
                self.end_open_tools(out);
                self.stop = Some(parse_openai_finish_reason(reason));
            }
        }
        if let Some(usage) = chunk.get("usage").filter(|u| u.is_object()) {
            self.begin(out);
            if self.counts.merge_openai(usage) {
                out.push(self.counts.event());
            }
        }
    }

    fn on_delta(&mut self, delta: &Value, out: &mut Vec<ConversationEvent>) {
        if let Some(text) =
            non_empty_str(delta, "reasoning_content").or_else(|| non_empty_str(delta, "reasoning"))
        {
            out.push(ConversationEvent::ReasoningDelta {
                delta: text.to_owned(),
                index: 0,
            });
        }
        if let Some(text) = non_empty_str(delta, "content") {
            out.push(ConversationEvent::OutputDelta {
                delta: text.to_owned(),
                index: 0,
            });
        }
        if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
            for (position, call) in calls.iter().enumerate() {
                let fallback = u32::try_from(position).unwrap_or(u32::MAX);
                self.on_tool_call(call, fallback, out);
                if self.done {
                    return;
                }
            }
        }
    }

    fn on_tool_call(
        &mut self,
        call: &Value,
        fallback_index: u32,
        out: &mut Vec<ConversationEvent>,
    ) {
        let index = index_of(call, fallback_index);
        let function = call.get("function").unwrap_or(&Value::Null);
        let arguments = function
            .get("arguments")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let (position, opening) =
            if let Some(position) = self.tools.iter().position(|t| t.index == index) {
                (position, false)
            } else {
                // A new call starts: the previous ones are complete.
                self.end_open_tools(out);
                self.tools.push(ToolCall {
                    index,
                    argument_bytes: 0,
                    open: true,
                });
                (self.tools.len() - 1, true)
            };
        let tool = &mut self.tools[position];
        tool.argument_bytes = tool.argument_bytes.saturating_add(arguments.len());
        if tool.argument_bytes > self.max_tool_argument_bytes {
            self.fail(out, 502, TOOL_ARGUMENTS_TOO_LARGE);
            return;
        }
        if opening {
            out.push(ConversationEvent::ToolCallDelta {
                tool_use_id: call
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                name: function
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                input_delta: arguments.to_owned(),
                index,
            });
        } else if !arguments.is_empty() {
            // Some providers repeat id and name on every fragment; only the first counts.
            out.push(ConversationEvent::ToolCallDelta {
                tool_use_id: String::new(),
                name: String::new(),
                input_delta: arguments.to_owned(),
                index,
            });
        }
    }

    fn end_open_tools(&mut self, out: &mut Vec<ConversationEvent>) {
        for tool in self.tools.iter_mut().filter(|t| t.open) {
            tool.open = false;
            out.push(ConversationEvent::ToolCallEnd { index: tool.index });
        }
    }

    /// `[DONE]` or the end of a stream that recorded a finish reason.
    fn complete(&mut self, out: &mut Vec<ConversationEvent>) {
        if !self.saw_event {
            self.fail(out, 502, EMPTY_STREAM);
            return;
        }
        self.end_open_tools(out);
        let reason = self.stop.take().unwrap_or(if self.tools.is_empty() {
            StopReason::EndTurn
        } else {
            StopReason::ToolUse
        });
        out.push(ConversationEvent::Completed {
            stop_reason: reason,
        });
        self.done = true;
    }

    fn end_of_stream(&mut self, out: &mut Vec<ConversationEvent>) {
        if self.done {
            return;
        }
        if !self.saw_event {
            self.fail(out, 502, EMPTY_STREAM);
        } else if self.stop.is_some() {
            // Several compatible providers omit `[DONE]`; the stream was complete.
            self.complete(out);
        } else {
            self.fail(out, 502, TRUNCATED_STREAM);
        }
    }
}
