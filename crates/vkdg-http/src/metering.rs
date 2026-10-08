//! Token usage metering on the way out to the client.
//!
//! The meter reads the response the client actually receives, in the client's
//! dialect, so it covers every path (passthrough SSE, decoded provider streams,
//! JSON bodies, cache hits) with one parser and never depends on how a
//! provider was reached.
//!
//! Anthropic reports `usage.input_tokens` in `message_start` and a growing
//! `usage.output_tokens` in `message_delta`; `OpenAI` reports
//! `usage.prompt_tokens` / `completion_tokens` in the body or the final stream
//! chunk. Counts only grow, so the meter keeps the largest value seen per field.

use serde_json::Value;

/// Tokens a response reported. Zero means "not reported".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TokenUsage {
    /// All input tokens, cached ones included.
    pub input: u64,
    pub output: u64,
    /// Part of `input` read from / written to the prompt cache (Anthropic).
    pub cache_read: u64,
    pub cache_write: u64,
}

impl TokenUsage {
    pub fn total(&self) -> u64 {
        self.input + self.output
    }

    pub fn billed(&self) -> vkdg_core::pricing::BilledTokens {
        vkdg_core::pricing::BilledTokens {
            input: self.input,
            cache_read: self.cache_read,
            cache_write: self.cache_write,
            output: self.output,
        }
    }
}

/// Tracks reported usage and logical completion without collecting SSE bodies.
#[derive(Debug, Default)]
pub struct UsageMeter {
    framer: vkdg_provider_sdk::SseFramer,
    json_body: Vec<u8>,
    sse: Option<bool>,
    state: MeterState,
}

#[derive(Debug, Default)]
struct MeterState {
    usage: TokenUsage,
    stop_reason: Option<String>,
    context_usage_pct: Option<f64>,
    end: crate::sse::StreamEnd,
}

const MAX_JSON_BODY: usize = 8 * 1024 * 1024;

impl UsageMeter {
    pub fn feed(&mut self, chunk: &[u8]) {
        if self.sse.is_none() {
            let Some(first) = chunk.iter().find(|b| !b.is_ascii_whitespace()) else {
                return;
            };
            self.sse = Some(!matches!(first, b'{' | b'['));
        }
        if self.sse == Some(false) {
            if self.json_body.len() + chunk.len() <= MAX_JSON_BODY {
                self.json_body.extend_from_slice(chunk);
            }
            return;
        }
        let state = &mut self.state;
        self.framer.push(chunk, |frame| match frame {
            Ok(frame) => {
                let value = serde_json::from_str::<Value>(&frame.data).ok();
                state.end.observe(frame.event, &frame.data, value.as_ref());
                if let Some(value) = value.as_ref() {
                    state.observe(value);
                }
            }
            Err(_) => state.end = crate::sse::StreamEnd::Failed,
        });
    }

    /// Flush reported usage, but never treat an unterminated SSE frame as a
    /// terminal event. A client can disconnect in the middle of `[DONE]`.
    pub fn finish(&mut self) -> TokenUsage {
        if self.sse == Some(true) {
            let state = &mut self.state;
            self.framer.finish(|frame| {
                if let Ok(frame) = frame {
                    if let Ok(value) = serde_json::from_str::<Value>(&frame.data) {
                        state.observe(&value);
                    }
                }
            });
        } else if let Ok(value) = serde_json::from_slice::<Value>(&self.json_body) {
            self.state.observe(&value);
        }
        self.state.usage
    }

    pub fn stop_reason(&self) -> Option<String> {
        self.state.stop_reason.clone()
    }

    pub fn context_usage_pct(&self) -> Option<f64> {
        self.state.context_usage_pct
    }

    pub(crate) fn stream_end(&self) -> crate::sse::StreamEnd {
        self.state.end
    }
}

impl MeterState {
    fn observe(&mut self, value: &Value) {
        let usage = value
            .get("usage")
            .or_else(|| value.get("message").and_then(|m| m.get("usage")))
            .or_else(|| value.get("response").and_then(|r| r.get("usage")));
        if let Some(usage) = usage {
            let read = usage
                .get("cache_read_input_tokens")
                .and_then(Value::as_u64)
                .or_else(|| {
                    usage
                        .pointer("/prompt_tokens_details/cached_tokens")
                        .and_then(Value::as_u64)
                })
                .or_else(|| {
                    usage
                        .pointer("/input_tokens_details/cached_tokens")
                        .and_then(Value::as_u64)
                });
            let write = usage
                .get("cache_creation_input_tokens")
                .and_then(Value::as_u64);
            if let Some(n) = read {
                self.usage.cache_read = n;
            }
            if let Some(n) = write {
                self.usage.cache_write = n;
            }
            if let Some(n) = usage.get("prompt_tokens").and_then(Value::as_u64) {
                self.usage.input = n.saturating_add(self.usage.cache_write);
            } else if let Some(n) = usage.get("input_tokens").and_then(Value::as_u64) {
                self.usage.input = if usage.get("input_tokens_details").is_some() {
                    n
                } else {
                    n.saturating_add(self.usage.cache_read)
                        .saturating_add(self.usage.cache_write)
                };
            }
            if let Some(n) = usage
                .get("output_tokens")
                .or_else(|| usage.get("completion_tokens"))
                .and_then(Value::as_u64)
            {
                self.usage.output = n;
            }
        }
        let stop = value
            .pointer("/delta/stop_reason")
            .and_then(Value::as_str)
            .or_else(|| {
                value
                    .get("choices")
                    .and_then(Value::as_array)
                    .and_then(|choices| {
                        choices
                            .iter()
                            .find_map(|c| c.get("finish_reason").and_then(Value::as_str))
                    })
            })
            .or_else(|| value.get("stop_reason").and_then(Value::as_str));
        if let Some(stop) = stop.filter(|s| !s.is_empty()) {
            self.stop_reason = Some(stop.to_owned());
        }
        if value.get("type").and_then(Value::as_str) == Some("vkdg_context_usage") {
            self.context_usage_pct = value.get("pct").and_then(Value::as_f64);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meter(chunks: &[&str]) -> TokenUsage {
        let mut m = UsageMeter::default();
        for c in chunks {
            m.feed(c.as_bytes());
        }
        m.finish()
    }

    #[test]
    fn anthropic_stream_sums_start_input_and_final_output() {
        let u = meter(&[
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":12,\"cache_read_input_tokens\":100,\"output_tokens\":1}}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\"}\n\n",
            // Split mid-line across chunks.
            "event: message_delta\ndata: {\"type\":\"message_delta\",\"usage\":{\"output_",
            "tokens\":42}}\n\n",
        ]);
        assert_eq!(
            u,
            TokenUsage {
                input: 112,
                output: 42,
                cache_read: 100,
                ..TokenUsage::default()
            }
        );
    }

    #[test]
    fn openai_stream_final_chunk_and_done() {
        let u = meter(&[
            "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":7,\"completion_tokens\":3}}\n\n",
            "data: [DONE]\n\n",
        ]);
        assert_eq!(
            u,
            TokenUsage {
                input: 7,
                output: 3,
                ..TokenUsage::default()
            }
        );
    }

    #[test]
    fn json_bodies_in_both_dialects() {
        assert_eq!(
            meter(&["{\"usage\":{\"input_tokens\":5,", "\"output_tokens\":9}}"]),
            TokenUsage {
                input: 5,
                output: 9,
                ..TokenUsage::default()
            }
        );
        assert_eq!(
            meter(&["{\"usage\":{\"prompt_tokens\":2,\"completion_tokens\":4}}"]),
            TokenUsage {
                input: 2,
                output: 4,
                ..TokenUsage::default()
            }
        );
    }

    // A client that disconnects mid-stream is still charged for what it got.
    #[test]
    fn partial_stream_reports_usage_seen_so_far() {
        let mut m = UsageMeter::default();
        m.feed(
            b"data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":30}}}\n",
        );
        assert_eq!(
            m.finish(),
            TokenUsage {
                input: 30,
                output: 0,
                ..TokenUsage::default()
            }
        );
    }

    #[test]
    fn unrelated_or_broken_bodies_report_zero() {
        assert_eq!(
            meter(&["{\"error\":{\"message\":\"x\"}}"]),
            TokenUsage::default()
        );
        assert_eq!(meter(&["not json"]), TokenUsage::default());
        assert_eq!(meter(&[]), TokenUsage::default());
    }
}

#[cfg(test)]
mod openai_cache_tests {
    use super::*;

    fn meter(chunks: &[&str]) -> TokenUsage {
        let mut m = UsageMeter::default();
        for c in chunks {
            m.feed(c.as_bytes());
        }
        m.finish()
    }

    // Refutes: treating the OpenAI shape like the Anthropic one. `prompt_tokens`
    // already counts the cached tokens, so adding a cache read on top bills them twice,
    // and ignoring `prompt_tokens_details.cached_tokens` bills them at the full price.
    // Cache creation is outside `prompt_tokens` (the gateway's own extension field).
    #[test]
    fn openai_usage_reads_cached_tokens_without_double_counting() {
        let u = meter(&[
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":112,\"completion_tokens\":3,\"total_tokens\":115,\"prompt_tokens_details\":{\"cached_tokens\":100},\"cache_creation_input_tokens\":20}}\n\n",
            "data: [DONE]\n\n",
        ]);
        assert_eq!(
            u,
            TokenUsage {
                input: 132,
                output: 3,
                cache_read: 100,
                cache_write: 20,
            }
        );
    }

    // Same shape in a non-streaming body.
    #[test]
    fn openai_json_body_reads_cached_tokens() {
        let u = meter(&[
            "{\"usage\":{\"prompt_tokens\":50,\"completion_tokens\":1,\"prompt_tokens_details\":{\"cached_tokens\":40}}}",
        ]);
        assert_eq!(
            u,
            TokenUsage {
                input: 50,
                output: 1,
                cache_read: 40,
                cache_write: 0,
            }
        );
    }
}
