//! Maps Kiro (`CodeWhisperer` `generateAssistantResponse`) `EventStream` frames to
//! [`ConversationEvent`]s. Mirrors `OmniRoute`'s `transformEventStreamToSSE`.
//!
//! Kiro has no explicit terminal event: `Completed` (and synthesized `Usage`) are
//! emitted from [`KiroEventDecoder::finish`] at end of stream.

use std::collections::HashMap;

use serde_json::Value;
use vkdg_core::{RequestId, VkdgError};
use vkdg_operations::{ConversationEvent, StopReason, UsageCount};

use crate::eventstream::{Frame, FrameError};

/// Context window used to turn `contextUsagePercentage` into tokens when the
/// model's own limit is unknown (`OmniRoute` `KIRO_DEFAULT_MAX_INPUT_TOKENS`).
const DEFAULT_MAX_INPUT_TOKENS: u64 = 200_000;

#[derive(Debug, Default)]
pub struct KiroEventDecoder {
    started: bool,
    terminated: bool,
    tool_indices: HashMap<String, u32>,
    buffered_tool_inputs: Vec<(String, u32, String)>,
    generated_tool_ids: u32,
    output_chars: u64,
    context_usage_pct: f64,
    reported_usage: Option<(u32, u32)>,
    cache_read: Option<u32>,
    cache_write: Option<u32>,
    /// State for the stream-safe `<thinking>…</thinking>` splitter.
    /// Tags can span chunk boundaries, so partial prefixes are held here.
    thinking_in_block: bool,
    thinking_pending: String,
}

impl KiroEventDecoder {
    pub fn new() -> Self {
        Self::default()
    }
    /// Returns `Some(pct)` if Kiro reported a context-usage percentage; `None` otherwise.
    pub fn context_usage_pct(&self) -> Option<f64> {
        if self.context_usage_pct > 0.0 {
            Some(self.context_usage_pct)
        } else {
            None
        }
    }

    pub fn on_frame(&mut self, frame: &Frame, out: &mut Vec<ConversationEvent>) {
        if self.terminated {
            return;
        }
        if !self.started {
            self.started = true;
            out.push(ConversationEvent::Started {
                request_id: RequestId::new(),
            });
        }

        let payload = parse_payload(&frame.payload);
        if let Some("exception" | "error") = frame.header_str(":message-type") {
            // AWS names the failure in a header; the Kiro plane puts an
            // `error_code` in the payload instead.
            let kind = frame
                .header_str(":exception-type")
                .or_else(|| frame.header_str(":error-code"))
                .or_else(|| error_code(&payload))
                .unwrap_or("UnknownException");
            let message = error_message(&payload).unwrap_or_default();
            self.fail(out, exception_status(kind), format!("{kind}: {message}"));
            return;
        }

        match frame.header_str(":event-type").unwrap_or_default() {
            "assistantResponseEvent" => {
                if let Some(content) = str_field(&payload, "content").filter(|c| !c.is_empty()) {
                    self.split_thinking(content, out);
                }
            }
            "codeEvent" => {
                if let Some(content) = str_field(&payload, "content").filter(|c| !c.is_empty()) {
                    out.push(ConversationEvent::OutputDelta {
                        delta: content.to_owned(),
                        index: 0,
                    });
                }
            }
            // Reasoning text carries its own field name (`text`), not `content`.
            // It goes out as ReasoningDelta so clients can render or hide it,
            // never merged into the visible answer.
            "reasoningContentEvent" => {
                if let Some(text) = str_field(&payload, "text").filter(|t| !t.is_empty()) {
                    out.push(ConversationEvent::ReasoningDelta {
                        delta: text.to_owned(),
                        index: 0,
                    });
                }
            }
            "toolUseEvent" => self.on_tool_use(payload, out),
            "messageStopEvent" => self.flush_buffered_tool_inputs(out),
            "contextUsageEvent" => {
                // The Kiro plane sends snake_case here, unlike its other events.
                if let Some(pct) = ["context_usage_percentage", "contextUsagePercentage"]
                    .iter()
                    .find_map(|k| payload.get(*k).and_then(Value::as_f64))
                    .filter(|p| *p > 0.0)
                {
                    self.context_usage_pct = pct;
                }
            }
            "metadataEvent" | "messageMetadataEvent" | "metricsEvent" | "usageEvent" => {
                self.on_metrics(&payload);
            }
            "invalidStateEvent" => {
                let message = error_message(&payload).unwrap_or("invalid state");
                self.fail(out, 502, format!("invalidStateEvent: {message}"));
            }
            // meteringEvent (credits), codeReferenceEvent, supplementaryWebLinksEvent,
            // followupPromptEvent and unknown events carry nothing to forward.
            _ => {}
        }
    }

    /// Surfaces a framing error; the stream is unusable afterwards.
    pub fn on_frame_error(&mut self, err: &FrameError, out: &mut Vec<ConversationEvent>) {
        self.fail(out, 502, format!("kiro eventstream: {err}"));
    }

    /// End of upstream stream: flush tool inputs, usage, and the terminal event.
    pub fn finish(&mut self, out: &mut Vec<ConversationEvent>) {
        if self.terminated {
            return;
        }
        if !self.started {
            self.fail(out, 502, "kiro stream ended without any event".to_owned());
            return;
        }
        self.terminated = true;
        // Flush any partial thinking content accumulated in thinking_pending
        // before closing the stream — mirrors OmniRoute's flushPendingThinking.
        self.flush_thinking(out);
        self.flush_buffered_tool_inputs(out);
        if let Some((input_tokens, output_tokens)) = self.usage() {
            out.push(ConversationEvent::Usage {
                input_tokens,
                output_tokens,
                cache_read_tokens: self
                    .cache_read
                    .map_or(UsageCount::Unknown, UsageCount::Reported),
                cache_creation_tokens: self
                    .cache_write
                    .map_or(UsageCount::Unknown, UsageCount::Reported),
            });
        }
        let stop_reason = if self.tool_indices.is_empty() {
            StopReason::EndTurn
        } else {
            StopReason::ToolUse
        };
        out.push(ConversationEvent::Completed { stop_reason });
    }

    fn fail(&mut self, out: &mut Vec<ConversationEvent>, code: u16, message: String) {
        self.terminated = true;
        out.push(ConversationEvent::Failed {
            error: VkdgError::UpstreamError {
                code,
                message,
                retry_after: None,
            },
        });
    }

    fn on_tool_use(&mut self, payload: Value, out: &mut Vec<ConversationEvent>) {
        let uses = match payload {
            Value::Array(items) => items,
            Value::Object(_) => vec![payload],
            _ => return,
        };
        for tool_use in uses {
            let Some(name) = str_field(&tool_use, "name")
                .map(str::trim)
                .filter(|n| !n.is_empty())
            else {
                self.fail(
                    out,
                    502,
                    "invalid Kiro toolUseEvent: missing tool name".to_owned(),
                );
                return;
            };
            let id = if let Some(id) = str_field(&tool_use, "toolUseId").filter(|id| !id.is_empty())
            {
                id.to_owned()
            } else {
                self.generated_tool_ids += 1;
                format!("call_kiro_{}", self.generated_tool_ids)
            };
            let next = u32::try_from(self.tool_indices.len()).unwrap_or(u32::MAX);
            let index = *self.tool_indices.entry(id.clone()).or_insert_with(|| {
                out.push(ConversationEvent::ToolCallDelta {
                    tool_use_id: id.clone(),
                    name: name.to_owned(),
                    input_delta: String::new(),
                    index: next,
                });
                next
            });
            match tool_use.get("input") {
                Some(Value::String(fragment)) if !fragment.is_empty() => {
                    out.push(ConversationEvent::ToolCallDelta {
                        tool_use_id: id,
                        name: String::new(),
                        input_delta: fragment.clone(),
                        index,
                    });
                }
                Some(obj @ Value::Object(_)) => {
                    let canonical = obj.to_string();
                    match self
                        .buffered_tool_inputs
                        .iter_mut()
                        .find(|(i, ..)| *i == id)
                    {
                        Some(entry) => entry.2 = canonical,
                        None => self.buffered_tool_inputs.push((id, index, canonical)),
                    }
                }
                _ => {}
            }
            // `stop: true` closes this call: the input will not grow, so clients
            // can parse and dispatch it without waiting for end of stream.
            if tool_use.get("stop").and_then(Value::as_bool) == Some(true) {
                self.flush_buffered_tool_inputs(out);
                out.push(ConversationEvent::ToolCallEnd { index });
            }
        }
    }

    fn flush_buffered_tool_inputs(&mut self, out: &mut Vec<ConversationEvent>) {
        for (tool_use_id, index, input_delta) in self.buffered_tool_inputs.drain(..) {
            out.push(ConversationEvent::ToolCallDelta {
                tool_use_id,
                name: String::new(),
                input_delta,
                index,
            });
        }
    }

    fn on_metrics(&mut self, payload: &Value) {
        let metrics = payload
            .get("metricsEvent")
            .or_else(|| payload.get("usageEvent"))
            .or_else(|| payload.get("usage"))
            .or_else(|| payload.get("metadataEvent").and_then(|m| m.get("usage")))
            .unwrap_or(payload);
        let read = |keys: &[&str]| {
            keys.iter()
                .find_map(|k| metrics.get(*k).and_then(Value::as_u64))
        };
        let input = read(&["inputTokens", "prompt_tokens"]).unwrap_or(0);
        let output = read(&["outputTokens", "completion_tokens"]).unwrap_or(0);
        if input > 0 || output > 0 {
            self.reported_usage = Some((saturate(input), saturate(output)));
        }
        // Spellings as OmniRoute reads them (Bedrock, camelCase, Anthropic).
        // Kept even without totals: the estimate fills input/output later.
        if let Some(n) = read(&[
            "cacheReadInputTokens",
            "cacheReadTokens",
            "cache_read_input_tokens",
        ])
        .filter(|n| *n > 0)
        {
            self.cache_read = Some(saturate(n));
        }
        if let Some(n) = read(&[
            "cacheWriteInputTokens",
            "cacheCreationTokens",
            "cache_creation_input_tokens",
        ])
        .filter(|n| *n > 0)
        {
            self.cache_write = Some(saturate(n));
        }
    }

    /// Reported counts if Kiro sent any; otherwise `OmniRoute`'s `ensureKiroUsage`
    /// estimate: output ≈ chars/4, total ≈ context% × window, input = total − output.
    fn usage(&self) -> Option<(UsageCount, UsageCount)> {
        if let Some((input, output)) = self.reported_usage {
            return Some((UsageCount::Reported(input), UsageCount::Reported(output)));
        }
        let output = if self.output_chars > 0 {
            (self.output_chars / 4).max(1)
        } else {
            0
        };
        #[allow(clippy::cast_precision_loss)] // DEFAULT_MAX_INPUT_TOKENS = 200_000 fits exactly in f64
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        // percentage 0–100, result always non-negative and fits u64
        let total = (self.context_usage_pct * DEFAULT_MAX_INPUT_TOKENS as f64 / 100.0) as u64;
        if total == 0 && output == 0 {
            return None;
        }
        let input = total.saturating_sub(output);
        Some((
            UsageCount::Estimated(saturate(input)),
            UsageCount::Estimated(saturate(output)),
        ))
    }

    /// Stream-safe `<thinking>…</thinking>` splitter (ported from `OmniRoute`'s
    /// `kiroThinking.ts`). Tags can span chunk boundaries: partial tag prefixes
    /// are held in `self.thinking_pending` and completed on the next call.
    fn split_thinking(&mut self, raw: &str, out: &mut Vec<ConversationEvent>) {
        const PARTIAL_MAX: usize = 11; // len("</thinking>")
        let mut text = if self.thinking_pending.is_empty() {
            raw.to_owned()
        } else {
            let mut s = self.thinking_pending.clone();
            s.push_str(raw);
            self.thinking_pending.clear();
            s
        };

        loop {
            let target = if self.thinking_in_block {
                "</thinking>"
            } else {
                "<thinking>"
            };
            match text.find(target) {
                None => {
                    // Hold back a tail that could be the start of `target`.
                    // Only at char boundaries to avoid slicing into multi-byte chars.
                    let hold_from = (text.len().saturating_sub(PARTIAL_MAX)..text.len())
                        .filter(|&i| text.is_char_boundary(i))
                        .find(|&i| {
                            let tail = &text[i..];
                            !tail.is_empty() && target.starts_with(tail)
                        })
                        .unwrap_or(text.len());
                    let flushable = &text[..hold_from];
                    if !flushable.is_empty() {
                        self.emit(flushable, out);
                    }
                    self.thinking_pending.clear();
                    self.thinking_pending.push_str(&text[hold_from..]);
                    return;
                }
                Some(idx) => {
                    let before = &text[..idx];
                    if !before.is_empty() {
                        self.emit(before, out);
                    }
                    self.thinking_in_block = !self.thinking_in_block;
                    text = text[idx + target.len()..].to_owned();
                }
            }
        }
    }

    fn emit(&mut self, text: &str, out: &mut Vec<ConversationEvent>) {
        if self.thinking_in_block {
            out.push(ConversationEvent::ReasoningDelta {
                delta: text.to_owned(),
                index: 0,
            });
        } else {
            self.output_chars += text.chars().count() as u64;
            out.push(ConversationEvent::OutputDelta {
                delta: text.to_owned(),
                index: 0,
            });
        }
    }

    /// Drain pending at end of stream; routes leftover partial tags to
    /// whichever channel is currently open (mirrors `OmniRoute` `flushPendingThinking`).
    pub fn flush_thinking(&mut self, out: &mut Vec<ConversationEvent>) {
        if !self.thinking_pending.is_empty() {
            let leftover = std::mem::take(&mut self.thinking_pending);
            self.emit(&leftover, out);
        }
    }
}

fn parse_payload(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).unwrap_or(Value::Null)
}

fn str_field<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

/// Error text from either shape: the Kiro plane sends `error_message`, while AWS
/// exception frames send `message`.
fn error_message(payload: &Value) -> Option<&str> {
    [
        "error_message",
        "errorMessage",
        "message",
        "Message",
        "reason",
    ]
    .iter()
    .find_map(|k| str_field(payload, k))
}

/// Error code from a `:message-type: error` payload, which the Kiro data plane
/// uses instead of the AWS `:exception-type` header.
fn error_code(payload: &Value) -> Option<&str> {
    ["error_code", "errorCode"]
        .iter()
        .find_map(|k| str_field(payload, k))
}

/// HTTP status for an upstream failure, covering both the AWS exception names and
/// the Kiro plane's own error codes.
fn exception_status(kind: &str) -> u16 {
    match kind {
        // AWS exception types.
        "ThrottlingException" | "ServiceQuotaExceededException" => 429,
        "ValidationException" => 400,
        "AccessDeniedException" => 403,
        "ResourceNotFoundException" => 404,
        // Kiro data-plane error codes.
        "RATE_LIMIT_EXCEEDED" => 429,
        // Quota is exhausted until the next cycle, so it is not a retryable 429.
        "MONTHLY_REQUEST_COUNT" => 402,
        "CONTENT_LENGTH_EXCEEDS_THRESHOLD" | "BAD_REQUEST" => 400,
        "INVALID_BEARER_TOKEN" => 401,
        _ => 502,
    }
}

fn saturate(n: u64) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}
