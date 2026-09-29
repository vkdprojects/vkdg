use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;

use vkdg_core::{ConnectionId, ExcludedCandidate, RequestEnvelope, Result, VkdgError};

use crate::scored::ScoredStrategy;
use crate::strategy::{
    FallbackChainStrategy, LowestLatencyStrategy, PowerOfTwoChoicesStrategy, RoundRobinStrategy,
    Strategy,
};
use crate::types::{EligibilityFilter, RouteConfig, RouteResult, RoutingHints, StrategyKind};

// ── Router ────────────────────────────────────────────────────────────────────

pub struct Router {
    /// Swapped whole on config reload or combo edit; a request routes against
    /// the snapshot it read, never a half-applied one.
    tables: RwLock<Tables>,
    /// Kept across reloads so round-robin positions do not reset.
    strategies: HashMap<String, Arc<dyn Strategy>>,
}

/// Combo routes (edited at runtime) and config routes (from the file), kept
/// apart so replacing one never drops the other. Combos match first.
#[derive(Default)]
struct Tables {
    combos: Arc<Vec<RouteConfig>>,
    config: Arc<Vec<RouteConfig>>,
    merged: Arc<Vec<RouteConfig>>,
}

impl Tables {
    fn merge(&mut self) {
        self.merged = Arc::new(
            self.combos
                .iter()
                .chain(self.config.iter())
                .cloned()
                .collect(),
        );
    }
}

impl Router {
    pub fn new(routes: Vec<RouteConfig>) -> Self {
        let mut strategies: HashMap<String, Arc<dyn Strategy>> = HashMap::new();
        strategies.insert("round_robin".into(), Arc::new(RoundRobinStrategy::new()));
        strategies.insert("fallback_chain".into(), Arc::new(FallbackChainStrategy));
        strategies.insert("lowest_latency".into(), Arc::new(LowestLatencyStrategy));
        strategies.insert(
            "power_of_two_choices".into(),
            Arc::new(PowerOfTwoChoicesStrategy::new()),
        );
        let mut tables = Tables {
            config: Arc::new(routes),
            ..Tables::default()
        };
        tables.merge();
        Self {
            tables: RwLock::new(tables),
            strategies,
        }
    }

    /// Replace the config route table from a new, already validated snapshot.
    /// Combo routes stay in force.
    pub fn replace_routes(&self, routes: Vec<RouteConfig>) {
        let mut t = self.tables.write();
        t.config = Arc::new(routes);
        t.merge();
    }

    /// Replace the combo routes. They match before config routes, so a combo
    /// named like a model takes that model's traffic. Config routes stay.
    pub fn replace_combo_routes(&self, routes: Vec<RouteConfig>) {
        let mut t = self.tables.write();
        t.combos = Arc::new(routes);
        t.merge();
    }

    /// The route table currently in force: combos first, then config routes.
    pub fn routes(&self) -> Arc<Vec<RouteConfig>> {
        Arc::clone(&self.tables.read().merged)
    }

    pub async fn route(
        &self,
        envelope: &RequestEnvelope,
        filter: &EligibilityFilter,
        hints: &RoutingHints,
    ) -> Result<RouteResult> {
        let routes = self.routes();
        let route = Self::match_route(&routes, envelope).ok_or(VkdgError::NoRouteMatched)?;

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
                hooks: route.plugin_hooks.clone(),
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
                            field: format!("strategy.steps[{i}].connection_id"),
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
                hooks: route.plugin_hooks.clone(),
            });
        }

        let named = |name: &str| {
            self.strategies
                .get(name)
                .cloned()
                .ok_or_else(|| VkdgError::Internal(format!("{name} not registered")))
        };
        let strategy: Arc<dyn Strategy> = match &route.strategy {
            StrategyKind::RoundRobin => named("round_robin")?,
            StrategyKind::FallbackChain => named("fallback_chain")?,
            StrategyKind::LowestLatency => named("lowest_latency")?,
            StrategyKind::PowerOfTwoChoices => named("power_of_two_choices")?,
            StrategyKind::Scored { mode_pack } => Arc::new(ScoredStrategy {
                mode_pack: mode_pack.clone(),
            }),
            // Config validation refuses these, so reaching here is a wiring bug:
            // fail loudly rather than quietly route round-robin.
            other => {
                return Err(VkdgError::ConfigInvalid {
                    field: format!("routes[{}].strategy", route.id.0),
                    message: format!("strategy {other:?} is not implemented"),
                })
            }
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
            hooks: route.plugin_hooks.clone(),
        })
    }

    fn match_route<'r>(
        routes: &'r [RouteConfig],
        envelope: &RequestEnvelope,
    ) -> Option<&'r RouteConfig> {
        let model = &envelope.model_requested;
        routes
            .iter()
            .find(|r| vkdg_core::glob::matches_any(&r.match_models, model))
    }
}
