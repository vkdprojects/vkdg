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
    fn name(&self) -> &'static str {
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
    fn name(&self) -> &'static str {
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

// ── Latency-aware strategies ──────────────────────────────────────────────────

/// Eligible candidates with their p50 latency. A connection with no data yet
/// counts as 0 ms, so a new connection gets sampled instead of starved.
fn with_latency<'a>(
    candidates: &'a [ConnectionId],
    filter: &EligibilityFilter,
    hints: &RoutingHints,
) -> Vec<(&'a ConnectionId, u32)> {
    candidates
        .iter()
        .filter(|c| !filter.is_excluded(c))
        .map(|c| (c, hints.latency_p50_ms.get(c).copied().unwrap_or(0)))
        .collect()
}

/// Always the eligible target with the lowest observed p50 latency.
pub struct LowestLatencyStrategy;

#[async_trait]
impl Strategy for LowestLatencyStrategy {
    fn name(&self) -> &'static str {
        "lowest_latency"
    }

    async fn select(
        &self,
        candidates: &[ConnectionId],
        _envelope: &RequestEnvelope,
        filter: &EligibilityFilter,
        hints: &RoutingHints,
    ) -> Result<ConnectionId> {
        with_latency(candidates, filter, hints)
            .into_iter()
            .min_by_key(|(_, ms)| *ms)
            .map(|(c, _)| c.clone())
            .ok_or(VkdgError::NoEligibleConnection)
    }
}

/// Power of two choices: sample two distinct eligible targets, take the one with
/// fewer requests in flight; a tie goes to the first sample, which is random.
/// Latency is deliberately not a signal here: it is only measured on targets
/// that receive traffic, so preferring the faster one starves the other for as
/// long as the first stays fast. Use `lowest_latency` to chase latency.
pub struct PowerOfTwoChoicesStrategy {
    seed: AtomicUsize,
}

impl PowerOfTwoChoicesStrategy {
    pub fn new() -> Self {
        Self {
            seed: AtomicUsize::new(0x9E37_79B9),
        }
    }

    /// xorshift over a shared counter: cheap, lock-free, good enough to spread picks.
    fn next(&self) -> usize {
        let mut x = self.seed.fetch_add(0x9E37_79B9, Ordering::Relaxed) | 1;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    }
}

impl Default for PowerOfTwoChoicesStrategy {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Strategy for PowerOfTwoChoicesStrategy {
    fn name(&self) -> &'static str {
        "power_of_two_choices"
    }

    async fn select(
        &self,
        candidates: &[ConnectionId],
        _envelope: &RequestEnvelope,
        filter: &EligibilityFilter,
        hints: &RoutingHints,
    ) -> Result<ConnectionId> {
        let eligible: Vec<&ConnectionId> = candidates
            .iter()
            .filter(|c| !filter.is_excluded(c))
            .collect();
        let n = eligible.len();
        if n == 0 {
            return Err(VkdgError::NoEligibleConnection);
        }
        let a = self.next() % n;
        let b = if n > 1 {
            (a + 1 + self.next() % (n - 1)) % n
        } else {
            a
        };
        let load = |c: &ConnectionId| hints.in_flight.get(c).copied().unwrap_or(0);
        let pick = if load(eligible[b]) < load(eligible[a]) {
            b
        } else {
            a
        };
        Ok(eligible[pick].clone())
    }
}
