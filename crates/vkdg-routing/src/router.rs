use std::collections::HashMap;
use std::sync::Arc;

use vkdg_core::{ConnectionId, ExcludedCandidate, RequestEnvelope, Result, VkdgError};

use crate::scored::ScoredStrategy;
use crate::strategy::{FallbackChainStrategy, RoundRobinStrategy, Strategy};
use crate::types::{EligibilityFilter, RouteConfig, RouteResult, RoutingHints, StrategyKind};

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
        hints: &RoutingHints,
    ) -> Result<RouteResult> {
        let route = self
            .match_route(envelope)
            .ok_or(VkdgError::NoEligibleConnection)?;

        // Fusion is handled separately — it builds fusion_targets directly.
        if let StrategyKind::Fusion { max_candidates } = &route.strategy {
            let eligible: Vec<ConnectionId> = route
                .targets
                .iter()
                .filter(|id| !filter.is_excluded(id))
                .cloned()
                .collect();
            if eligible.is_empty() {
                return Err(VkdgError::NoEligibleConnection);
            }
            let max = max_candidates.unwrap_or(eligible.len());
            let fusion_targets: Vec<ConnectionId> = eligible.into_iter().take(max).collect();
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
            return Ok(RouteResult {
                connection_id: fusion_targets[0].clone(),
                route_id: route.id.clone(),
                excluded,
                fusion_targets,
                chain_steps: vec![],
            });
        }
        // PromptChain is handled separately — it builds chain_steps directly.
        if let StrategyKind::PromptChain { steps } = &route.strategy {
            if steps.is_empty() {
                return Err(VkdgError::ConfigInvalid {
                    field: "strategy.steps".into(),
                    message: "PromptChain requires at least one step".into(),
                });
            }
            // Validate that each step's connection_id exists in route targets.
            for (i, step) in steps.iter().enumerate() {
                if let Some(conn_id) = &step.connection_id {
                    if !route.targets.contains(conn_id) {
                        return Err(VkdgError::ConfigInvalid {
                            field: format!("strategy.steps[{}].connection_id", i),
                            message: format!("connection '{}' not in route targets", conn_id.0),
                        });
                    }
                }
            }
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
            // Primary connection is the first step's target (or first route target).
            let primary = steps[0]
                .connection_id
                .clone()
                .or_else(|| route.targets.first().cloned())
                .ok_or(VkdgError::NoEligibleConnection)?;
            return Ok(RouteResult {
                connection_id: primary,
                route_id: route.id.clone(),
                excluded,
                fusion_targets: vec![],
                chain_steps: steps.clone(),
            });
        }

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
            StrategyKind::Scored { mode_pack } => Arc::new(ScoredStrategy {
                mode_pack: mode_pack.clone(),
            }),
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
            .select(&route.targets, envelope, filter, hints)
            .await?;

        Ok(RouteResult {
            connection_id,
            route_id: route.id.clone(),
            excluded,
            fusion_targets: vec![],
            chain_steps: vec![],
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
