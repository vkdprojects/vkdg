use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use vkdg_core::{ConnectionId, ExcludedCandidate, RequestEnvelope, Result, VkdgError};
pub mod scorer;
pub use scorer::{CandidateSignals, ScoringWeights, rank_candidates};

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

// ── Route result ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct RouteResult {
    pub connection_id: ConnectionId,
    pub route_id: RouteId,
    pub excluded: Vec<ExcludedCandidate>,
}

// ── Strategy trait ────────────────────────────────────────────────────────────

#[async_trait]
pub trait Strategy: Send + Sync {
    fn name(&self) -> &str;
    async fn select(
        &self,
        candidates: &[ConnectionId],
        envelope: &RequestEnvelope,
        filter: &EligibilityFilter,
    ) -> Result<ConnectionId>;
}

// ── RoundRobinStrategy ────────────────────────────────────────────────────────

pub struct RoundRobinStrategy {
    counter: AtomicUsize,
}

impl RoundRobinStrategy {
    pub fn new() -> Self {
        Self { counter: AtomicUsize::new(0) }
    }
}

impl Default for RoundRobinStrategy {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Strategy for RoundRobinStrategy {
    fn name(&self) -> &str {
        "round_robin"
    }

    async fn select(
        &self,
        candidates: &[ConnectionId],
        _envelope: &RequestEnvelope,
        filter: &EligibilityFilter,
    ) -> Result<ConnectionId> {
        let eligible: Vec<&ConnectionId> =
            candidates.iter().filter(|c| !filter.is_excluded(c)).collect();
        if eligible.is_empty() {
            return Err(VkdgError::NoEligibleConnection);
        }
        let idx = self.counter.fetch_add(1, Ordering::Relaxed) % eligible.len();
        Ok(eligible[idx].clone())
    }
}

// ── FallbackChainStrategy ─────────────────────────────────────────────────────

pub struct FallbackChainStrategy;

#[async_trait]
impl Strategy for FallbackChainStrategy {
    fn name(&self) -> &str {
        "fallback_chain"
    }

    async fn select(
        &self,
        candidates: &[ConnectionId],
        _envelope: &RequestEnvelope,
        filter: &EligibilityFilter,
    ) -> Result<ConnectionId> {
        candidates
            .iter()
            .find(|c| !filter.is_excluded(c))
            .cloned()
            .ok_or(VkdgError::NoEligibleConnection)
    }
}

// ── Router ────────────────────────────────────────────────────────────────────

pub struct Router {
    routes: Vec<RouteConfig>,
    strategies: HashMap<String, Arc<dyn Strategy>>,
}

impl Router {
    pub fn new(routes: Vec<RouteConfig>) -> Self {
        let mut strategies: HashMap<String, Arc<dyn Strategy>> = HashMap::new();
        strategies.insert("round_robin".into(), Arc::new(RoundRobinStrategy::new()));
        strategies.insert("fallback_chain".into(), Arc::new(FallbackChainStrategy));
        Self { routes, strategies }
    }

    pub async fn route(
        &self,
        envelope: &RequestEnvelope,
        filter: &EligibilityFilter,
    ) -> Result<RouteResult> {
        let route = self.match_route(envelope).ok_or(VkdgError::NoEligibleConnection)?;

        let strategy_key = match &route.strategy {
            StrategyKind::RoundRobin => "round_robin",
            StrategyKind::FallbackChain => "fallback_chain",
            // All other strategies fall back to round-robin until implemented.
            _ => "round_robin",
        };

        let strategy = self
            .strategies
            .get(strategy_key)
            .ok_or_else(|| VkdgError::Internal(format!("strategy {strategy_key} not registered")))?;

        let excluded: Vec<ExcludedCandidate> = filter
            .excluded_connections
            .iter()
            .map(|id| ExcludedCandidate {
                connection_id: id.clone(),
                reason: filter
                    .reason_map
                    .get(id)
                    .cloned()
                    .unwrap_or_else(|| "filtered".into()),
            })
            .collect();

        let connection_id = strategy
            .select(&route.targets, envelope, filter)
            .await?;

        Ok(RouteResult {
            connection_id,
            route_id: route.id.clone(),
            excluded,
        })
    }

    fn match_route(&self, envelope: &RequestEnvelope) -> Option<&RouteConfig> {
        let model = &envelope.model_requested;
        self.routes.iter().find(|r| {
            r.match_models.iter().any(|pattern| {
                if let Some(prefix) = pattern.strip_suffix('*') {
                    model.starts_with(prefix)
                } else {
                    model == pattern
                }
            })
        })
    }
}
