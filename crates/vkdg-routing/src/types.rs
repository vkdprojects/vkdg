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

/// How the previous step's response is injected into the next step's messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InjectMode {
    /// Append the previous response as an assistant message before the last user message.
    /// This creates a natural dialogue: user → assistant (previous output) → user (original).
    AsAssistant,
    /// Append the previous response to the last user message:
    /// `"{original_user_message}\n\nPrevious output:\n{previous_response}"`
    AppendToUser,
    /// Prepend the previous response to the system prompt of this step.
    AsSystem,
}

/// A single step in a PromptChain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainStep {
    /// Connection to use for this step. `None` = auto-route from catalog using the request model.
    pub connection_id: Option<vkdg_core::ConnectionId>,
    /// System prompt override for this step. Replaces (not appends) the request system.
    pub system: Option<String>,
    /// How to inject the previous step's response into this step's input.
    /// Ignored for the first step (there is no previous response).
    pub inject_previous: InjectMode,
}

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
    Scored {
        mode_pack: String,
    },
    /// Fan-out to up to `max_candidates` targets in parallel; return the fastest response.
    /// `None` means fan out to all eligible targets.
    Fusion {
        max_candidates: Option<usize>,
    },
    /// Sequential prompt chain: steps run one after another.
    /// The response of step N is injected into step N+1's messages via `inject_previous`.
    /// If any step fails, the chain aborts immediately and the error is returned to the client.
    PromptChain {
        steps: Vec<ChainStep>,
    },
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
    /// Non-empty only for Fusion routes. Contains all targets to dispatch in parallel.
    /// The pipeline races these and returns the first successful response.
    pub fusion_targets: Vec<ConnectionId>,
    /// Non-empty when strategy is PromptChain; steps to execute sequentially.
    pub chain_steps: Vec<ChainStep>,
}
