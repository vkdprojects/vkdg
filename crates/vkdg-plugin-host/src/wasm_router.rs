//! Bridges a WASM component to the routing [`Strategy`] trait.
//!
//! Routing is the third extension point users reach for: which account serves a
//! request is a business decision (cost, quota, tenancy) that no built-in strategy
//! can anticipate. A plugin reorders the candidates the gateway already filtered
//! for health and capability; it never sees credentials.
//!
//! A plugin that fails or returns nothing usable falls back to the candidate order
//! the gateway computed, so a broken strategy degrades to the default rather than
//! failing the request.

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use vkdg_core::{ConnectionId, RequestEnvelope, Result, VkdgError};
use vkdg_routing::{EligibilityFilter, RoutingHints, Strategy};

use crate::{PluginManifest, WasmPluginInstance};

mod export {
    pub const NAME: &str = "name";
    pub const ROUTE: &str = "route";
}

/// What the gateway hands a router.
#[derive(Serialize)]
struct RouteInput<'a> {
    /// Candidates already filtered for health, capability and eligibility.
    candidates: Vec<&'a str>,
}

/// What a router returns: the candidate ids it wants tried, in order.
#[derive(Deserialize)]
struct RouteOutput {
    candidates: Vec<String>,
}

/// A routing strategy backed by a WASM component.
pub struct WasmRouter {
    name: String,
    instance: Arc<WasmPluginInstance>,
}

impl WasmRouter {
    pub fn new(instance: Arc<WasmPluginInstance>, manifest: &PluginManifest) -> Self {
        let name = instance
            .call_string_fn(export::NAME, "")
            .unwrap_or_else(|_| manifest.id.0.clone());
        Self { name, instance }
    }

    /// The plugin's preferred order, or `None` when it cannot answer.
    fn plugin_order(&self, candidates: &[ConnectionId]) -> Option<Vec<ConnectionId>> {
        let input = RouteInput {
            candidates: candidates.iter().map(|c| c.0.as_str()).collect(),
        };
        let payload = serde_json::to_string(&input).ok()?;
        let out = self.instance.call_string_fn(export::ROUTE, &payload).ok()?;
        let parsed: RouteOutput = serde_json::from_str(&out).ok()?;

        // Only ids the gateway offered are honoured: a plugin must not conjure a
        // connection, and must not be able to route around eligibility filtering.
        let reordered: Vec<ConnectionId> = parsed
            .candidates
            .into_iter()
            .filter_map(|id| candidates.iter().find(|c| c.0 == id).cloned())
            .collect();
        // An empty result is a plugin failure, not a decision to serve nothing.
        (!reordered.is_empty()).then_some(reordered)
    }
}

#[async_trait]
impl Strategy for WasmRouter {
    fn name(&self) -> &str {
        &self.name
    }

    async fn select(
        &self,
        candidates: &[ConnectionId],
        _envelope: &RequestEnvelope,
        filter: &EligibilityFilter,
        _hints: &RoutingHints,
    ) -> Result<ConnectionId> {
        // Eligibility is the gateway's call, applied before the plugin sees the
        // list, so a plugin cannot route to an excluded connection.
        let eligible: Vec<ConnectionId> = candidates
            .iter()
            .filter(|c| !filter.is_excluded(c))
            .cloned()
            .collect();
        if eligible.is_empty() {
            return Err(VkdgError::NoEligibleConnection);
        }
        // Fall back to the gateway's own order when the plugin cannot answer.
        Ok(self
            .plugin_order(&eligible)
            .and_then(|ordered| ordered.first().cloned())
            .unwrap_or_else(|| eligible[0].clone()))
    }
}
