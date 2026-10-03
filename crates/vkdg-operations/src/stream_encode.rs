//! Client-dialect stream encoders.
//!
//! One [`StreamEncoder`] instance per response stream turns
//! [`ConversationEvent`]s into wire bytes for the dialect the client spoke.
//! This is the only place a dialect is written, so every ingress and every
//! provider that needs protocol translation emits the same bytes.
//!
//! Wire objects are `Serialize` structs, not `json!` maps: serde keeps field
//! order, which is the order the dialects document and clients' fixtures expect.

use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use vkdg_core::ApiType;

use crate::error_encode::{protocol_error_frame, stream_error_frame};
use crate::stop_wire::{anthropic_stop_reason, openai_finish_reason};
use crate::tool_id::ToolIdMint;
use crate::usage::{AnthropicUsage, OpenAiUsage, UsageTally};
use crate::{ConversationEvent, StopReason, StreamContext, StreamEncoder};

const OUT_OF_ORDER: &str = "tool call fragments arrived out of order";

fn write_sse<T: Serialize>(out: &mut Vec<u8>, event: Option<&str>, data: &T) {
    if let Some(event) = event {
        out.extend_from_slice(b"event: ");
        out.extend_from_slice(event.as_bytes());
        out.push(b'\n');
    }
    out.extend_from_slice(b"data: ");
    // Wire structs hold strings and integers only; serialization cannot fail.
    let _ = serde_json::to_writer(&mut *out, data);
    out.extend_from_slice(b"\n\n");
}

// ── Anthropic Messages ────────────────────────────────────────────────────────

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

#[derive(Serialize)]
struct MessageStart<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    message: MessageObject<'a>,
}

#[derive(Serialize)]
struct MessageObject<'a> {
    id: &'a str,
    #[serde(rename = "type")]
    kind: &'static str,
    role: &'static str,
    model: &'a str,
    content: [(); 0],
    stop_reason: Option<&'static str>,
    stop_sequence: Option<&'static str>,
    usage: AnthropicUsage,
}

#[derive(Serialize)]
struct BlockStart<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    index: u32,
    content_block: ContentBlock<'a>,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ContentBlock<'a> {
    Text {
        text: &'static str,
    },
    Thinking {
        thinking: &'static str,
    },
    ToolUse {
        id: &'a str,
        name: &'a str,
        input: Empty,
    },
}

#[derive(Serialize)]
struct Empty {}

#[derive(Serialize)]
struct BlockDelta<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    index: u32,
    delta: DeltaPayload<'a>,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(clippy::enum_variant_names)] // `text_delta` etc. are the wire names
enum DeltaPayload<'a> {
    TextDelta { text: &'a str },
    ThinkingDelta { thinking: &'a str },
    InputJsonDelta { partial_json: &'a str },
}

#[derive(Serialize)]
struct BlockStop {
    #[serde(rename = "type")]
    kind: &'static str,
    index: u32,
}

#[derive(Serialize)]
struct MessageDelta {
    #[serde(rename = "type")]
    kind: &'static str,
    delta: StopPayload,
    usage: AnthropicUsage,
}

#[derive(Serialize)]
struct StopPayload {
    stop_reason: &'static str,
    stop_sequence: Option<&'static str>,
}

#[derive(Serialize)]
struct MessageStop {
    #[serde(rename = "type")]
    kind: &'static str,
}

/// Encodes an Anthropic Messages SSE stream.
///
/// Guarantees a well-formed stream: `message_start` carries a real message
/// object, every delta sits inside a `content_block_start`/`content_block_stop`
/// pair with sequential wire indices, and the stream always terminates with
/// `message_delta` + `message_stop`, or with an `error` event after a failure,
/// even when the provider sends no stop event.
#[derive(Debug)]
pub struct AnthropicStreamEncoder {
    message_id: String,
    model: String,
    started: bool,
    open: Option<OpenBlock>,
    next_wire_index: u32,
    /// Source indices of tool calls already opened; a call opens once.
    tool_calls_seen: Vec<u32>,
    ids: ToolIdMint,
    usage: UsageTally,
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
            tool_calls_seen: Vec::new(),
            ids: ToolIdMint::new("toolu_", &ctx.request_id.0.to_string()),
            usage: UsageTally::default(),
            terminated: false,
        }
    }

    fn ensure_started(&mut self, out: &mut Vec<u8>) {
        if self.started {
            return;
        }
        self.started = true;
        let start = MessageStart {
            kind: "message_start",
            message: MessageObject {
                id: &self.message_id,
                kind: "message",
                role: "assistant",
                model: &self.model,
                content: [],
                stop_reason: None,
                stop_sequence: None,
                usage: self.usage.to_anthropic(),
            },
        };
        write_sse(out, Some("message_start"), &start);
    }

    fn close_block(&mut self, out: &mut Vec<u8>) {
        if let Some(block) = self.open.take() {
            let stop = BlockStop {
                kind: "content_block_stop",
                index: block.wire_index,
            };
            write_sse(out, Some("content_block_stop"), &stop);
        }
    }

    /// Opens a block of `kind` for `source_index` unless it is the one already open.
    fn open_block(
        &mut self,
        kind: BlockKind,
        source_index: u32,
        content_block: ContentBlock<'_>,
        out: &mut Vec<u8>,
    ) -> u32 {
        if let Some(open) = self.open {
            if open.kind == kind && open.source_index == source_index {
                return open.wire_index;
            }
            self.close_block(out);
        }
        let wire_index = self.next_wire_index;
        self.next_wire_index += 1;
        self.open = Some(OpenBlock {
            kind,
            source_index,
            wire_index,
        });
        let start = BlockStart {
            kind: "content_block_start",
            index: wire_index,
            content_block,
        };
        write_sse(out, Some("content_block_start"), &start);
        wire_index
    }

    fn delta(out: &mut Vec<u8>, index: u32, delta: DeltaPayload<'_>) {
        let frame = BlockDelta {
            kind: "content_block_delta",
            index,
            delta,
        };
        write_sse(out, Some("content_block_delta"), &frame);
    }

    fn tool_fragment(
        &mut self,
        tool_use_id: &str,
        name: &str,
        input_delta: &str,
        index: u32,
        out: &mut Vec<u8>,
    ) {
        let continuing = self
            .open
            .is_some_and(|b| b.kind == BlockKind::ToolUse && b.source_index == index);
        let wire_index = if continuing {
            self.open.map_or(0, |b| b.wire_index)
        } else if self.tool_calls_seen.contains(&index) || name.is_empty() {
            // A fragment for a call that closed, or for one that never opened.
            self.terminate_with(
                &protocol_error_frame(&ApiType::AnthropicMessages, OUT_OF_ORDER),
                out,
            );
            return;
        } else {
            self.tool_calls_seen.push(index);
            let id = self.ids.id_for(tool_use_id);
            self.open_block(
                BlockKind::ToolUse,
                index,
                ContentBlock::ToolUse {
                    id: &id,
                    name,
                    input: Empty {},
                },
                out,
            )
        };
        if !input_delta.is_empty() {
            Self::delta(
                out,
                wire_index,
                DeltaPayload::InputJsonDelta {
                    partial_json: input_delta,
                },
            );
        }
    }

    fn terminate_with(&mut self, frame: &[u8], out: &mut Vec<u8>) {
        self.ensure_started(out);
        out.extend_from_slice(frame);
        self.terminated = true;
    }

    fn complete(&mut self, reason: &StopReason, out: &mut Vec<u8>) {
        self.ensure_started(out);
        self.close_block(out);
        let delta = MessageDelta {
            kind: "message_delta",
            delta: StopPayload {
                stop_reason: anthropic_stop_reason(reason),
                stop_sequence: None,
            },
            usage: self.usage.to_anthropic(),
        };
        write_sse(out, Some("message_delta"), &delta);
        write_sse(
            out,
            Some("message_stop"),
            &MessageStop {
                kind: "message_stop",
            },
        );
        self.terminated = true;
    }
}

impl StreamEncoder for AnthropicStreamEncoder {
    fn encode_into(&mut self, event: &ConversationEvent, out: &mut Vec<u8>) {
        if self.terminated {
            return;
        }
        match event {
            ConversationEvent::Started { .. } => self.ensure_started(out),
            ConversationEvent::OutputDelta { delta, index } => {
                self.ensure_started(out);
                let wire = self.open_block(
                    BlockKind::Text,
                    *index,
                    ContentBlock::Text { text: "" },
                    out,
                );
                Self::delta(out, wire, DeltaPayload::TextDelta { text: delta });
            }
            ConversationEvent::ReasoningDelta { delta, index } => {
                self.ensure_started(out);
                let wire = self.open_block(
                    BlockKind::Thinking,
                    *index,
                    ContentBlock::Thinking { thinking: "" },
                    out,
                );
                Self::delta(out, wire, DeltaPayload::ThinkingDelta { thinking: delta });
            }
            ConversationEvent::ToolCallDelta {
                tool_use_id,
                name,
                input_delta,
                index,
            } => {
                self.ensure_started(out);
                self.tool_fragment(tool_use_id, name, input_delta, *index, out);
            }
            ConversationEvent::ToolCallEnd { index } => {
                if self
                    .open
                    .is_some_and(|b| b.kind == BlockKind::ToolUse && b.source_index == *index)
                {
                    self.close_block(out);
                }
            }
            ConversationEvent::Usage {
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_creation_tokens,
            } => self.usage.update(
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_creation_tokens,
            ),
            ConversationEvent::Completed { stop_reason } => self.complete(stop_reason, out),
            ConversationEvent::Failed { error } => {
                let frame = stream_error_frame(&ApiType::AnthropicMessages, error);
                self.terminate_with(&frame, out);
            }
        }
    }

    fn finish_into(&mut self, out: &mut Vec<u8>) {
        if self.terminated {
            return;
        }
        // A stream that ends after a tool call without a stop event is a tool turn.
        let reason = if self.tool_calls_seen.is_empty() {
            StopReason::EndTurn
        } else {
            StopReason::ToolUse
        };
        self.complete(&reason, out);
    }
}

// ── OpenAI Chat Completions ───────────────────────────────────────────────────

#[derive(Serialize)]
struct Chunk<'a> {
    id: &'a str,
    object: &'static str,
    created: u64,
    model: &'a str,
    choices: Vec<Choice<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    usage: Option<OpenAiUsage>,
}

#[derive(Serialize)]
struct Choice<'a> {
    index: u32,
    delta: Delta<'a>,
    finish_reason: Option<&'static str>,
}

#[derive(Serialize, Default)]
struct Delta<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_content: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<[ToolCallChunk<'a>; 1]>,
}

#[derive(Serialize)]
struct ToolCallChunk<'a> {
    index: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<&'a str>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    kind: Option<&'static str>,
    function: FunctionChunk<'a>,
}

#[derive(Serialize)]
struct FunctionChunk<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
    arguments: &'a str,
}

/// A tool call as the `OpenAI` client sees it.
#[derive(Debug)]
struct OpenAiToolCall {
    source_index: u32,
    closed: bool,
}

/// Encodes an `OpenAI` Chat Completions chunk stream, ending with a usage
/// chunk (when any count is known) and `[DONE]`.
#[derive(Debug)]
pub struct OpenAiStreamEncoder {
    id: String,
    model: String,
    created: u64,
    started: bool,
    /// Calls in wire order: the position is the wire `index`.
    calls: Vec<OpenAiToolCall>,
    ids: ToolIdMint,
    usage: UsageTally,
    terminated: bool,
}

impl OpenAiStreamEncoder {
    pub fn new(ctx: &StreamContext<'_>) -> Self {
        Self {
            id: format!("chatcmpl-{}", ctx.request_id.0),
            model: ctx.model.to_owned(),
            created: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_secs()),
            started: false,
            calls: Vec::new(),
            ids: ToolIdMint::new("call_", &ctx.request_id.0.to_string()),
            usage: UsageTally::default(),
            terminated: false,
        }
    }

    fn chunk(&self, out: &mut Vec<u8>, delta: Delta<'_>, finish: Option<&'static str>) {
        let chunk = Chunk {
            id: &self.id,
            object: "chat.completion.chunk",
            created: self.created,
            model: &self.model,
            choices: vec![Choice {
                index: 0,
                delta,
                finish_reason: finish,
            }],
            usage: None,
        };
        write_sse(out, None, &chunk);
    }

    fn ensure_started(&mut self, out: &mut Vec<u8>) {
        if self.started {
            return;
        }
        self.started = true;
        self.chunk(
            out,
            Delta {
                role: Some("assistant"),
                content: Some(""),
                ..Delta::default()
            },
            None,
        );
    }

    fn tool_fragment(
        &mut self,
        tool_use_id: &str,
        name: &str,
        input_delta: &str,
        index: u32,
        out: &mut Vec<u8>,
    ) {
        let known = self.calls.iter().position(|c| c.source_index == index);
        let (wire, opening, id) = match known {
            Some(pos) if !self.calls[pos].closed => (pos, false, None),
            // A fragment for a call that closed, or for one that never opened.
            Some(_) => return self.fail(out),
            None if name.is_empty() => return self.fail(out),
            None => {
                self.calls.push(OpenAiToolCall {
                    source_index: index,
                    closed: false,
                });
                (
                    self.calls.len() - 1,
                    true,
                    Some(self.ids.id_for(tool_use_id)),
                )
            }
        };
        let wire = u32::try_from(wire).unwrap_or(u32::MAX);
        let call = ToolCallChunk {
            index: wire,
            id: id.as_deref(),
            kind: opening.then_some("function"),
            function: FunctionChunk {
                name: opening.then_some(name),
                arguments: input_delta,
            },
        };
        self.chunk(
            out,
            Delta {
                tool_calls: Some([call]),
                ..Delta::default()
            },
            None,
        );
    }

    fn fail(&mut self, out: &mut Vec<u8>) {
        out.extend_from_slice(&protocol_error_frame(
            &ApiType::OpenAiChatCompletions,
            OUT_OF_ORDER,
        ));
        self.terminated = true;
    }

    fn complete(&mut self, reason: &StopReason, out: &mut Vec<u8>) {
        self.ensure_started(out);
        self.chunk(out, Delta::default(), Some(openai_finish_reason(reason)));
        if self.usage.is_known() {
            let chunk = Chunk {
                id: &self.id,
                object: "chat.completion.chunk",
                created: self.created,
                model: &self.model,
                choices: Vec::new(),
                usage: Some(self.usage.to_openai()),
            };
            write_sse(out, None, &chunk);
        }
        out.extend_from_slice(b"data: [DONE]\n\n");
        self.terminated = true;
    }
}

impl StreamEncoder for OpenAiStreamEncoder {
    fn encode_into(&mut self, event: &ConversationEvent, out: &mut Vec<u8>) {
        if self.terminated {
            return;
        }
        match event {
            ConversationEvent::Started { .. } => self.ensure_started(out),
            ConversationEvent::OutputDelta { delta, .. } => {
                self.ensure_started(out);
                self.chunk(
                    out,
                    Delta {
                        content: Some(delta),
                        ..Delta::default()
                    },
                    None,
                );
            }
            ConversationEvent::ReasoningDelta { delta, .. } => {
                self.ensure_started(out);
                self.chunk(
                    out,
                    Delta {
                        reasoning_content: Some(delta),
                        ..Delta::default()
                    },
                    None,
                );
            }
            ConversationEvent::ToolCallDelta {
                tool_use_id,
                name,
                input_delta,
                index,
            } => {
                self.ensure_started(out);
                self.tool_fragment(tool_use_id, name, input_delta, *index, out);
            }
            ConversationEvent::ToolCallEnd { index } => {
                if let Some(call) = self.calls.iter_mut().find(|c| c.source_index == *index) {
                    call.closed = true;
                }
            }
            ConversationEvent::Usage {
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_creation_tokens,
            } => self.usage.update(
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_creation_tokens,
            ),
            ConversationEvent::Completed { stop_reason } => self.complete(stop_reason, out),
            ConversationEvent::Failed { error } => {
                out.extend_from_slice(&stream_error_frame(&ApiType::OpenAiChatCompletions, error));
                self.terminated = true;
            }
        }
    }

    fn finish_into(&mut self, out: &mut Vec<u8>) {
        if self.terminated {
            return;
        }
        let reason = if self.calls.is_empty() {
            StopReason::EndTurn
        } else {
            StopReason::ToolUse
        };
        self.complete(&reason, out);
    }
}

#[cfg(test)]
mod tests;
