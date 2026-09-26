use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;

use vkdg_core::{ConnectionId, RequestEnvelope, Result, VkdgError};

use crate::types::{EligibilityFilter, RoutingHints};

// ── Strategy trait ────────────────────────────────────────────────────────────

#[async_trait]
pub trait Strategy: Send + Sync {
    fn name(&self) -> &str;
    async fn select(
        &self,
        candidates: &[ConnectionId],
        envelope: &RequestEnvelope,
        filter: &EligibilityFilter,
        hints: &RoutingHints,
    ) -> Result<ConnectionId>;
}

// ── RoundRobinStrategy ────────────────────────────────────────────────────────

pub struct RoundRobinStrategy {
    counter: AtomicUsize,
}

impl RoundRobinStrategy {
    pub fn new() -> Self {
        Self {
            counter: AtomicUsize::new(0),
        }
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
        _hints: &RoutingHints,
    ) -> Result<ConnectionId> {
        let eligible: Vec<&ConnectionId> = candidates
            .iter()
            .filter(|c| !filter.is_excluded(c))
            .collect();
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
        _hints: &RoutingHints,
    ) -> Result<ConnectionId> {
        candidates
            .iter()
            .find(|c| !filter.is_excluded(c))
            .cloned()
            .ok_or(VkdgError::NoEligibleConnection)
    }
}
