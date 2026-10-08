//! Non-streaming response encoders.
//!
//! A non-streaming upstream body is decoded to the same [`ConversationEvent`]s a
//! stream produces, and a [`JsonEncoder`] folds them into one body in the
//! client's dialect. Streams and complete bodies therefore share one mapping.
//!
//! Everything is accumulated in memory, so the total is bounded by
//! [`MAX_JSON_RESPONSE_BYTES`]; a longer answer is a clear gateway error, not an
//! unbounded allocation.

use serde::Serialize;
use serde_json::{Map, Value};
use vkdg_core::{ApiType, VkdgError};

use crate::stop_wire::{anthropic_stop_reason, openai_finish_reason};
use crate::tool_id::ToolIdMint;
use crate::usage::{AnthropicUsage, OpenAiUsage, UsageTally};
use crate::{ConversationEvent, StopReason, StreamContext};

/// Most text, reasoning and tool-argument bytes one non-streaming response may hold.
pub const MAX_JSON_RESPONSE_BYTES: usize = 32 << 20;

/// Folds a conversation's events into one complete response body.
pub trait JsonEncoder: Send {
    /// Feeds the next event. Never fails: the first problem is reported by
    /// [`finish`](Self::finish).
    fn push(&mut self, event: &ConversationEvent);

    /// The complete body, or the error that makes the response unusable (a
    /// `Failed` event, a malformed stream, an oversize answer).
    fn finish(self: Box<Self>) -> Result<Vec<u8>, VkdgError>;
}

/// The encoder for the dialect the client spoke (`OpenAI` Chat or, for every
/// other dialect, Anthropic Messages, as in [`stream_encoder_for`](crate::stream_encoder_for)).
pub fn json_encoder_for(api_type: &ApiType, ctx: &StreamContext<'_>) -> Box<dyn JsonEncoder> {
    let openai = matches!(api_type, ApiType::OpenAiChatCompletions);
    Box::new(Collector::new(ctx, openai))
}

fn upstream(message: &str) -> VkdgError {
    VkdgError::UpstreamError {
        code: 502,
        message: message.to_owned(),
        retry_after: None,
    }
}

/// One content block in the order it arrived.
#[derive(Debug)]
enum Block {
    Text {
        index: u32,
        text: String,
    },
    Thinking {
        index: u32,
        text: String,
    },
    Tool {
        index: u32,
        id: String,
        name: String,
        arguments: String,
    },
}

#[derive(Debug)]
struct Collector {
    openai: bool,
    id: String,
    model: String,
    blocks: Vec<Block>,
    /// Source indices of tool calls opened so far.
    tools_seen: Vec<u32>,
    ids: ToolIdMint,
    usage: UsageTally,
    stop: Option<StopReason>,
    /// Anything beyond `Started` arrived.
    saw_content: bool,
    bytes: usize,
    error: Option<VkdgError>,
}

impl Collector {
    fn new(ctx: &StreamContext<'_>, openai: bool) -> Self {
        let rid = ctx.request_id.0.to_string();
        Self {
            openai,
            id: if openai {
                format!("chatcmpl-{rid}")
            } else {
                format!("msg_{rid}")
            },
            model: ctx.model.to_owned(),
            blocks: Vec::new(),
            tools_seen: Vec::new(),
            ids: ToolIdMint::new(if openai { "call_" } else { "toolu_" }, &rid),
            usage: UsageTally::default(),
            stop: None,
            saw_content: false,
            bytes: 0,
            error: None,
        }
    }

    fn fail(&mut self, error: VkdgError) {
        if self.error.is_none() {
            self.error = Some(error);
        }
    }

    /// Accounts `n` more bytes; false (and an error) when over the limit.
    fn reserve(&mut self, n: usize) -> bool {
        self.bytes = self.bytes.saturating_add(n);
        if self.bytes > MAX_JSON_RESPONSE_BYTES {
            self.fail(upstream(
                "upstream response exceeds the non-streaming size limit",
            ));
            return false;
        }
        true
    }

    /// Text or reasoning: extends the last block when it is the same kind and index.
    fn push_text(&mut self, delta: &str, index: u32, thinking: bool) {
        if !self.reserve(delta.len()) {
            return;
        }
        match self.blocks.last_mut() {
            Some(Block::Text { index: i, text }) if !thinking && *i == index => {
                text.push_str(delta);
            }
            Some(Block::Thinking { index: i, text }) if thinking && *i == index => {
                text.push_str(delta);
            }
            _ => self.blocks.push(if thinking {
                Block::Thinking {
                    index,
                    text: delta.to_owned(),
                }
            } else {
                Block::Text {
                    index,
                    text: delta.to_owned(),
                }
            }),
        }
    }

    fn push_tool(&mut self, tool_use_id: &str, name: &str, fragment: &str, index: u32) {
        if !self.reserve(fragment.len()) {
            return;
        }
        let continues = matches!(
            self.blocks.last(),
            Some(Block::Tool { index: i, .. }) if *i == index
        );
        if continues && name.is_empty() {
            if let Some(Block::Tool { arguments, .. }) = self.blocks.last_mut() {
                arguments.push_str(fragment);
            }
        } else if name.is_empty() || self.tools_seen.contains(&index) {
            // A fragment for a call that never opened, or that already closed.
            self.fail(upstream("upstream sent tool call fragments out of order"));
        } else {
            self.tools_seen.push(index);
            let id = self.ids.id_for(tool_use_id);
            self.blocks.push(Block::Tool {
                index,
                id,
                name: name.to_owned(),
                arguments: fragment.to_owned(),
            });
        }
    }

    fn render(self) -> Result<Vec<u8>, VkdgError> {
        if let Some(error) = self.error {
            return Err(error);
        }
        if !self.saw_content {
            return Err(upstream("upstream returned an empty response"));
        }
        let reason = self.stop.clone().unwrap_or(if self.tools_seen.is_empty() {
            StopReason::EndTurn
        } else {
            StopReason::ToolUse
        });
        let body = if self.openai {
            serde_json::to_vec(&self.openai_body(&reason))
        } else {
            serde_json::to_vec(&self.anthropic_body(&reason)?)
        };
        body.map_err(|_| VkdgError::Internal("response body did not serialize".into()))
    }

    fn anthropic_body(&self, reason: &StopReason) -> Result<AnthropicMessage<'_>, VkdgError> {
        let content = self
            .blocks
            .iter()
            .map(|block| match block {
                Block::Text { text, .. } => Ok(AnthropicBlock::Text { text }),
                Block::Thinking { text, .. } => Ok(AnthropicBlock::Thinking {
                    thinking: text,
                    signature: "",
                }),
                Block::Tool {
                    id,
                    name,
                    arguments,
                    ..
                } => Ok(AnthropicBlock::ToolUse {
                    id,
                    name,
                    input: tool_input(arguments)?,
                }),
            })
            .collect::<Result<Vec<_>, VkdgError>>()?;
        Ok(AnthropicMessage {
            id: &self.id,
            kind: "message",
            role: "assistant",
            model: &self.model,
            content,
            stop_reason: anthropic_stop_reason(reason),
            stop_sequence: None,
            usage: self.usage.to_anthropic(),
        })
    }

    fn openai_body(&self, reason: &StopReason) -> OpenAiCompletion<'_> {
        let mut text = String::new();
        let mut reasoning = String::new();
        let mut tool_calls = Vec::new();
        for block in &self.blocks {
            match block {
                Block::Text { text: t, .. } => text.push_str(t),
                Block::Thinking { text: t, .. } => reasoning.push_str(t),
                Block::Tool {
                    id,
                    name,
                    arguments,
                    ..
                } => tool_calls.push(OpenAiToolCall {
                    id,
                    kind: "function",
                    function: OpenAiFunction {
                        name,
                        arguments: if arguments.is_empty() {
                            "{}"
                        } else {
                            arguments
                        },
                    },
                }),
            }
        }
        // `content` is null next to tool calls when the model wrote no text.
        let content = if text.is_empty() && !tool_calls.is_empty() {
            None
        } else {
            Some(text)
        };
        OpenAiCompletion {
            id: &self.id,
            object: "chat.completion",
            created: unix_now(),
            model: &self.model,
            choices: [OpenAiChoice {
                index: 0,
                message: OpenAiMessage {
                    role: "assistant",
                    content,
                    reasoning_content: (!reasoning.is_empty()).then_some(reasoning),
                    tool_calls: (!tool_calls.is_empty()).then_some(tool_calls),
                },
                finish_reason: openai_finish_reason(reason),
            }],
            usage: self.usage.is_known().then(|| self.usage.to_openai()),
        }
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Anthropic `input` is a JSON object; empty arguments mean no arguments.
fn tool_input(arguments: &str) -> Result<Map<String, Value>, VkdgError> {
    if arguments.trim().is_empty() {
        return Ok(Map::new());
    }
    match serde_json::from_str::<Value>(arguments) {
        Ok(Value::Object(map)) => Ok(map),
        // Never echo the arguments: they are user payload.
        _ => Err(upstream(
            "upstream returned tool call arguments that are not valid JSON",
        )),
    }
}

impl JsonEncoder for Collector {
    fn push(&mut self, event: &ConversationEvent) {
        if self.error.is_some() {
            return;
        }
        match event {
            ConversationEvent::Started { .. } => {}
            ConversationEvent::OutputDelta { delta, index } => {
                self.saw_content = true;
                self.push_text(delta, *index, false);
            }
            ConversationEvent::ReasoningDelta { delta, index } => {
                self.saw_content = true;
                self.push_text(delta, *index, true);
            }
            ConversationEvent::ToolCallDelta {
                tool_use_id,
                name,
                input_delta,
                index,
            } => {
                self.saw_content = true;
                self.push_tool(tool_use_id, name, input_delta, *index);
            }
            ConversationEvent::ToolCallEnd { .. } => {}
            ConversationEvent::Usage {
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_creation_tokens,
            } => {
                self.saw_content = true;
                self.usage.update(
                    input_tokens,
                    output_tokens,
                    cache_read_tokens,
                    cache_creation_tokens,
                );
            }
            ConversationEvent::Completed { stop_reason } => {
                self.saw_content = true;
                self.stop = Some(stop_reason.clone());
            }
            ConversationEvent::Failed { error } => self.fail(error.clone()),
        }
    }

    fn finish(self: Box<Self>) -> Result<Vec<u8>, VkdgError> {
        self.render()
    }
}

// Field order is the wire order.

#[derive(Serialize)]
struct AnthropicMessage<'a> {
    id: &'a str,
    #[serde(rename = "type")]
    kind: &'static str,
    role: &'static str,
    model: &'a str,
    content: Vec<AnthropicBlock<'a>>,
    stop_reason: &'static str,
    stop_sequence: Option<&'static str>,
    usage: AnthropicUsage,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AnthropicBlock<'a> {
    Text {
        text: &'a str,
    },
    Thinking {
        thinking: &'a str,
        signature: &'static str,
    },
    ToolUse {
        id: &'a str,
        name: &'a str,
        input: Map<String, Value>,
    },
}

#[derive(Serialize)]
struct OpenAiCompletion<'a> {
    id: &'a str,
    object: &'static str,
    created: u64,
    model: &'a str,
    choices: [OpenAiChoice<'a>; 1],
    #[serde(skip_serializing_if = "Option::is_none")]
    usage: Option<OpenAiUsage>,
}

#[derive(Serialize)]
struct OpenAiChoice<'a> {
    index: u32,
    message: OpenAiMessage<'a>,
    finish_reason: &'static str,
}

#[derive(Serialize)]
struct OpenAiMessage<'a> {
    role: &'static str,
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OpenAiToolCall<'a>>>,
}

#[derive(Serialize)]
struct OpenAiToolCall<'a> {
    id: &'a str,
    #[serde(rename = "type")]
    kind: &'static str,
    function: OpenAiFunction<'a>,
}

#[derive(Serialize)]
struct OpenAiFunction<'a> {
    name: &'a str,
    arguments: &'a str,
}

#[cfg(test)]
mod tests;
