//! Client-dialect stream encoders.
//!
//! One [`StreamEncoder`] instance per response stream turns
//! [`ConversationEvent`]s into wire bytes for the dialect the client spoke.
//! This is the only place a dialect is written, so every ingress and every
//! provider that needs protocol translation emits the same bytes.

use serde_json::{json, Value};

use crate::{ConversationEvent, StopReason, StreamContext, StreamEncoder, UsageCount};

/// Content block kinds an Anthropic stream can open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockKind {
    Text,
    Thinking,
    ToolUse,
}

/// The block currently open in an Anthropic stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OpenBlock {
    kind: BlockKind,
    /// Index carried by the source events, which is not the wire index.
    source_index: u32,
    /// Index sent to the client; blocks are numbered in the order they open.
    wire_index: u32,
}

fn sse(event: &str, data: &Value) -> Vec<u8> {
    format!("event: {event}\ndata: {data}\n\n").into_bytes()
}

fn stop_reason_anthropic(reason: &StopReason) -> &'static str {
    match reason {
        StopReason::EndTurn | StopReason::Cancelled => "end_turn",
        StopReason::MaxTokens => "max_tokens",
        StopReason::ToolUse => "tool_use",
        StopReason::StopSequence => "stop_sequence",
    }
}

fn stop_reason_openai(reason: &StopReason) -> &'static str {
    match reason {
        StopReason::EndTurn | StopReason::Cancelled | StopReason::StopSequence => "stop",
        StopReason::MaxTokens => "length",
        StopReason::ToolUse => "tool_calls",
    }
}

/// Token counts gathered from [`ConversationEvent::Usage`].
#[derive(Debug, Default, Clone, Copy)]
struct Usage {
    input: u32,
    output: u32,
    cache_read: u32,
    cache_creation: u32,
}

impl Usage {
    fn update(
        &mut self,
        input: &UsageCount,
        output: &UsageCount,
        cache_read: &UsageCount,
        cache_creation: &UsageCount,
    ) {
        self.input = input.value();
        self.output = output.value();
        self.cache_read = cache_read.value();
        self.cache_creation = cache_creation.value();
    }

    fn to_anthropic(self) -> Value {
        let mut usage = json!({ "input_tokens": self.input, "output_tokens": self.output });
        // Anthropic omits the cache fields entirely when a provider reports none.
        if self.cache_read > 0 {
            usage["cache_read_input_tokens"] = json!(self.cache_read);
        }
        if self.cache_creation > 0 {
            usage["cache_creation_input_tokens"] = json!(self.cache_creation);
        }
        usage
    }
}

// ── Anthropic Messages ────────────────────────────────────────────────────────

/// Encodes an Anthropic Messages SSE stream.
///
/// Guarantees a well-formed stream: `message_start` carries a real message
/// object, every delta sits inside a `content_block_start`/`content_block_stop`
/// pair with sequential wire indices, and the stream always terminates with
/// `message_delta` + `message_stop` — even when the provider sends no stop event.
#[derive(Debug)]
pub struct AnthropicStreamEncoder {
    message_id: String,
    model: String,
    started: bool,
    open: Option<OpenBlock>,
    next_wire_index: u32,
    usage: Usage,
    stop_reason: StopReason,
    terminated: bool,
}

impl AnthropicStreamEncoder {
    pub fn new(ctx: &StreamContext<'_>) -> Self {
        Self {
            message_id: format!("msg_{}", ctx.request_id.0),
            model: ctx.model.to_owned(),
            started: false,
            open: None,
            next_wire_index: 0,
            usage: Usage::default(),
            stop_reason: StopReason::EndTurn,
            terminated: false,
        }
    }

    fn start(&mut self, out: &mut Vec<u8>) {
        if self.started {
            return;
        }
        self.started = true;
        out.extend(sse(
            "message_start",
            &json!({
                "type": "message_start",
                "message": {
                    "id": self.message_id,
                    "type": "message",
                    "role": "assistant",
                    "model": self.model,
                    "content": [],
                    "stop_reason": Value::Null,
                    "stop_sequence": Value::Null,
                    "usage": self.usage.to_anthropic(),
                },
            }),
        ));
    }

    fn close_block(&mut self, out: &mut Vec<u8>) {
        if let Some(block) = self.open.take() {
            out.extend(sse(
                "content_block_stop",
                &json!({ "type": "content_block_stop", "index": block.wire_index }),
            ));
        }
    }

    /// Opens a block unless the wanted one is already open, closing any other first.
    fn open_block(
        &mut self,
        kind: BlockKind,
        source_index: u32,
        content_block: Value,
        out: &mut Vec<u8>,
    ) -> u32 {
        if let Some(block) = self.open {
            if block.kind == kind && block.source_index == source_index {
                return block.wire_index;
            }
        }
        self.close_block(out);
        let wire_index = self.next_wire_index;
        self.next_wire_index += 1;
        self.open = Some(OpenBlock {
            kind,
            source_index,
            wire_index,
        });
        out.extend(sse(
            "content_block_start",
            &json!({
                "type": "content_block_start",
                "index": wire_index,
                "content_block": content_block,
            }),
        ));
        wire_index
    }
}

impl StreamEncoder for AnthropicStreamEncoder {
    fn encode(&mut self, event: &ConversationEvent) -> Vec<u8> {
        let mut out = Vec::new();
        if self.terminated {
            return out;
        }
        match event {
            ConversationEvent::Started { .. } => self.start(&mut out),
            ConversationEvent::OutputDelta { delta, index } => {
                self.start(&mut out);
                let wire_index = self.open_block(
                    BlockKind::Text,
                    *index,
                    json!({ "type": "text", "text": "" }),
                    &mut out,
                );
                out.extend(sse(
                    "content_block_delta",
                    &json!({
                        "type": "content_block_delta",
                        "index": wire_index,
                        "delta": { "type": "text_delta", "text": delta },
                    }),
                ));
            }
            ConversationEvent::ReasoningDelta { delta, index } => {
                self.start(&mut out);
                let wire_index = self.open_block(
                    BlockKind::Thinking,
                    *index,
                    json!({ "type": "thinking", "thinking": "" }),
                    &mut out,
                );
                out.extend(sse(
                    "content_block_delta",
                    &json!({
                        "type": "content_block_delta",
                        "index": wire_index,
                        "delta": { "type": "thinking_delta", "thinking": delta },
                    }),
                ));
            }
            ConversationEvent::ToolCallDelta {
                tool_use_id,
                name,
                input_delta,
                index,
            } => {
                self.start(&mut out);
                let wire_index = self.open_block(
                    BlockKind::ToolUse,
                    *index,
                    json!({ "type": "tool_use", "id": tool_use_id, "name": name, "input": {} }),
                    &mut out,
                );
                // The opening fragment carries the name and no input yet.
                if !input_delta.is_empty() {
                    out.extend(sse(
                        "content_block_delta",
                        &json!({
                            "type": "content_block_delta",
                            "index": wire_index,
                            "delta": { "type": "input_json_delta", "partial_json": input_delta },
                        }),
                    ));
                }
            }
            ConversationEvent::ToolCallEnd { index } => {
                if self
                    .open
                    .is_some_and(|b| b.kind == BlockKind::ToolUse && b.source_index == *index)
                {
                    self.close_block(&mut out);
                }
            }
            ConversationEvent::Usage {
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_creation_tokens,
            } => {
                // Usage arriving before the first delta belongs in message_start.
                self.usage.update(
                    input_tokens,
                    output_tokens,
                    cache_read_tokens,
                    cache_creation_tokens,
                );
            }
            ConversationEvent::Completed { stop_reason } => {
                self.stop_reason = stop_reason.clone();
                out.extend(self.finish());
            }
            ConversationEvent::Failed { error } => {
                self.terminated = true;
                out.extend(sse(
                    "error",
                    &json!({
                        "type": "error",
                        "error": { "type": "api_error", "message": error.to_string() },
                    }),
                ));
            }
        }
        out
    }

    fn finish(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        if self.terminated {
            return out;
        }
        self.terminated = true;
        self.start(&mut out);
        self.close_block(&mut out);
        out.extend(sse(
            "message_delta",
            &json!({
                "type": "message_delta",
                "delta": {
                    "stop_reason": stop_reason_anthropic(&self.stop_reason),
                    "stop_sequence": Value::Null,
                },
                // Final, complete usage: `message_start` went out before the
                // provider reported any, so this is where input and cache counts
                // reach the client (and the gateway's own meter).
                "usage": self.usage.to_anthropic(),
            }),
        ));
        out.extend(sse("message_stop", &json!({ "type": "message_stop" })));
        out
    }
}

// ── OpenAI Chat Completions ───────────────────────────────────────────────────

/// Encodes an OpenAI Chat Completions chunk stream, terminated by `[DONE]`.
#[derive(Debug)]
pub struct OpenAiStreamEncoder {
    id: String,
    model: String,
    created: u64,
    /// Wire index per source tool-call index, in the order calls appear.
    tool_indices: Vec<u32>,
    usage: Usage,
    terminated: bool,
}

impl OpenAiStreamEncoder {
    pub fn new(ctx: &StreamContext<'_>) -> Self {
        Self {
            id: format!("chatcmpl-{}", ctx.request_id.0),
            model: ctx.model.to_owned(),
            created: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            tool_indices: Vec::new(),
            usage: Usage::default(),
            terminated: false,
        }
    }

    fn chunk(&self, choices: Value) -> Vec<u8> {
        let body = json!({
            "id": self.id,
            "object": "chat.completion.chunk",
            "created": self.created,
            "model": self.model,
            "choices": choices,
        });
        format!("data: {body}\n\n").into_bytes()
    }

    fn delta(&self, delta: Value) -> Vec<u8> {
        self.chunk(json!([{ "index": 0, "delta": delta, "finish_reason": Value::Null }]))
    }

    /// Stable wire index for a source tool-call index, assigned on first use.
    fn tool_index(&mut self, source_index: u32) -> usize {
        match self.tool_indices.iter().position(|i| *i == source_index) {
            Some(pos) => pos,
            None => {
                self.tool_indices.push(source_index);
                self.tool_indices.len() - 1
            }
        }
    }
}

impl StreamEncoder for OpenAiStreamEncoder {
    fn encode(&mut self, event: &ConversationEvent) -> Vec<u8> {
        if self.terminated {
            return Vec::new();
        }
        match event {
            ConversationEvent::Started { .. } => {
                self.delta(json!({ "role": "assistant", "content": "" }))
            }
            ConversationEvent::OutputDelta { delta, .. } => self.delta(json!({ "content": delta })),
            // OpenAI carries reasoning in its own field, never in `content`.
            ConversationEvent::ReasoningDelta { delta, .. } => {
                self.delta(json!({ "reasoning_content": delta }))
            }
            ConversationEvent::ToolCallDelta {
                tool_use_id,
                name,
                input_delta,
                index,
            } => {
                let wire_index = self.tool_index(*index);
                let mut call = json!({ "index": wire_index });
                // Identify the call once; later fragments carry arguments only.
                if !name.is_empty() {
                    call["id"] = json!(tool_use_id);
                    call["type"] = json!("function");
                    call["function"] = json!({ "name": name, "arguments": input_delta });
                } else {
                    call["function"] = json!({ "arguments": input_delta });
                }
                self.delta(json!({ "tool_calls": [call] }))
            }
            // OpenAI has no per-call terminator: arguments simply stop arriving.
            ConversationEvent::ToolCallEnd { .. } => Vec::new(),
            ConversationEvent::Usage {
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_creation_tokens,
            } => {
                self.usage.update(
                    input_tokens,
                    output_tokens,
                    cache_read_tokens,
                    cache_creation_tokens,
                );
                Vec::new()
            }
            ConversationEvent::Completed { stop_reason } => {
                let mut out = self.chunk(json!([{
                    "index": 0,
                    "delta": {},
                    "finish_reason": stop_reason_openai(stop_reason),
                }]));
                self.terminated = true;
                out.extend(self.usage_chunk());
                out.extend_from_slice(b"data: [DONE]\n\n");
                out
            }
            ConversationEvent::Failed { error } => {
                self.terminated = true;
                let body = json!({
                    "error": { "type": "api_error", "message": error.to_string() },
                });
                let mut out = format!("data: {body}\n\n").into_bytes();
                out.extend_from_slice(b"data: [DONE]\n\n");
                out
            }
        }
    }

    fn finish(&mut self) -> Vec<u8> {
        if self.terminated {
            return Vec::new();
        }
        self.terminated = true;
        // No upstream stop event: close with `stop` so the client is not left hanging.
        let mut out = self.chunk(json!([{
            "index": 0,
            "delta": {},
            "finish_reason": "stop",
        }]));
        out.extend(self.usage_chunk());
        out.extend_from_slice(b"data: [DONE]\n\n");
        out
    }
}

impl OpenAiStreamEncoder {
    /// Usage-only chunk; omitted when the provider reported nothing.
    fn usage_chunk(&self) -> Vec<u8> {
        if self.usage.input == 0 && self.usage.output == 0 {
            return Vec::new();
        }
        let mut usage = json!({
            "prompt_tokens": self.usage.input,
            "completion_tokens": self.usage.output,
            "total_tokens": self.usage.input + self.usage.output,
        });
        if self.usage.cache_read > 0 {
            usage["prompt_tokens_details"] = json!({ "cached_tokens": self.usage.cache_read });
        }
        let body = json!({
            "id": self.id,
            "object": "chat.completion.chunk",
            "created": self.created,
            "model": self.model,
            "choices": [],
            "usage": usage,
        });
        format!("data: {body}\n\n").into_bytes()
    }
}

#[cfg(test)]
mod usage_tests {
    use super::*;
    use crate::{ConversationEvent, StreamContext, StreamEncoder, UsageCount};

    fn usage_event() -> ConversationEvent {
        ConversationEvent::Usage {
            input_tokens: UsageCount::Estimated(500),
            output_tokens: UsageCount::Reported(3),
            cache_read_tokens: UsageCount::Reported(40),
            cache_creation_tokens: UsageCount::Unknown,
        }
    }

    fn sse_json(bytes: &[u8], event_type: &str) -> serde_json::Value {
        let text = String::from_utf8_lossy(bytes);
        text.lines()
            .filter_map(|l| l.strip_prefix("data: "))
            .filter_map(|d| serde_json::from_str::<serde_json::Value>(d).ok())
            .find(|v| {
                v["type"] == event_type
                    || (event_type == "usage"
                        && v.get("usage").is_some()
                        && v["choices"].as_array().is_some_and(Vec::is_empty))
            })
            .unwrap_or_else(|| panic!("no {event_type} in {text}"))
    }

    // Found live: Kiro key usage recorded input 0. message_start goes out before
    // any usage is known, and message_delta carried output only, so the
    // decoder's input count never reached the client or the meter.
    #[test]
    fn anthropic_message_delta_carries_the_full_final_usage() {
        let rid = vkdg_core::RequestId::new();
        let mut enc = AnthropicStreamEncoder::new(&StreamContext {
            model: "m",
            request_id: &rid,
        });
        let mut out = enc.encode(&usage_event());
        out.extend(enc.finish());
        let delta = sse_json(&out, "message_delta");
        assert_eq!(delta["usage"]["input_tokens"], 500, "{delta}");
        assert_eq!(delta["usage"]["output_tokens"], 3, "{delta}");
        assert_eq!(delta["usage"]["cache_read_input_tokens"], 40, "{delta}");
    }

    #[test]
    fn openai_final_usage_chunk_carries_prompt_tokens() {
        let rid = vkdg_core::RequestId::new();
        let mut enc = OpenAiStreamEncoder::new(&StreamContext {
            model: "m",
            request_id: &rid,
        });
        let mut out = enc.encode(&usage_event());
        out.extend(enc.finish());
        let chunk = sse_json(&out, "usage");
        assert!(
            chunk["usage"]["prompt_tokens"].as_u64().unwrap() >= 500,
            "{chunk}"
        );
        assert_eq!(chunk["usage"]["completion_tokens"], 3, "{chunk}");
    }
}
