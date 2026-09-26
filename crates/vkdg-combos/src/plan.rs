use serde::{Deserialize, Serialize};
use vkdg_core::ConnectionId;
use vkdg_routing::StrategyKind;

/// A named routing plan.
///
/// Combo names shadow model IDs in resolution order:
///   combo exact match -> combo/<prefix> glob -> model-ID glob -> bare model ID
///
/// Example config:
/// ```toml
/// [[combos]]
/// id = "coding-fast"
/// match_patterns = ["coding-fast", "code:*"]
/// strategy = "fallback_chain"
/// targets = ["claude-haiku", "gpt-4o-mini"]
/// compression = "caveman"
/// cache_ttl_secs = 300
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Combo {
    pub id: String,
    /// Patterns that trigger this combo (exact names or globs)
    pub match_patterns: Vec<String>,
    pub strategy: StrategyKind,
    pub targets: Vec<ConnectionId>,
    pub compression: Option<CompressionPolicy>,
    pub cache: Option<CachePolicy>,
    pub budget: Option<BudgetPolicy>,
    pub mode_pack: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressionPolicy {
    /// Plugin ID: "caveman" | "rtk" | "truncate" | custom
    pub plugin_id: String,
    /// Auto-trigger when estimated tokens exceed this threshold
    pub auto_trigger_tokens: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachePolicy {
    /// Plugin ID: "sqlite-exact" | "redis-exact" | "redis-semantic" | custom
    pub plugin_id: String,
    pub ttl_secs: Option<u32>,
    /// Bypass cache for multi-turn conversations (>1 assistant turn in history)
    pub bypass_multiturn: bool,
    /// Bypass cache when messages contain tool_calls or tool results
    pub bypass_tool_calls: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetPolicy {
    /// Maximum cost in microdollars for a single request
    pub max_cost_microdollars: Option<u64>,
    /// Behavior when budget is exceeded: "strict" (reject) | "cheapest" (downgrade)
    pub overflow: String,
}
