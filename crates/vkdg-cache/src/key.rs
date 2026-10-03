//! Cache key derivation.
//!
//! The key is SHA-256 of: model + client dialect + sorted(messages as JSON).
//! Temperature and `max_tokens` are intentionally excluded -- same semantic
//! query with different params still hits the cache (the caller controls bypass).
//! The dialect is part of the key because the cache stores the client-facing
//! body: an Anthropic client and an `OpenAI` client asking the same question get
//! differently shaped answers and must never share an entry.

use sha2::{Digest, Sha256};
use vkdg_core::ApiType;
use vkdg_operations::ConversationRequest;

/// Compute a deterministic hex cache key for a conversation request answered in
/// the dialect of `api_type`.
/// Plausible wrong impl: including `temperature/max_tokens` causes false misses
/// on retry with slightly different params.
pub fn cache_key(model: &str, api_type: &ApiType, req: &ConversationRequest) -> String {
    let mut hasher = Sha256::new();
    hasher.update(model.as_bytes());
    // The NUL fences keep the dialect from blending into the model or messages.
    hasher.update([0]);
    hasher.update(format!("{api_type:?}").as_bytes());
    hasher.update([0]);
    // Serialize messages in stable order (they are ordered by position)
    if let Ok(msgs_json) = serde_json::to_string(&req.messages) {
        hasher.update(msgs_json.as_bytes());
    }
    if let Some(system) = &req.system {
        hasher.update(system.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use vkdg_operations::{CapabilitySet, ConversationRequest, Message, MessageContent, Role};

    fn make_req(msg: &str) -> ConversationRequest {
        ConversationRequest {
            model: "test-model".into(),
            messages: vec![Message {
                role: Role::User,
                content: MessageContent::Text(msg.into()),
            }],
            tools: vec![],
            max_tokens: Some(100),
            temperature: Some(0.7),
            stream: false,
            system: None,
            required_capabilities: CapabilitySet::default(),
            thinking: None,
            ..Default::default()
        }
    }

    // Plausible wrong impl: key changes when temperature changes,
    // causing identical semantic queries to miss the cache.
    #[test]
    fn same_messages_same_key_regardless_of_temperature() {
        let model = "claude-3-5-haiku-20241022";
        let mut req1 = make_req("hello");
        let mut req2 = make_req("hello");
        req1.temperature = Some(0.0);
        req2.temperature = Some(1.0);
        assert_eq!(
            cache_key(model, &ApiType::AnthropicMessages, &req1),
            cache_key(model, &ApiType::AnthropicMessages, &req2)
        );
    }

    // Plausible wrong impl: different messages produce the same key (collision).
    #[test]
    fn different_messages_different_key() {
        let model = "claude-3-5-haiku-20241022";
        assert_ne!(
            cache_key(model, &ApiType::AnthropicMessages, &make_req("hello")),
            cache_key(model, &ApiType::AnthropicMessages, &make_req("goodbye"))
        );
    }

    // Plausible wrong impl: model name ignored in key, two models share a cached response.
    #[test]
    fn different_models_different_key() {
        let req = make_req("hello");
        assert_ne!(
            cache_key(
                "claude-3-5-haiku-20241022",
                &ApiType::AnthropicMessages,
                &req
            ),
            cache_key("gpt-4o", &ApiType::AnthropicMessages, &req)
        );
    }
}

#[cfg(test)]
mod dialect_tests {
    use vkdg_core::ApiType;
    use vkdg_operations::{ConversationRequest, Message, MessageContent, Role};

    use crate::cache_key;

    fn req() -> ConversationRequest {
        ConversationRequest {
            model: "m".into(),
            messages: vec![Message {
                role: Role::User,
                content: MessageContent::Text("hello".into()),
            }],
            ..Default::default()
        }
    }

    // Refutes a key that ignores the client dialect: the response cache stores the
    // client-facing body, so an OpenAI client's hit would serve an Anthropic
    // client's `message` object (and the reverse) for the same prompt.
    #[test]
    fn same_prompt_in_two_client_dialects_never_shares_an_entry() {
        let r = req();
        assert_ne!(
            cache_key("m", &ApiType::AnthropicMessages, &r),
            cache_key("m", &ApiType::OpenAiChatCompletions, &r)
        );
    }

    // Refutes a dialect component that makes the key unstable for one client.
    #[test]
    fn same_dialect_same_prompt_is_stable() {
        let r = req();
        assert_eq!(
            cache_key("m", &ApiType::OpenAiChatCompletions, &r),
            cache_key("m", &ApiType::OpenAiChatCompletions, &r)
        );
    }
}
