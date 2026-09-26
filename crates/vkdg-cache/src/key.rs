//! Cache key derivation.
//!
//! The key is SHA-256 of: model + sorted(messages as JSON).
//! Temperature and max_tokens are intentionally excluded -- same semantic
//! query with different params still hits the cache (the caller controls bypass).

use sha2::{Digest, Sha256};
use vkdg_operations::ConversationRequest;

/// Compute a deterministic hex cache key for a conversation request.
/// Plausible wrong impl: including temperature/max_tokens causes false misses
/// on retry with slightly different params.
pub fn cache_key(model: &str, req: &ConversationRequest) -> String {
    let mut hasher = Sha256::new();
    hasher.update(model.as_bytes());
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
        assert_eq!(cache_key(model, &req1), cache_key(model, &req2));
    }

    // Plausible wrong impl: different messages produce the same key (collision).
    #[test]
    fn different_messages_different_key() {
        let model = "claude-3-5-haiku-20241022";
        assert_ne!(
            cache_key(model, &make_req("hello")),
            cache_key(model, &make_req("goodbye"))
        );
    }

    // Plausible wrong impl: model name ignored in key, two models share a cached response.
    #[test]
    fn different_models_different_key() {
        let req = make_req("hello");
        assert_ne!(
            cache_key("claude-3-5-haiku-20241022", &req),
            cache_key("gpt-4o", &req)
        );
    }
}
