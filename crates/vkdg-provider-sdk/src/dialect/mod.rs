//! Upstream dialect decoding: provider bytes to canonical conversation events.
//!
//! One decoder per dialect and transport. Streams ([`AnthropicSseDecoder`],
//! [`OpenAiSseDecoder`], [`ResponsesSseDecoder`]) and complete bodies
//! ([`decode_anthropic_json`], [`decode_openai_json`]) share the event vocabulary,
//! so a client dialect is written once, by the encoders in `vkdg-operations`.
//! [`sse_decoder_for`] and [`json_decoder_for`] pick the decoder for a [`WireFormat`].
//! Responses providers select [`ResponsesSseDecoder`] through their stream hook,
//! independently of the client wire format.

mod anthropic_json;
mod anthropic_sse;
mod openai_json;
mod openai_sse;
mod responses_sse;
mod sse_frame;
mod token_counts;
mod upstream_failure;

use vkdg_core::VkdgError;
use vkdg_operations::{ConversationEvent, WireFormat};

use crate::ConversationStreamDecoder;

pub use anthropic_json::decode_anthropic_json;
pub use anthropic_sse::AnthropicSseDecoder;
pub use openai_json::decode_openai_json;
pub use openai_sse::OpenAiSseDecoder;
pub use responses_sse::ResponsesSseDecoder;
pub use sse_frame::{FrameError, SseFrame, SseFramer, DEFAULT_MAX_EVENT_BYTES};
pub use upstream_failure::DEFAULT_MAX_TOOL_ARGUMENT_BYTES;

/// The streaming decoder for a provider that speaks `format`.
pub fn sse_decoder_for(format: WireFormat) -> Box<dyn ConversationStreamDecoder> {
    match format {
        WireFormat::AnthropicMessages => Box::new(AnthropicSseDecoder::new()),
        WireFormat::OpenAiChat => Box::new(OpenAiSseDecoder::new()),
    }
}

/// Decodes one complete response body to events.
pub type JsonDecoder = fn(&[u8]) -> Result<Vec<ConversationEvent>, VkdgError>;

/// The complete-body decoder for a provider that speaks `format`.
pub fn json_decoder_for(format: WireFormat) -> JsonDecoder {
    match format {
        WireFormat::AnthropicMessages => decode_anthropic_json,
        WireFormat::OpenAiChat => decode_openai_json,
    }
}

#[cfg(test)]
mod anthropic_sse_tests;
#[cfg(test)]
mod event_text;
#[cfg(test)]
mod json_response_tests;
#[cfg(test)]
mod openai_sse_tests;
#[cfg(test)]
mod responses_sse_tests;
#[cfg(test)]
mod sse_frame_tests;
