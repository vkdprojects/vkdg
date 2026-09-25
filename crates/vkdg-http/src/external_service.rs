//! Client for an external service process communicating over HTTP.
//! Includes deadline propagation and a simple circuit breaker.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use bytes::Bytes;
use vkdg_core::VkdgError;

// ── Circuit breaker ───────────────────────────────────────────────────────────

/// Simple half-open circuit breaker.
pub struct CircuitBreaker {
    failures: AtomicU32,
    last_failure: Mutex<Option<Instant>>,
    threshold: u32,
    cooldown_secs: u64,
}

impl CircuitBreaker {
    pub fn new(threshold: u32, cooldown_secs: u64) -> Self {
        Self {
            failures: AtomicU32::new(0),
            last_failure: Mutex::new(None),
            threshold,
            cooldown_secs,
        }
    }

    /// Returns true if the circuit is open (requests should be rejected).
    pub fn is_open(&self) -> bool {
        let failures = self.failures.load(Ordering::Relaxed);
        if failures < self.threshold {
            return false;
        }
        let last = self.last_failure.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(t) = *last {
            t.elapsed().as_secs() < self.cooldown_secs
        } else {
            false
        }
    }

    pub fn record_failure(&self) {
        self.failures.fetch_add(1, Ordering::Relaxed);
        let mut last = self.last_failure.lock().unwrap_or_else(|p| p.into_inner());
        *last = Some(Instant::now());
    }

    pub fn record_success(&self) {
        self.failures.store(0, Ordering::Relaxed);
    }
}

// ── External service client ───────────────────────────────────────────────────

/// Client for an external service process communicating over HTTP.
/// Applies a per-request timeout and a circuit breaker to fail fast when the
/// downstream is unhealthy.
pub struct ExternalServiceClient {
    base_url: String,
    client: reqwest::Client,
    breaker: CircuitBreaker,
}

impl ExternalServiceClient {
    pub fn new(base_url: String, timeout_ms: u64, threshold: u32, cooldown_secs: u64) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(timeout_ms))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            base_url,
            client,
            breaker: CircuitBreaker::new(threshold, cooldown_secs),
        }
    }

    /// Call the external service. Returns `Err` if the circuit is open or the
    /// request fails.
    pub async fn call(&self, path: &str, body: Bytes) -> Result<Bytes, VkdgError> {
        if self.breaker.is_open() {
            return Err(VkdgError::UpstreamError {
                code: 503,
                message: "circuit open".into(),
            });
        }
        let url = format!("{}{}", self.base_url, path);
        let resp = self
            .client
            .post(&url)
            .header("content-type", "application/json")
            .body(body)
            .send()
            .await
            .map_err(|e| {
                self.breaker.record_failure();
                VkdgError::UpstreamError {
                    code: 0,
                    message: e.to_string(),
                }
            })?;
        if !resp.status().is_success() {
            self.breaker.record_failure();
            return Err(VkdgError::UpstreamError {
                code: resp.status().as_u16(),
                message: "external service error".into(),
            });
        }
        self.breaker.record_success();
        resp.bytes().await.map_err(|e| VkdgError::UpstreamError {
            code: 0,
            message: e.to_string(),
        })
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // Defeat: circuit stays closed even after threshold failures, never rejecting
    // requests — the breaker would be wired but never opens.
    #[test]
    fn circuit_breaker_opens_after_threshold() {
        let cb = CircuitBreaker::new(3, 60);
        assert!(!cb.is_open(), "should be closed initially");
        cb.record_failure();
        cb.record_failure();
        assert!(!cb.is_open(), "should be closed before threshold");
        cb.record_failure();
        assert!(cb.is_open(), "should open at threshold (3 failures)");
    }

    // Defeat: circuit stays open forever, never allowing recovery even after
    // the cooldown period has elapsed.
    // Use cooldown_secs=0 so we don't need any sleep.
    #[test]
    fn circuit_breaker_closes_after_cooldown() {
        let cb = CircuitBreaker::new(1, 0); // cooldown=0s → elapsed always ≥ 0
        cb.record_failure();
        // elapsed() >= 0 == cooldown_secs, so condition `elapsed < 0` is false
        assert!(!cb.is_open(), "should be closed when cooldown_secs=0");
    }

    // Defeat: record_success is called but failures counter is not reset, so
    // the circuit stays open and never recovers after a healthy response.
    #[test]
    fn circuit_breaker_resets_on_success() {
        let cb = CircuitBreaker::new(2, 60);
        cb.record_failure();
        cb.record_failure();
        assert!(cb.is_open(), "should be open after threshold");
        cb.record_success();
        assert!(!cb.is_open(), "should be closed after success resets failures");
    }
}
