//! Context compression policy for conversation requests.
//! Applied as a pre_dispatch hook when configured on a route.
//! Never modifies context without explicit route consent.

pub mod metrics;
pub mod truncate;

pub use metrics::CompressionMetrics;
pub use truncate::{TruncatePolicy, TruncateResult};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "strategy", rename_all = "snake_case")]
pub enum CompressionStrategy {
    /// Keep only the last N tokens of messages (approximate).
    Truncate { max_tokens: u32 },
    /// Summarize older messages via a model call (Phase E; stub for now).
    ModelSummarize { model: String, budget_tokens: u32 },
}

#[derive(Debug, thiserror::Error)]
pub enum CompressionError {
    #[error("budget exhausted: required {required}, available {available}")]
    BudgetExhausted { required: u32, available: u32 },
    #[error("strategy not available: {0}")]
    StrategyUnavailable(String),
}

/// Apply the configured compression strategy to a ConversationRequest.
/// Returns the (possibly modified) request and metrics about what changed.
pub fn apply(
    strategy: &CompressionStrategy,
    req: vkdg_operations::ConversationRequest,
    budget: u32,
) -> Result<(vkdg_operations::ConversationRequest, CompressionMetrics), CompressionError> {
    match strategy {
        CompressionStrategy::Truncate { max_tokens } => {
            truncate::apply(req, *max_tokens, budget)
        }
        CompressionStrategy::ModelSummarize { model, budget_tokens: _ } => {
            Err(CompressionError::StrategyUnavailable(
                format!("model_summarize via {model} not yet implemented (Phase E)"),
            ))
        }
    }
}
