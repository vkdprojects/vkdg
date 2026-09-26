use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use vkdg_core::{ConnectionId, ExcludedCandidate};

// ── Newtypes ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RouteId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ConnectionWeight(pub u32);

// ── Plugin hooks ──────────────────────────────────────────────────────────────

/// All fields are plugin IDs; all default to empty.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PluginHooks {
    pub pre_auth: Vec<String>,
    pub auth: Vec<String>,
    pub rate_limit: Vec<String>,
    pub pre_dispatch: Vec<String>,
    pub on_event: Vec<String>,
    pub on_finish: Vec<String>,
}

// ── Strategy kind ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrategyKind {
    RoundRobin,
    Weighted,
    LowestLatency,
    PowerOfTwoChoices,
    FallbackChain,
    LastKnownGood,
    /// Multi-factor scored strategy.
    /// mode_pack: "ship-fast" | "cost-saver" | "quality-first" | "offline-friendly" | "balanced"
    Scored { mode_pack: String },
}

// ── Route config ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteConfig {
    pub id: RouteId,
    /// Glob patterns — `*` suffix = prefix match, otherwise exact.
    pub match_models: Vec<String>,
    pub strategy: StrategyKind,
    pub targets: Vec<ConnectionId>,
    pub plugin_hooks: PluginHooks,
}

// ── Eligibility filter ────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct EligibilityFilter {
    pub excluded_connections: Vec<ConnectionId>,
    pub reason_map: HashMap<ConnectionId, String>,
}

impl EligibilityFilter {
    pub fn is_excluded(&self, id: &ConnectionId) -> bool {
        self.excluded_connections.contains(id)
    }
}

// ── Routing hints ─────────────────────────────────────────────────────────────

/// Optional live signals to improve routing decisions.
/// Populated by the gateway from QuotaTracker and latency measurements.
/// All maps are keyed by ConnectionId; absent entries use scorer defaults.
#[derive(Debug, Default, Clone)]
pub struct RoutingHints {
    /// quota_headroom per connection: 0.0 = exhausted, 1.0 = full
    pub quota_headroom: HashMap<ConnectionId, f32>,
    /// p50 latency per connection in milliseconds
    pub latency_p50_ms: HashMap<ConnectionId, u32>,
    /// Request-level mode pack override (from X-VKDG-Mode header or combo)
    pub mode_pack: Option<String>,
}

// ── Route result ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct RouteResult {
    pub connection_id: ConnectionId,
    pub route_id: RouteId,
    pub excluded: Vec<ExcludedCandidate>,
}
