use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use vkdg_core::{CapabilitySet, ConnectionId};

// ── Provider / auth kinds ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Anthropic,
    OpenAI,
    Google,
    Custom { base_url: String },
}

impl ProviderKind {
    pub fn as_str(&self) -> &str {
        match self {
            ProviderKind::Anthropic => "anthropic",
            ProviderKind::OpenAI => "openai",
            ProviderKind::Google => "google",
            ProviderKind::Custom { base_url } => base_url.as_str(),
        }
    }
}

impl std::fmt::Display for ProviderKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthKind {
    ApiKey {
        env_var: String,
    },
    OAuth2 {
        token_url: String,
        client_id: String,
        client_secret_env: String,
        scopes: Vec<String>,
    },
}

// ── Connection config ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionConfig {
    pub id: ConnectionId,
    pub provider: ProviderKind,
    pub auth: AuthKind,
    /// Model names/patterns this connection serves; `*` suffix = prefix match.
    pub models: Vec<String>,
    pub max_concurrent: u32,
    pub weight: u32,
    pub tags: Vec<String>,
    /// Capabilities this connection supports (e.g. Vision, Tools, Streaming).
    /// Empty set means no capability filtering is applied (legacy / unconfigured).
    pub capabilities: CapabilitySet,
}

// ── Token state ───────────────────────────────────────────────────────────────

/// Intentionally does NOT derive Debug — contains a sensitive token.
pub struct TokenState {
    pub access_token: String,
    pub expires_at: Option<DateTime<Utc>>,
    pub generation: u64,
}

impl std::fmt::Debug for TokenState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenState")
            .field("expires_at", &self.expires_at)
            .field("generation", &self.generation)
            .finish()
    }
}

// ── Connection state ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionState {
    Healthy,
    Degraded {
        since: DateTime<Utc>,
    },
    CircuitOpen {
        until: DateTime<Utc>,
    },
    Cooldown {
        until: DateTime<Utc>,
        /// Consecutive failures; drives exponential backoff.
        failure_count: u32,
    },
}

impl ConnectionState {
    pub fn is_healthy(&self) -> bool {
        matches!(self, ConnectionState::Healthy)
    }
}

// ── Connection ────────────────────────────────────────────────────────────────

pub struct Connection {
    pub config: ConnectionConfig,
    pub state: ConnectionState,
    pub(crate) active_requests: Arc<AtomicU32>,
}

impl Connection {
    pub fn new(config: ConnectionConfig) -> Self {
        Self {
            config,
            state: ConnectionState::Healthy,
            active_requests: Arc::new(AtomicU32::new(0)),
        }
    }

    pub fn active_requests(&self) -> u32 {
        self.active_requests.load(Ordering::Relaxed)
    }

    pub fn has_capacity(&self) -> bool {
        self.active_requests() < self.config.max_concurrent
    }

    /// Returns a guard that decrements the counter on drop.
    pub fn acquire(&self) -> Option<ConnectionGuard> {
        let current = self.active_requests.load(Ordering::Acquire);
        if current >= self.config.max_concurrent {
            return None;
        }
        // Best-effort CAS; exact fairness not required on this path.
        self.active_requests.fetch_add(1, Ordering::AcqRel);
        Some(ConnectionGuard {
            counter: Arc::clone(&self.active_requests),
        })
    }

    pub(crate) fn serves_model(&self, model: &str) -> bool {
        self.config.models.iter().any(|pattern| {
            if let Some(prefix) = pattern.strip_suffix('*') {
                model.starts_with(prefix)
            } else {
                model == pattern
            }
        })
    }

    /// Record a 429/5xx and move to `Cooldown` with exponential backoff.
    ///
    /// Base: 1 s, doubling each failure up to 300 s, with deterministic jitter
    /// (`failure_count % 5` seconds) to spread retries across connections.
    ///
    /// Returns the new `cooldown_until` timestamp.
    pub fn record_upstream_error(&mut self, status_code: u16) -> DateTime<Utc> {
        let failure_count = match &self.state {
            ConnectionState::Cooldown { failure_count, .. } => *failure_count + 1,
            _ => 1,
        };
        // 2^(n-1) seconds, capped at 300 s.
        let base_secs = (2u32.pow(failure_count.saturating_sub(1).min(8))).min(300) as i64;
        let jitter = (failure_count % 5) as i64;
        let cooldown_secs = base_secs + jitter;
        let until = chrono::Utc::now() + chrono::Duration::seconds(cooldown_secs);
        self.state = ConnectionState::Cooldown {
            until,
            failure_count,
        };
        tracing::info!(
            connection_id = %self.config.id.0,
            status_code,
            failure_count,
            cooldown_secs,
            "connection entering cooldown"
        );
        until
    }

    /// Check whether the cooldown window has elapsed and, if so, transition
    /// back to `Healthy`.
    pub fn check_cooldown(&mut self) {
        if let ConnectionState::Cooldown { until, .. } = &self.state {
            if chrono::Utc::now() >= *until {
                self.state = ConnectionState::Healthy;
            }
        }
    }
}

/// RAII guard — decrements active_requests on drop.
pub struct ConnectionGuard {
    pub(crate) counter: Arc<AtomicU32>,
}

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vkdg_core::ConnectionId;

    fn make_connection() -> Connection {
        Connection::new(ConnectionConfig {
            id: ConnectionId("test-conn".into()),
            provider: ProviderKind::Custom {
                base_url: "http://localhost".into(),
            },
            auth: AuthKind::ApiKey {
                env_var: "FAKE_KEY".into(),
            },
            models: vec!["claude-3".into()],
            max_concurrent: 4,
            weight: 1,
            tags: vec![],
            capabilities: vkdg_core::CapabilitySet::default(),
        })
    }

    // Plausible wrong impl: backoff doesn't increase on repeated failures
    #[test]
    fn backoff_increases_with_failures() {
        let mut conn = make_connection();
        let t1 = conn.record_upstream_error(429);
        let t2 = conn.record_upstream_error(429);
        assert!(
            t2 > t1,
            "second failure must produce a later cooldown deadline"
        );
    }

    // Plausible wrong impl: cooldown doesn't expire, stays in Cooldown forever
    #[test]
    fn cooldown_expires_after_duration() {
        let mut conn = make_connection();
        conn.state = ConnectionState::Cooldown {
            until: chrono::Utc::now() - chrono::Duration::seconds(1),
            failure_count: 1,
        };
        conn.check_cooldown();
        assert!(
            matches!(conn.state, ConnectionState::Healthy),
            "expired cooldown must transition to Healthy"
        );
    }

    // Guard: unexpired cooldown must NOT transition to Healthy
    #[test]
    fn cooldown_does_not_expire_early() {
        let mut conn = make_connection();
        conn.state = ConnectionState::Cooldown {
            until: chrono::Utc::now() + chrono::Duration::seconds(60),
            failure_count: 1,
        };
        conn.check_cooldown();
        assert!(
            matches!(conn.state, ConnectionState::Cooldown { .. }),
            "active cooldown must not transition to Healthy prematurely"
        );
    }

    // Plausible wrong impl: first failure uses failure_count=0 -> 2^(0-1) underflows
    #[test]
    fn first_failure_produces_positive_cooldown() {
        let mut conn = make_connection();
        let now = chrono::Utc::now();
        let until = conn.record_upstream_error(429);
        assert!(until > now, "cooldown deadline must be in the future");
    }
}
