//! EWMA latency tracker per connection.
//!
//! Records upstream response times and maintains an exponentially-weighted
//! moving average per connection. Fed into the routing scorer as
//! `latency_p50_ms` signals to prefer faster connections.
//!
//! Phase E: add p95/p99 tracking via histogram.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use vkdg_core::ConnectionId;

/// EWMA with configurable smoothing factor (alpha).
/// Higher alpha = faster response to recent changes.
/// Standard alpha = 0.1 (slow smooth), 0.3 (moderate), 0.7 (fast).
const DEFAULT_ALPHA: f32 = 0.2;

#[derive(Debug, Clone)]
pub struct EwmaLatency {
    /// Current EWMA latency in milliseconds.
    pub ewma_ms: f32,
    /// Number of samples recorded.
    pub sample_count: u32,
    pub alpha: f32,
}

impl EwmaLatency {
    pub fn new(alpha: f32) -> Self {
        Self {
            ewma_ms: 0.0,
            sample_count: 0,
            alpha,
        }
    }

    pub fn record(&mut self, latency_ms: u32) {
        let l = latency_ms as f32;
        if self.sample_count == 0 {
            self.ewma_ms = l;
        } else {
            self.ewma_ms = self.alpha * l + (1.0 - self.alpha) * self.ewma_ms;
        }
        self.sample_count += 1;
    }

    pub fn p50_ms(&self) -> Option<u32> {
        if self.sample_count == 0 {
            None
        } else {
            Some(self.ewma_ms as u32)
        }
    }
}

pub struct LatencyTracker {
    latencies: RwLock<HashMap<ConnectionId, EwmaLatency>>,
    alpha: f32,
}

impl LatencyTracker {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            latencies: RwLock::new(HashMap::new()),
            alpha: DEFAULT_ALPHA,
        })
    }

    pub fn with_alpha(alpha: f32) -> Arc<Self> {
        Arc::new(Self {
            latencies: RwLock::new(HashMap::new()),
            alpha: alpha.clamp(0.01, 0.99),
        })
    }

    /// Record a latency sample for a connection.
    pub async fn record(&self, conn: &ConnectionId, latency_ms: u32) {
        let mut map = self.latencies.write().await;
        map.entry(conn.clone())
            .or_insert_with(|| EwmaLatency::new(self.alpha))
            .record(latency_ms);
    }

    /// Get the current EWMA p50 latency for a connection.
    /// Returns None if no samples have been recorded.
    pub async fn p50_ms(&self, conn: &ConnectionId) -> Option<u32> {
        let map = self.latencies.read().await;
        map.get(conn).and_then(|e| e.p50_ms())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Plausible wrong impl: EWMA not updated on second sample (stays at first value)
    #[tokio::test]
    async fn ewma_updates_toward_new_value() {
        let t = LatencyTracker::with_alpha(0.5); // high alpha for fast convergence
        let conn = ConnectionId("c".into());
        t.record(&conn, 100).await;
        t.record(&conn, 200).await; // should move toward 200
        let p50 = t.p50_ms(&conn).await.unwrap();
        assert!(
            p50 > 100,
            "EWMA must move toward new higher value, got {p50}"
        );
    }

    // Plausible wrong impl: first sample ignored, p50 stays None
    #[tokio::test]
    async fn first_sample_initializes_ewma() {
        let t = LatencyTracker::new();
        let conn = ConnectionId("c".into());
        assert!(t.p50_ms(&conn).await.is_none(), "no samples = no p50");
        t.record(&conn, 150).await;
        assert_eq!(
            t.p50_ms(&conn).await,
            Some(150),
            "first sample must initialize EWMA"
        );
    }

    // Plausible wrong impl: different connections share the same EWMA
    #[tokio::test]
    async fn different_connections_tracked_independently() {
        let t = LatencyTracker::new();
        let a = ConnectionId("a".into());
        let b = ConnectionId("b".into());
        t.record(&a, 50).await;
        t.record(&b, 500).await;
        assert!(t.p50_ms(&a).await.unwrap() < t.p50_ms(&b).await.unwrap());
    }

    // Plausible wrong impl: alpha clamp not applied — with_alpha(0.0) uses 0.0
    // and produces NaN/zero scores because 0.0 * x + 1.0 * 0.0 = 0.0 forever.
    #[tokio::test]
    async fn alpha_zero_clamped_to_minimum() {
        let t = LatencyTracker::with_alpha(0.0);
        let conn = ConnectionId("c".into());
        t.record(&conn, 100).await;
        t.record(&conn, 200).await;
        // Clamped to 0.01; EWMA must be finite and positive, not NaN or zero.
        let p50 = t.p50_ms(&conn).await.unwrap();
        assert!(
            p50 > 0,
            "clamped alpha must produce valid EWMA, not NaN or zero"
        );
    }

    // Plausible wrong impl: EWMA does not converge toward steady-state value
    // (alpha applied in wrong direction, e.g. (1-alpha)*new + alpha*old).
    #[tokio::test]
    async fn ewma_converges_toward_steady_state() {
        let t = LatencyTracker::with_alpha(0.2);
        let conn = ConnectionId("c".into());
        // Feed 30 samples of 100ms; after convergence EWMA must be within 10ms of 100.
        for _ in 0..30 {
            t.record(&conn, 100).await;
        }
        let p50 = t.p50_ms(&conn).await.unwrap();
        assert!(
            (p50 as i64 - 100).abs() < 10,
            "EWMA must converge to ~100ms after 30 samples, got {p50}"
        );
    }
}
