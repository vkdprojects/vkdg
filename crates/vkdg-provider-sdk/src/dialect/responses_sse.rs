//! `OpenAI` Responses SSE to canonical conversation events.
//!
//! Text/reasoning deltas retain their output indices; function calls use `call_id`
//! (not the output item's `id`) and end independently. Text/item/arguments `done`
//! events are not response terminals: only completed/done/incomplete can complete.
//! Errors and EOF before a response terminal fail, including after text.done.

use std::collections::BTreeMap;

use bytes::Bytes;
use serde_json::Value;
use vkdg_core::RequestId;
use vkdg_operations::{ConversationEvent, StopReason};

use super::openai_sse::error_status;
use super::sse_frame::{FrameError, SseFrame, SseFramer, DEFAULT_MAX_EVENT_BYTES};
use super::token_counts::TokenCounts;
use super::upstream_failure::{
    capped, failed, DEFAULT_MAX_TOOL_ARGUMENT_BYTES, EMPTY_STREAM, EVENT_TOO_LARGE,
    TOOL_ARGUMENTS_TOO_LARGE, TRUNCATED_STREAM,
};
use crate::ConversationStreamDecoder;

const MALFORMED: &str = "upstream sent a malformed responses stream event";
// Tracking only counters still needs a bound for a stream with arbitrarily many calls.
const MAX_TOOL_CALLS: usize = 4096;
const TOO_MANY_TOOLS: &str = "upstream tool call count exceeded the gateway limit";

/// Decodes one Responses stream, with bounded frame and tool metadata memory.
///
/// Unknown extension events are ignored. Function argument done snapshots are
/// cumulative: only their not-yet-streamed suffix is emitted. Argument text is not
/// retained. At most 4096 tool indices are tracked, including finished calls.
#[derive(Debug)]
pub struct ResponsesSseDecoder {
    framer: SseFramer,
    state: StreamState,
}

#[derive(Debug)]
struct ToolCall {
    argument_bytes: usize,
    ended: bool,
}

#[derive(Debug)]
struct StreamState {
    started: bool,
    done: bool,
    max_tool_argument_bytes: usize,
    tools: BTreeMap<u32, ToolCall>,
    counts: TokenCounts,
}

impl ResponsesSseDecoder {
    pub fn new() -> Self {
        Self::with_limits(DEFAULT_MAX_EVENT_BYTES, DEFAULT_MAX_TOOL_ARGUMENT_BYTES)
    }

    /// Bounds one SSE event and each tool call's cumulative argument bytes.
    /// Exceeding either limit emits a terminal upstream failure, never completion.
    pub fn with_limits(max_event_bytes: usize, max_tool_argument_bytes: usize) -> Self {
        Self {
            framer: SseFramer::with_max_event_bytes(max_event_bytes),
            state: StreamState {
                started: false,
                done: false,
                max_tool_argument_bytes,
                tools: BTreeMap::new(),
                counts: TokenCounts::default(),
            },
        }
    }
}

impl Default for ResponsesSseDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl ConversationStreamDecoder for ResponsesSseDecoder {
    fn feed(&mut self, chunk: Bytes) -> Vec<ConversationEvent> {
        let mut out = Vec::new();
        if !self.state.done {
            let Self { framer, state } = self;
            framer.push(&chunk, |frame| state.on_frame(frame, &mut out, false));
        }
        out
    }

    fn finish(&mut self) -> Vec<ConversationEvent> {
        let mut out = Vec::new();
        if !self.state.done {
            let Self { framer, state } = self;
            framer.finish(|frame| state.on_frame(frame, &mut out, true));
            if !state.done {
                state.fail(
                    &mut out,
                    502,
                    if state.started {
                        TRUNCATED_STREAM
                    } else {
                        EMPTY_STREAM
                    },
                );
            }
        }
        out
    }
}

fn output_index(value: &Value) -> Option<u32> {
    value.get("output_index")?.as_u64()?.try_into().ok()
}

impl StreamState {
    fn fail(&mut self, out: &mut Vec<ConversationEvent>, code: u16, message: &str) {
        out.push(failed(code, message));
        self.done = true;
    }

    fn begin(&mut self, out: &mut Vec<ConversationEvent>) {
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
        // This sentinel is not evidence that a Responses request succeeded.
        if frame.data.trim() == "[DONE]" {
            self.fail(out, 502, TRUNCATED_STREAM);
            return;
        }
        let Ok(value) = serde_json::from_str::<Value>(&frame.data) else {
            self.fail(out, 502, if at_eof { TRUNCATED_STREAM } else { MALFORMED });
            return;
        };
        let Some(kind) = value.get("type").and_then(Value::as_str).or(frame.event) else {
            self.fail(out, 502, MALFORMED);
            return;
        };
        match kind {
            "response.created" | "response.in_progress" => self.begin(out),
            "response.output_text.delta"
            | "response.reasoning_summary_text.delta"
            | "response.reasoning_text.delta" => {
                let (Some(index), Some(delta)) = (
                    output_index(&value),
                    value.get("delta").and_then(Value::as_str),
                ) else {
                    self.fail(out, 502, MALFORMED);
                    return;
                };
                self.begin(out);
                if !delta.is_empty() {
                    out.push(if kind == "response.output_text.delta" {
                        ConversationEvent::OutputDelta {
                            delta: delta.to_owned(),
                            index,
                        }
                    } else {
                        ConversationEvent::ReasoningDelta {
                            delta: delta.to_owned(),
                            index,
                        }
                    });
                }
            }
            "response.output_item.added" | "response.output_item.done" => {
                let Some(item) = value.get("item").filter(|item| item.is_object()) else {
                    self.fail(out, 502, MALFORMED);
                    return;
                };
                self.begin(out);
                if item.get("type").and_then(Value::as_str) == Some("function_call") {
                    let Some(index) = output_index(&value) else {
                        self.fail(out, 502, MALFORMED);
                        return;
                    };
                    self.add_tool(index, item, out);
                    if !self.done && kind == "response.output_item.done" {
                        self.arguments_done(index, item, out);
                    }
                }
            }
            "response.function_call_arguments.delta" | "response.function_call_arguments.done" => {
                let Some(index) = output_index(&value) else {
                    self.fail(out, 502, MALFORMED);
                    return;
                };
                if kind == "response.function_call_arguments.done" {
                    self.arguments_done(index, &value, out);
                } else if let Some(delta) = value.get("delta").and_then(Value::as_str) {
                    self.arguments_delta(index, delta, out);
                } else {
                    self.fail(out, 502, MALFORMED);
                }
            }
            "response.failed" => self.provider_error(
                value
                    .get("response")
                    .and_then(|r| r.get("error"))
                    .unwrap_or(&Value::Null),
                out,
            ),
            "error" => self.provider_error(value.get("error").unwrap_or(&value), out),
            "response.completed" | "response.done" | "response.incomplete" => {
                self.terminal(kind, &value, out);
            }
            // done snapshots for text/reasoning and structural events carry no new
            // content and, importantly, are not successful response terminals.
            _ => {}
        }
    }

    fn provider_error(&mut self, error: &Value, out: &mut Vec<ConversationEvent>) {
        let kind = error
            .get("type")
            .and_then(Value::as_str)
            .filter(|kind| *kind != "error")
            .or_else(|| error.get("code").and_then(Value::as_str))
            .unwrap_or_default();
        let code = match kind {
            "rate_limit_exceeded" => 429,
            "invalid_prompt" => 400,
            _ => error_status(kind),
        };
        let message = capped(
            error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("upstream responses request failed"),
        );
        self.fail(out, code, &message);
    }

    fn add_tool(&mut self, index: u32, item: &Value, out: &mut Vec<ConversationEvent>) {
        if self.tools.contains_key(&index) {
            return;
        }
        if self.tools.len() >= MAX_TOOL_CALLS {
            self.fail(out, 502, TOO_MANY_TOOLS);
            return;
        }
        let (Some(id), Some(name)) = (
            item.get("call_id")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty()),
            item.get("name")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty()),
        ) else {
            self.fail(out, 502, MALFORMED);
            return;
        };
        let arguments = item
            .get("arguments")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if arguments.len() > self.max_tool_argument_bytes {
            self.fail(out, 502, TOOL_ARGUMENTS_TOO_LARGE);
            return;
        }
        self.tools.insert(
            index,
            ToolCall {
                argument_bytes: arguments.len(),
                ended: false,
            },
        );
        out.push(ConversationEvent::ToolCallDelta {
            tool_use_id: id.to_owned(),
            name: name.to_owned(),
            input_delta: arguments.to_owned(),
            index,
        });
    }

    fn arguments_delta(&mut self, index: u32, delta: &str, out: &mut Vec<ConversationEvent>) {
        let Some(tool) = self.tools.get_mut(&index).filter(|tool| !tool.ended) else {
            self.fail(out, 502, MALFORMED);
            return;
        };
        let bytes = tool.argument_bytes.saturating_add(delta.len());
        if bytes > self.max_tool_argument_bytes {
            self.fail(out, 502, TOOL_ARGUMENTS_TOO_LARGE);
            return;
        }
        tool.argument_bytes = bytes;
        if !delta.is_empty() {
            out.push(ConversationEvent::ToolCallDelta {
                tool_use_id: String::new(),
                name: String::new(),
                input_delta: delta.to_owned(),
                index,
            });
        }
    }

    fn arguments_done(&mut self, index: u32, value: &Value, out: &mut Vec<ConversationEvent>) {
        let (Some(tool), Some(arguments)) = (
            self.tools.get(&index),
            value.get("arguments").and_then(Value::as_str),
        ) else {
            self.fail(out, 502, MALFORMED);
            return;
        };
        if arguments.len() > self.max_tool_argument_bytes {
            self.fail(out, 502, TOOL_ARGUMENTS_TOO_LARGE);
            return;
        }
        // No cumulative argument buffers: the upstream done snapshot repeats the
        // prefix already emitted. Validate length and UTF-8 suffix boundaries.
        let Some(suffix) = arguments.get(tool.argument_bytes..) else {
            self.fail(out, 502, MALFORMED);
            return;
        };
        if tool.ended {
            if !suffix.is_empty() {
                self.fail(out, 502, MALFORMED);
            }
            return;
        }
        self.arguments_delta(index, suffix, out);
        if !self.done {
            self.end_tool(index, out);
        }
    }

    fn end_tool(&mut self, index: u32, out: &mut Vec<ConversationEvent>) {
        if let Some(tool) = self.tools.get_mut(&index).filter(|tool| !tool.ended) {
            tool.ended = true;
            out.push(ConversationEvent::ToolCallEnd { index });
        }
    }

    fn terminal(&mut self, kind: &str, value: &Value, out: &mut Vec<ConversationEvent>) {
        let Some(response) = value.get("response").filter(|r| r.is_object()) else {
            self.fail(out, 502, MALFORMED);
            return;
        };
        let status = response.get("status").and_then(Value::as_str);
        if status == Some("failed") || response.get("error").is_some_and(|e| !e.is_null()) {
            self.provider_error(response.get("error").unwrap_or(&Value::Null), out);
            return;
        }
        let incomplete = kind == "response.incomplete" || status == Some("incomplete");
        let stop_reason = if incomplete {
            if response
                .get("incomplete_details")
                .and_then(|d| d.get("reason"))
                .and_then(Value::as_str)
                != Some("max_output_tokens")
            {
                self.fail(out, 502, "upstream responses request ended incomplete");
                return;
            }
            StopReason::MaxTokens
        } else if status.is_some_and(|s| s != "completed") {
            self.fail(out, 502, MALFORMED);
            return;
        } else if self.tools.is_empty() {
            StopReason::EndTurn
        } else {
            StopReason::ToolUse
        };
        self.begin(out);
        for (&index, tool) in &mut self.tools {
            if !tool.ended {
                tool.ended = true;
                out.push(ConversationEvent::ToolCallEnd { index });
            }
        }
        if let Some(usage) = response.get("usage") {
            if self.counts.merge_responses(usage) {
                out.push(self.counts.event());
            }
        }
        out.push(ConversationEvent::Completed { stop_reason });
        self.done = true;
    }
}
