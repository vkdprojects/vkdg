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
// ── ScoredStrategy ────────────────────────────────────────────────────────────

pub struct ScoredStrategy {
    pub mode_pack: String,
}

#[async_trait]
impl Strategy for ScoredStrategy {
    fn name(&self) -> &str {
        "scored"
    }

    async fn select(
        &self,
        candidates: &[ConnectionId],
        _envelope: &RequestEnvelope,
        filter: &EligibilityFilter,
    ) -> Result<ConnectionId> {
        let signals: Vec<scorer::CandidateSignals> = candidates
            .iter()
            .filter(|id| !filter.is_excluded(id))
            .map(|id| scorer::CandidateSignals {
                connection_id: id.0.clone(),
                health: 1.0,
                quota_headroom: None,
                cost_per_ktoken: None,
                latency_p50_ms: None,
                instability_events: 0,
            })
            .collect();

        if signals.is_empty() {
            return Err(VkdgError::NoEligibleConnection);
        }

        let weights = scorer::ScoringWeights::from_mode_pack(&self.mode_pack);
        scorer::rank_candidates(&signals, &weights)
            .into_iter()
            .next()
            .map(|id| ConnectionId(id))
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

        let strategy: Arc<dyn Strategy> = match &route.strategy {
            StrategyKind::RoundRobin => self
                .strategies
                .get("round_robin")
                .cloned()
                .ok_or_else(|| VkdgError::Internal("round_robin not registered".into()))?,
            StrategyKind::FallbackChain => self
                .strategies
                .get("fallback_chain")
                .cloned()
                .ok_or_else(|| VkdgError::Internal("fallback_chain not registered".into()))?,
            StrategyKind::Scored { mode_pack } => {
                Arc::new(ScoredStrategy { mode_pack: mode_pack.clone() })
            }
            // Other strategies fall back to round-robin until implemented.
            _ => self
                .strategies
                .get("round_robin")
                .cloned()
                .ok_or_else(|| VkdgError::Internal("round_robin not registered".into()))?,
        };

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

#[cfg(test)]
mod tests {
    use super::*;
    use vkdg_core::{ApiType, ClientId, RequestId, TenantId};

    fn test_envelope(model: &str) -> RequestEnvelope {
        RequestEnvelope {
            request_id: RequestId::new(),
            client_id: ClientId("test-client".into()),
            tenant_id: TenantId("test-tenant".into()),
            session_key: None,
            api_type: ApiType::AnthropicMessages,
            model_requested: model.to_string(),
            deadline: None,
            mode_pack_override: None,
            compression_override: None,
            cache_bypass: false,
            include_think_tags: false,
        }
    }

    #[tokio::test]
    async fn scored_strategy_selects_single_candidate() {
        let conn = ConnectionId("conn-a".into());
        let route = RouteConfig {
            id: RouteId("r".into()),
            match_models: vec!["claude-*".into()],
            strategy: StrategyKind::Scored { mode_pack: "balanced".into() },
            targets: vec![conn.clone()],
            plugin_hooks: PluginHooks::default(),
        };
        let router = Router::new(vec![route]);
        let envelope = test_envelope("claude-3-5-haiku-20241022");
        let result = router.route(&envelope, &EligibilityFilter::default()).await;
        assert!(result.is_ok(), "scored strategy must select from available candidates");
        assert_eq!(result.unwrap().connection_id, conn);
    }

    #[tokio::test]
    async fn scored_strategy_uses_configured_mode_pack() {
        let route = RouteConfig {
            id: RouteId("r".into()),
            match_models: vec!["gpt-*".into()],
            strategy: StrategyKind::Scored { mode_pack: "ship-fast".into() },
            targets: vec![ConnectionId("fast".into()), ConnectionId("cheap".into())],
            plugin_hooks: PluginHooks::default(),
        };
        let router = Router::new(vec![route]);
        let envelope = test_envelope("gpt-4o");
        let result = router.route(&envelope, &EligibilityFilter::default()).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn scored_strategy_returns_no_eligible_when_all_excluded() {
        let conn = ConnectionId("conn-a".into());
        let route = RouteConfig {
            id: RouteId("r".into()),
            match_models: vec!["claude-*".into()],
            strategy: StrategyKind::Scored { mode_pack: "balanced".into() },
            targets: vec![conn.clone()],
            plugin_hooks: PluginHooks::default(),
        };
        let router = Router::new(vec![route]);
        let envelope = test_envelope("claude-3-opus-20240229");
        let mut filter = EligibilityFilter::default();
        filter.excluded_connections.push(conn);
        let result = router.route(&envelope, &filter).await;
        assert!(matches!(result, Err(VkdgError::NoEligibleConnection)));
    }
}
