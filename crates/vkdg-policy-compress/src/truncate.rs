use vkdg_operations::{ConversationRequest, Message, MessageContent, ContentBlock, Role};
use crate::metrics::CompressionMetrics;
use crate::CompressionError;

/// Truncation policy: remove oldest messages until estimated token count <= max_tokens.
/// System messages are preserved. Oldest non-system messages are dropped first.
pub struct TruncatePolicy {
    pub max_tokens: u32,
}

pub struct TruncateResult {
    pub request: ConversationRequest,
    pub metrics: CompressionMetrics,
}

/// `Compressor` impl wrapping the truncation strategy.
pub struct TruncateCompressor {
    pub max_tokens: u32,
}

impl crate::Compressor for TruncateCompressor {
    fn name(&self) -> &str { "truncate" }

    fn estimate_tokens(&self, req: &ConversationRequest) -> u32 {
        estimated_tokens(&req.messages)
    }

    fn compress(
        &self,
        req: ConversationRequest,
        budget: u32,
    ) -> Result<(ConversationRequest, crate::CompressionMetrics), crate::CompressionError> {
        apply(req, self.max_tokens, budget)
    }
}

/// Apply truncation to a request.
/// Estimate tokens by character count / 4 (rough approximation).
/// Remove oldest non-system messages first.
pub fn apply(
    mut req: ConversationRequest,
    max_tokens: u32,
    _budget: u32,
) -> Result<(ConversationRequest, CompressionMetrics), CompressionError> {
    let original_count = req.messages.len();
    let original_estimated_tokens = estimated_tokens(&req.messages);

    if original_estimated_tokens <= max_tokens {
        return Ok((req, CompressionMetrics {
            original_message_count: original_count,
            compressed_message_count: original_count,
            estimated_tokens_removed: 0,
            strategy: "truncate".into(),
            lossless: true,
        }));
    }

    // Preserve system messages always; drop oldest non-system first.
    let mut kept: Vec<Message> = req.messages.iter()
        .filter(|m| matches!(m.role, Role::System))
        .cloned()
        .collect();
    let non_system: Vec<Message> = req.messages.into_iter()
        .filter(|m| !matches!(m.role, Role::System))
        .collect();

    // Add from newest backwards until token budget is filled.
    let mut tokens_so_far = estimated_tokens(&kept);
    let mut added: Vec<Message> = Vec::new();
    for msg in non_system.into_iter().rev() {
        let t = estimated_tokens(std::slice::from_ref(&msg));
        if tokens_so_far + t <= max_tokens {
            tokens_so_far += t;
            added.push(msg);
        }
        // else: dropped (truncated)
    }
    added.reverse();
    kept.extend(added);

    let compressed_count = kept.len();
    let removed_tokens = original_estimated_tokens.saturating_sub(tokens_so_far);

    req.messages = kept;
    Ok((req, CompressionMetrics {
        original_message_count: original_count,
        compressed_message_count: compressed_count,
        estimated_tokens_removed: removed_tokens,
        strategy: "truncate".into(),
        lossless: original_count == compressed_count,
    }))
}

/// Rough token estimate: chars / 4.
fn estimated_tokens(messages: &[Message]) -> u32 {
    messages.iter().map(|m| {
        match &m.content {
            MessageContent::Text(s) => (s.len() as u32).saturating_div(4),
            MessageContent::Blocks(blocks) => blocks.iter().map(|b| {
                match b {
                    ContentBlock::Text { text } => (text.len() as u32).saturating_div(4),
                    _ => 50,
                }
            }).sum(),
        }
    }).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use vkdg_core::CapabilitySet;
    use crate::{CompressionStrategy, apply as policy_apply, CompressionError};

    fn make_req(messages: Vec<Message>) -> ConversationRequest {
        ConversationRequest {
            messages,
            tools: vec![],
            max_tokens: None,
            temperature: None,
            stream: false,
            system: None,
            required_capabilities: CapabilitySet::default(),
        }
    }

    fn text_msg(role: Role, content: &str) -> Message {
        Message { role, content: MessageContent::Text(content.into()) }
    }

    #[test]
    fn truncate_noop_when_under_budget() {
        // 10 short messages, large budget → lossless=true, count unchanged.
        // Defeat: truncating even when not needed.
        let messages: Vec<Message> = (0..10)
            .map(|i| text_msg(Role::User, &format!("msg {i}")))
            .collect();
        let req = make_req(messages);
        let (out, metrics) = apply(req, 10_000, 10_000).unwrap();
        assert!(metrics.lossless, "should be lossless when under budget");
        assert_eq!(metrics.compressed_message_count, 10);
        assert_eq!(out.messages.len(), 10);
    }

    #[test]
    fn truncate_removes_oldest_first() {
        // Three ~100-token messages (400 chars / 4 = 100 tokens each).
        // Budget = 110 → only the newest (C) fits.
        // Defeat: removing newest messages instead of oldest.
        let a = "A".repeat(400);
        let b = "B".repeat(400);
        let c = "C".repeat(400);
        let messages = vec![
            text_msg(Role::User, &a),
            text_msg(Role::Assistant, &b),
            text_msg(Role::User, &c),
        ];
        let req = make_req(messages);
        let (out, metrics) = apply(req, 110, 110).unwrap();
        assert!(!metrics.lossless);
        assert_eq!(out.messages.len(), 1, "only newest should remain");
        if let MessageContent::Text(s) = &out.messages[0].content {
            assert!(s.starts_with('C'), "newest (C) should be kept, got: {s:.10}");
        } else {
            panic!("unexpected content type");
        }
    }

    #[test]
    fn truncate_preserves_system_messages() {
        // System + 5 user messages with tight budget → system always present.
        // Defeat: system message removed along with user messages.
        let system = text_msg(Role::System, &"S".repeat(4)); // ~1 token
        let users: Vec<Message> = (0..5)
            .map(|i| text_msg(Role::User, &"U".repeat(400 * (i + 1))))
            .collect();
        let mut messages = vec![system];
        messages.extend(users);
        let req = make_req(messages);
        // Budget so tight only system fits.
        let (out, _metrics) = apply(req, 5, 5).unwrap();
        let has_system = out.messages.iter().any(|m| matches!(m.role, Role::System));
        assert!(has_system, "system message must always be preserved");
    }

    #[test]
    fn truncate_metrics_count_correct() {
        // 5 messages ~100 tokens each. Budget 220 → fits 2 newest.
        // Defeat: metrics reporting wrong counts.
        let messages: Vec<Message> = (0..5)
            .map(|_| text_msg(Role::User, &"X".repeat(400)))
            .collect();
        let req = make_req(messages);
        let (_out, metrics) = apply(req, 220, 220).unwrap();
        assert_eq!(metrics.original_message_count, 5);
        assert_eq!(metrics.compressed_message_count, 2);
        assert!(!metrics.lossless);
        assert!(metrics.estimated_tokens_removed > 0);
    }

    #[test]
    fn model_summarize_returns_unavailable() {
        // ModelSummarize is Phase E stub; must return StrategyUnavailable.
        // Defeat: stub returning Ok() instead of Err.
        let req = make_req(vec![text_msg(Role::User, "hello")]);
        let strategy = CompressionStrategy::ModelSummarize {
            model: "gpt-4".into(),
            budget_tokens: 1000,
        };
        let result = policy_apply(&strategy, req, 1000);
        assert!(
            matches!(result, Err(CompressionError::StrategyUnavailable(_))),
            "model_summarize should return StrategyUnavailable"
        );
    }
}
