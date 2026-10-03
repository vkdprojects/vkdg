//! Failures a decoder reports as `ConversationEvent::Failed`.
//!
//! Messages are fixed strings or a capped copy of the provider's own message:
//! the payload of a malformed event may carry user content and is never echoed.

use vkdg_core::VkdgError;
use vkdg_operations::ConversationEvent;

/// Most bytes of arguments one tool call may stream before the stream fails.
pub const DEFAULT_MAX_TOOL_ARGUMENT_BYTES: usize = 8 << 20;

/// Longest provider-supplied message kept, in characters.
const MAX_PROVIDER_MESSAGE_CHARS: usize = 300;

pub(super) const MALFORMED_ANTHROPIC: &str = "upstream sent a malformed anthropic stream event";
pub(super) const MALFORMED_OPENAI: &str = "upstream sent a malformed openai stream event";
pub(super) const EVENT_TOO_LARGE: &str = "upstream sent an SSE event larger than the gateway limit";
pub(super) const TOOL_ARGUMENTS_TOO_LARGE: &str =
    "upstream tool call arguments exceeded the gateway limit";
pub(super) const EMPTY_STREAM: &str = "upstream returned an empty stream";
pub(super) const TRUNCATED_STREAM: &str = "upstream stream ended before completion";

pub(super) fn failed(code: u16, message: &str) -> ConversationEvent {
    ConversationEvent::Failed {
        error: VkdgError::UpstreamError {
            code,
            message: message.to_owned(),
            retry_after: None,
        },
    }
}

/// `message` cut to the diagnostics limit (characters, not bytes).
pub(super) fn capped(message: &str) -> String {
    message.chars().take(MAX_PROVIDER_MESSAGE_CHARS).collect()
}
