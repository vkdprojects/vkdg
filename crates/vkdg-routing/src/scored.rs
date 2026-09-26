use async_trait::async_trait;

use vkdg_core::{ConnectionId, RequestEnvelope, Result, VkdgError};

use crate::scorer;
use crate::strategy::Strategy;
use crate::types::{EligibilityFilter, RoutingHints};

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
        hints: &RoutingHints,
    ) -> Result<ConnectionId> {
        let effective_mode = hints.mode_pack.as_deref().unwrap_or(&self.mode_pack);
        let signals: Vec<scorer::CandidateSignals> = candidates
            .iter()
            .filter(|id| !filter.is_excluded(id))
            .map(|id| scorer::CandidateSignals {
                connection_id: id.0.clone(),
                health: 1.0,
                quota_headroom: hints.quota_headroom.get(id).copied(),
                cost_per_ktoken: None,
                latency_p50_ms: hints.latency_p50_ms.get(id).copied(),
                instability_events: 0,
            })
            .collect();

        if signals.is_empty() {
            return Err(VkdgError::NoEligibleConnection);
        }

        let weights = scorer::ScoringWeights::from_mode_pack(effective_mode);
        scorer::rank_candidates(&signals, &weights)
            .into_iter()
            .next()
            .map(ConnectionId)
            .ok_or(VkdgError::NoEligibleConnection)
    }
}
