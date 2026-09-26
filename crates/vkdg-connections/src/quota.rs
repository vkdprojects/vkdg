//! Per-connection quota tracker.
//!
//! Tracks estimated tokens consumed per connection in the current quota window.
//! Used by the routing scorer to compute quota_headroom signals.
//!
//! Phase D: in-memory, resets on restart.
//! Phase E: persist with the OAuth vault; reconcile after restart.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use tokio::sync::RwLock;

use vkdg_core::ConnectionId;

#[derive(Debug, Clone)]
pub struct QuotaWindow {
    /// Tokens consumed since window start
    pub consumed: u64,
    /// Maximum tokens in the window (None = unknown/unlimited)
    pub limit: Option<u64>,
    pub window_start: DateTime<Utc>,
    /// Window duration in seconds (e.g., 60 for per-minute, 2_592_000 for per-month)
    pub window_secs: u64,
}

impl QuotaWindow {
    pub fn headroom(&self) -> Option<f32> {
        let limit = self.limit? as f32;
        let remaining = (limit - self.consumed as f32).max(0.0);
        Some(remaining / limit)
    }

    pub fn is_window_expired(&self) -> bool {
        let age = Utc::now().signed_duration_since(self.window_start);
        age.num_seconds() as u64 >= self.window_secs
    }
}

pub struct QuotaTracker {
    windows: RwLock<HashMap<ConnectionId, QuotaWindow>>,
}

impl QuotaTracker {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { windows: RwLock::new(HashMap::new()) })
    }

    /// Record token usage for a connection.
    pub async fn record_usage(&self, conn: &ConnectionId, tokens: u64) {
        let mut windows = self.windows.write().await;
        let entry = windows.entry(conn.clone()).or_insert_with(|| QuotaWindow {
            consumed: 0,
            limit: None,
            window_start: Utc::now(),
            window_secs: 2_592_000, // default: monthly
        });
        if entry.is_window_expired() {
            entry.consumed = 0;
            entry.window_start = Utc::now();
        }
        entry.consumed += tokens;
    }

    /// Set the quota limit for a connection (from provider response headers or config).
    pub async fn set_limit(&self, conn: &ConnectionId, limit: u64, window_secs: u64) {
        let mut windows = self.windows.write().await;
        let entry = windows.entry(conn.clone()).or_insert_with(|| QuotaWindow {
            consumed: 0,
            limit: None,
            window_start: Utc::now(),
            window_secs,
        });
        entry.limit = Some(limit);
        entry.window_secs = window_secs;
    }

    /// Get headroom [0.0, 1.0] for a connection. None if no quota configured.
    pub async fn headroom(&self, conn: &ConnectionId) -> Option<f32> {
        let windows = self.windows.read().await;
        windows.get(conn).and_then(|w| w.headroom())
    }
}

impl Default for QuotaTracker {
    fn default() -> Self {
        Self { windows: RwLock::new(HashMap::new()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Plausible wrong impl: usage not tracked, headroom always None
    #[tokio::test]
    async fn headroom_decreases_as_tokens_consumed() {
        let t = QuotaTracker::new();
        let conn = ConnectionId("c".into());
        t.set_limit(&conn, 1000, 3600).await;
        t.record_usage(&conn, 500).await;
        let h = t.headroom(&conn).await.unwrap();
        assert!((h - 0.5).abs() < 0.01, "headroom must be 0.5 after consuming half, got {h}");
    }

    // Plausible wrong impl: usage accumulates across windows (never resets)
    #[tokio::test]
    async fn window_reset_clears_consumption() {
        let t = QuotaTracker::new();
        let conn = ConnectionId("c".into());
        // Set limit with 0s window so it's immediately expired on next record_usage
        t.set_limit(&conn, 1000, 0).await;
        t.record_usage(&conn, 900).await;
        // Next record_usage sees expired window → resets consumed to 0, then adds 100
        t.record_usage(&conn, 100).await;
        let h = t.headroom(&conn).await.unwrap();
        // After reset: consumed = 100, limit = 1000, headroom = 0.9
        assert!(h > 0.8, "headroom must reset when window expires, got {h}");
    }

    // Plausible wrong impl: headroom returns Some(0) when no limit set
    #[tokio::test]
    async fn no_limit_returns_none_headroom() {
        let t = QuotaTracker::new();
        let conn = ConnectionId("c".into());
        t.record_usage(&conn, 500).await;
        assert!(t.headroom(&conn).await.is_none());
    }
}
