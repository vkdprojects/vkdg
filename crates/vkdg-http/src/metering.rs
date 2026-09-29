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

/// Incremental usage parser over response bytes, SSE or JSON.
#[derive(Debug, Default)]
pub struct UsageMeter {
    /// Bytes of an incomplete trailing line.
    partial: Vec<u8>,
    /// Whole non-SSE body, parsed once at the end.
    body: Vec<u8>,
    sse: Option<bool>,
    usage: TokenUsage,
}

/// Non-streaming bodies larger than this are not metered from their JSON: the
/// usage block would be at the top level of a body this size only for giant
/// outputs, and buffering them twice is not worth it.
const MAX_JSON_BODY: usize = 8 * 1024 * 1024;

impl UsageMeter {
    pub fn feed(&mut self, chunk: &[u8]) {
        let is_sse = *self.sse.get_or_insert_with(|| looks_like_sse(chunk));
        if !is_sse {
            if self.body.len() + chunk.len() <= MAX_JSON_BODY {
                self.body.extend_from_slice(chunk);
            }
            return;
        }
        self.partial.extend_from_slice(chunk);
        while let Some(pos) = self.partial.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.partial.drain(..=pos).collect();
            self.observe_line(&line);
        }
    }

    /// Usage seen so far, including a trailing line or JSON body not yet
    /// terminated. Safe to call more than once.
    pub fn finish(&mut self) -> TokenUsage {
        if self.sse == Some(true) {
            let rest = std::mem::take(&mut self.partial);
            self.observe_line(&rest);
        } else if let Ok(v) = serde_json::from_slice::<Value>(&self.body) {
            self.observe(&v);
        }
        self.usage
    }

    fn observe_line(&mut self, line: &[u8]) {
        let Some(data) = line.strip_prefix(b"data:") else {
            return;
        };
        if let Ok(v) = serde_json::from_slice::<Value>(data.trim_ascii()) {
            self.observe(&v);
        }
    }

    fn observe(&mut self, v: &Value) {
        // Anthropic `message_start` nests usage under `message`.
        for usage in [v.get("usage"), v.pointer("/message/usage")]
            .into_iter()
            .flatten()
        {
            let n = |k: &str| usage.get(k).and_then(Value::as_u64).unwrap_or(0);
            let (read, write) = (
                n("cache_read_input_tokens"),
                n("cache_creation_input_tokens"),
            );
            let input = n("input_tokens").max(n("prompt_tokens")) + read + write;
            let output = n("output_tokens").max(n("completion_tokens"));
            self.usage.input = self.usage.input.max(input);
            self.usage.output = self.usage.output.max(output);
            self.usage.cache_read = self.usage.cache_read.max(read);
            self.usage.cache_write = self.usage.cache_write.max(write);
        }
    }
}

fn looks_like_sse(first: &[u8]) -> bool {
    let t = first.trim_ascii_start();
    t.starts_with(b"event:") || t.starts_with(b"data:") || t.starts_with(b":")
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
