//! Token counts gathered while decoding one upstream response.

use serde_json::Value;
use vkdg_operations::{ConversationEvent, UsageCount};

/// Counts reported so far. A later usage object only replaces the fields it
/// carries: Anthropic's `message_start` knows the input and `message_delta` may
/// report only the output, so the input must survive it.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct TokenCounts {
    pub(super) input: Option<u32>,
    pub(super) output: Option<u32>,
    pub(super) cache_read: Option<u32>,
    pub(super) cache_creation: Option<u32>,
}

fn count_field(object: &Value, key: &str) -> Option<u32> {
    object
        .get(key)
        .and_then(Value::as_u64)
        .map(|n| u32::try_from(n).unwrap_or(u32::MAX))
}

impl TokenCounts {
    /// Merges the Anthropic `usage` object; false when it reported nothing.
    pub(super) fn merge_anthropic(&mut self, usage: &Value) -> bool {
        let fields = [
            (&mut self.input, "input_tokens"),
            (&mut self.output, "output_tokens"),
            (&mut self.cache_read, "cache_read_input_tokens"),
            // The `cache_creation` object (per-TTL breakdown) is not the token count.
            (&mut self.cache_creation, "cache_creation_input_tokens"),
        ];
        let mut any = false;
        for (slot, key) in fields {
            if let Some(n) = count_field(usage, key) {
                *slot = Some(n);
                any = true;
            }
        }
        any
    }

    /// Merges an `OpenAI` `usage` object into event semantics: `prompt_tokens`
    /// includes cached tokens, the event model's `input` does not.
    pub(super) fn merge_openai(&mut self, usage: &Value) -> bool {
        let prompt = count_field(usage, "prompt_tokens");
        let output = count_field(usage, "completion_tokens");
        let cached = usage
            .get("prompt_tokens_details")
            .and_then(|d| count_field(d, "cached_tokens"));
        let created = count_field(usage, "cache_creation_input_tokens");
        if prompt.is_none() && output.is_none() && cached.is_none() && created.is_none() {
            return false;
        }
        if let Some(prompt) = prompt {
            self.input = Some(prompt.saturating_sub(cached.unwrap_or(0)));
        }
        if let Some(output) = output {
            self.output = Some(output);
        }
        if let Some(cached) = cached {
            self.cache_read = Some(cached);
        }
        if let Some(created) = created {
            self.cache_creation = Some(created);
        }
        true
    }

    pub(super) fn event(&self) -> ConversationEvent {
        let count = |n: Option<u32>| n.map_or(UsageCount::Unknown, UsageCount::Reported);
        ConversationEvent::Usage {
            input_tokens: count(self.input),
            output_tokens: count(self.output),
            cache_read_tokens: count(self.cache_read),
            cache_creation_tokens: count(self.cache_creation),
        }
    }
}
