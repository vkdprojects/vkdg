//! Token accounting shared by every encoder.
//!
//! The event model counts the Anthropic way: `input_tokens` excludes cache
//! tokens and the two cache counts are separate. `OpenAI` clients expect
//! `prompt_tokens` to include cached tokens, so that is where the two dialects
//! differ; the conversion lives here and nowhere else.

use serde::Serialize;

use crate::UsageCount;

/// Counts gathered from [`ConversationEvent::Usage`](crate::ConversationEvent::Usage).
///
/// A later event only overwrites the fields it actually reports: `message_start`
/// knows the input, `message_delta` only the output, and an `Unknown` field must
/// never wipe a count the stream already established.
#[derive(Debug, Default, Clone, Copy)]
pub struct UsageTally {
    input: u32,
    output: u32,
    cache_read: u32,
    cache_creation: u32,
    /// Some event reported at least one count.
    known: bool,
}

impl UsageTally {
    pub fn update(
        &mut self,
        input: &UsageCount,
        output: &UsageCount,
        cache_read: &UsageCount,
        cache_creation: &UsageCount,
    ) {
        for (slot, count) in [
            (&mut self.input, input),
            (&mut self.output, output),
            (&mut self.cache_read, cache_read),
            (&mut self.cache_creation, cache_creation),
        ] {
            if !matches!(count, UsageCount::Unknown) {
                *slot = count.value();
                self.known = true;
            }
        }
    }

    pub fn is_known(&self) -> bool {
        self.known
    }

    pub fn to_anthropic(self) -> AnthropicUsage {
        AnthropicUsage {
            input_tokens: self.input,
            output_tokens: self.output,
            // Anthropic omits the cache fields when a provider reports none.
            cache_read_input_tokens: (self.cache_read > 0).then_some(self.cache_read),
            cache_creation_input_tokens: (self.cache_creation > 0).then_some(self.cache_creation),
        }
    }

    /// `prompt_tokens` includes cache reads (what `OpenAI` clients bill) but never
    /// cache creation: Anthropic pads short prompts up to a cache minimum, so
    /// counting creation turned a 2-token prompt into ~2000 billed tokens.
    pub fn to_openai(self) -> OpenAiUsage {
        let prompt = self.input.saturating_add(self.cache_read);
        OpenAiUsage {
            prompt_tokens: prompt,
            completion_tokens: self.output,
            total_tokens: prompt.saturating_add(self.output),
            prompt_tokens_details: (self.cache_read > 0).then_some(PromptTokensDetails {
                cached_tokens: self.cache_read,
            }),
            cache_creation_input_tokens: (self.cache_creation > 0).then_some(self.cache_creation),
        }
    }
}

// Field order is the wire order.

#[derive(Debug, Serialize)]
#[allow(clippy::struct_field_names)] // the wire names are Anthropic's
pub struct AnthropicUsage {
    input_tokens: u32,
    output_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_read_input_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_creation_input_tokens: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct OpenAiUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
    total_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt_tokens_details: Option<PromptTokensDetails>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_creation_input_tokens: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct PromptTokensDetails {
    cached_tokens: u32,
}
