//! Tool call ids that survive the round trip.
//!
//! Anthropic accepts only `^[a-zA-Z0-9_-]+$` on the next turn, and an `OpenAI`
//! client cannot answer a call with an empty id. A provider that sends neither
//! (Kiro streams some calls without one) gets a synthesized, unique id.

/// `id` with every character outside `[A-Za-z0-9_-]` replaced by `_`, or `None`
/// when nothing is left to use (empty id).
pub fn sanitize_tool_id(id: &str) -> Option<String> {
    if id.is_empty() {
        return None;
    }
    Some(
        id.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect(),
    )
}

/// Hands out ids for calls the provider did not name. Unique per response:
/// the request id seeds it and a counter separates calls.
#[derive(Debug)]
pub struct ToolIdMint {
    prefix: &'static str,
    seed: String,
    next: u32,
}

impl ToolIdMint {
    pub fn new(prefix: &'static str, request_id: &str) -> Self {
        Self {
            prefix,
            seed: request_id
                .chars()
                .filter(char::is_ascii_alphanumeric)
                .collect(),
            next: 0,
        }
    }

    /// The provider's id, sanitized, or a fresh one when it sent none.
    pub fn id_for(&mut self, provider_id: &str) -> String {
        sanitize_tool_id(provider_id).unwrap_or_else(|| {
            self.next += 1;
            format!("{}{}_{}", self.prefix, self.seed, self.next)
        })
    }
}
