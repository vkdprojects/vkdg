//! Context compression policy for conversation requests.
//! Applied as a pre_dispatch hook when configured on a route.
//! Never modifies context without explicit route consent.

pub mod caveman;
pub mod metrics;
pub mod rtk;
pub mod stacked;
pub mod styles;
pub mod truncate;

pub use caveman::CavemanCompressor;
pub use metrics::CompressionMetrics;
pub use rtk::RtkCompressor;
pub use stacked::StackedCompressor;
pub use styles::OutputStyle;
pub use truncate::{TruncateCompressor, TruncatePolicy, TruncateResult};

use vkdg_operations::ConversationRequest;

/// Native Rust compressor interface — mirrors the WIT compressor-plugin interface.
/// Native implementations (Caveman, RTK) implement this trait directly.
/// WASM plugins implement the WIT interface; the host adapts via a shim.
pub trait Compressor: Send + Sync {
    fn name(&self) -> &str;

    /// Estimate tokens before compressing (cheap, used for threshold check).
    fn estimate_tokens(&self, req: &ConversationRequest) -> u32;

    /// Compress the request. Returns the modified request + metrics.
    /// Returns Err if this compressor cannot handle this request type.
    fn compress(
        &self,
        req: ConversationRequest,
        budget: u32,
    ) -> Result<(ConversationRequest, CompressionMetrics), CompressionError>;
}

#[derive(Debug, thiserror::Error)]
pub enum CompressionError {
    #[error("budget exhausted: required {required}, available {available}")]
    BudgetExhausted { required: u32, available: u32 },
    #[error("not applicable for this request type")]
    NotApplicable,
    #[error("strategy unavailable: {0}")]
    StrategyUnavailable(String),
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "strategy", rename_all = "snake_case")]
pub enum CompressionStrategy {
    /// Keep only the last N tokens of messages (approximate).
    Truncate { max_tokens: u32 },
    /// Summarize older messages via a model call (Phase E; stub for now).
    ModelSummarize { model: String, budget_tokens: u32 },
}

/// Apply the configured compression strategy to a ConversationRequest.
/// Returns the (possibly modified) request and metrics about what changed.
pub fn apply(
    strategy: &CompressionStrategy,
    req: ConversationRequest,
    budget: u32,
) -> Result<(ConversationRequest, CompressionMetrics), CompressionError> {
    match strategy {
        CompressionStrategy::Truncate { max_tokens } => truncate::apply(req, *max_tokens, budget),
        CompressionStrategy::ModelSummarize {
            model,
            budget_tokens: _,
        } => Err(CompressionError::StrategyUnavailable(format!(
            "model_summarize via {model} not yet implemented (Phase E)"
        ))),
    }
}
