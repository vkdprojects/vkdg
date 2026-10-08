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
    /// A provider plugin addressed by its registry id, e.g. `kiro`, `groq`.
    Plugin {
        id: String,
    },
    /// An `OpenAI` Chat Completions endpoint (`openai-compat` in config).
    Custom {
        base_url: String,
    },
    /// An Anthropic Messages endpoint (`anthropic-compat` in config).
    AnthropicCompat {
        base_url: String,
    },
}

impl ProviderKind {
    pub fn as_str(&self) -> &str {
        match self {
            ProviderKind::Anthropic => "anthropic",
            ProviderKind::OpenAI => "openai",
            ProviderKind::Google => "google",
            ProviderKind::Plugin { id } => id.as_str(),
            ProviderKind::Custom { base_url } | ProviderKind::AnthropicCompat { base_url } => {
                base_url.as_str()
            }
        }
    }

    /// Returns the registry key used to look up a [`ProviderAdapter`] for this kind.
    /// `Custom` connections are bare OpenAI-compatible endpoints, so they use the
    /// `OpenAI` adapter; `Plugin` connections address their own adapter by id.
    pub fn adapter_id(&self) -> &str {
        match self {
            ProviderKind::Anthropic => "anthropic",
            ProviderKind::OpenAI => "openai",
            ProviderKind::Google => "google",
            ProviderKind::Plugin { id } => id.as_str(),
            ProviderKind::Custom { .. } => "openai",
            ProviderKind::AnthropicCompat { .. } => "anthropic",
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
    /// A persisted provider account (OAuth login or imported token) in the
    /// [`AccountStore`](crate::AccountStore). Refresh is delegated to the plugin.
    Account {
        account_id: String,
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
    /// Provider endpoint this connection uses, when the provider exposes more
    /// than one (`kiro`: `runtime` | `codewhisperer`). `None` lets the plugin
    /// choose from the credential type.
    pub endpoint: Option<String>,
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
            .finish_non_exhaustive()
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
    /// True when the connection can currently be selected.
    ///
    /// `Cooldown` and `CircuitOpen` carry an `until` deadline rather than a
    /// standing flag: once the deadline has passed the connection is
    /// eligible again even if nothing has yet written `Healthy` back into
    /// `self`. The write-back (`Connection::check_cooldown`) only runs after
    /// a request is actually dispatched to the connection, which can never
    /// happen while a naive discriminant match keeps excluding it — that
    /// deadlock is what let all three production connections stay stuck in
    /// `Cooldown` long after `until` had elapsed. Comparing the clock here,
    /// at the read used for selection, makes recovery self-healing without
    /// taking a writer lock on the hot path.
    pub fn is_healthy(&self) -> bool {
        match self {
            ConnectionState::Healthy => true,
            ConnectionState::Degraded { .. } => false,
            ConnectionState::CircuitOpen { until } | ConnectionState::Cooldown { until, .. } => {
                chrono::Utc::now() >= *until
            }
        }
    }
}

// ── Connection ────────────────────────────────────────────────────────────────

/// Longest cooldown an upstream `retry-after` can impose: 6 hours. Covers a
/// subscription window that resets in hours; anything larger is treated as noise.
pub const MAX_RETRY_AFTER_COOLDOWN_SECS: i64 = 6 * 60 * 60;

/// `name` with every `.` replaced by `-`, borrowed when it has no dot.
///
/// Providers spell the same model with dots or dashes between version digits
/// (`claude-sonnet-4.6` / `claude-sonnet-4-6`); this is the form eligibility
/// compares in.
fn canonical_model(name: &str) -> std::borrow::Cow<'_, str> {
    if name.contains('.') {
        std::borrow::Cow::Owned(name.replace('.', "-"))
    } else {
        std::borrow::Cow::Borrowed(name)
    }
}

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

    /// True when one of this connection's `models` patterns covers `model`.
    ///
    /// Kiro spells versions with dots (`claude-sonnet-4.6`) and Anthropic with
    /// dashes (`claude-sonnet-4-6`); for eligibility they are the same model, so
    /// patterns and the requested model are compared in their dash form. Glob
    /// semantics (`*`, `?`) are those of [`vkdg_core::glob`] and are untouched.
    pub(crate) fn serves_model(&self, model: &str) -> bool {
        let model = canonical_model(model);
        self.config
            .models
            .iter()
            .any(|pattern| vkdg_core::glob::matches(&canonical_model(pattern), &model))
    }

    /// Record a 429/5xx (or a 402 out-of-credits) and move to `Cooldown`.
    ///
    /// The floor is exponential backoff: 1 s, doubling each failure up to 300 s,
    /// with deterministic jitter (`failure_count % 5` seconds) to spread retries
    /// across connections. When the upstream said how long to wait
    /// (`retry_after`, seconds), the cooldown lasts at least that long, capped at
    /// [`MAX_RETRY_AFTER_COOLDOWN_SECS`] so a bogus header cannot park a
    /// connection for days. A subscription limit that resets in hours is thereby
    /// not re-probed every five minutes.
    ///
    /// A 402 means the account is out of credits. Credits do not come back
    /// within minutes, so it holds the connection for the full
    /// [`MAX_RETRY_AFTER_COOLDOWN_SECS`] whatever `retry_after` says: a shorter
    /// backoff would only spend a request re-probing a dead account.
    ///
    /// Returns the new `cooldown_until` timestamp.
    pub fn record_upstream_error(
        &mut self,
        status_code: u16,
        retry_after: Option<u32>,
    ) -> DateTime<Utc> {
        let failure_count = match &self.state {
            ConnectionState::Cooldown { failure_count, .. } => *failure_count + 1,
            _ => 1,
        };
        // 2^(n-1) seconds, capped at 300 s.
        let base_secs = i64::from((2u32.pow(failure_count.saturating_sub(1).min(8))).min(300));
        let jitter = i64::from(failure_count % 5);
        let backoff_secs = base_secs + jitter;
        let requested_secs = if status_code == 402 {
            MAX_RETRY_AFTER_COOLDOWN_SECS
        } else {
            retry_after.map_or(0, |s| i64::from(s).min(MAX_RETRY_AFTER_COOLDOWN_SECS))
        };
        let cooldown_secs = backoff_secs.max(requested_secs);
        let until = chrono::Utc::now() + chrono::Duration::seconds(cooldown_secs);
        self.state = ConnectionState::Cooldown {
            until,
            failure_count,
        };
        tracing::info!(
            connection_id = %self.config.id.0,
            status_code,
            failure_count,
            retry_after,
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

    /// Explicitly reset any active cooldown or circuit open state back to `Healthy`.
    pub fn reset_cooldown(&mut self) {
        self.state = ConnectionState::Healthy;
    }
}

/// RAII guard — decrements `active_requests` on drop.
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
            endpoint: None,
            capabilities: vkdg_core::CapabilitySet::default(),
        })
    }

    // Plausible wrong impl: backoff doesn't increase on repeated failures
    #[test]
    fn backoff_increases_with_failures() {
        let mut conn = make_connection();
        let t1 = conn.record_upstream_error(429, None);
        let t2 = conn.record_upstream_error(429, None);
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
        let until = conn.record_upstream_error(429, None);
        assert!(until > now, "cooldown deadline must be in the future");
    }

    // Plausible wrong impl: ConnectionCatalog.eligible() returns a connection
    // that is in Cooldown state — is_healthy() guard missing or bypassed.
    #[test]
    fn connection_in_cooldown_is_not_eligible() {
        use crate::catalog::ConnectionCatalog;

        let conn_id = ConnectionId("c".into());
        let config = ConnectionConfig {
            id: conn_id.clone(),
            provider: ProviderKind::Anthropic,
            auth: AuthKind::ApiKey {
                env_var: "K".into(),
            },
            models: vec!["claude-*".into()],
            max_concurrent: 10,
            weight: 1,
            tags: vec![],
            endpoint: None,
            capabilities: vkdg_core::CapabilitySet::default(),
        };
        let catalog = ConnectionCatalog::new(vec![config]);
        // Set the connection into an unexpired future cooldown.
        {
            let conn_arc = catalog.get(&conn_id).unwrap();
            let mut conn = conn_arc.try_write().unwrap();
            conn.state = ConnectionState::Cooldown {
                until: chrono::Utc::now() + chrono::Duration::seconds(300),
                failure_count: 1,
            };
        }
        let eligible = catalog.eligible("claude-3-5-haiku-20241022", &[]);
        assert!(
            eligible.is_empty(),
            "connection in cooldown must not be eligible for routing"
        );
    }
    #[test]
    fn reset_cooldown_restores_health_and_eligibility() {
        use crate::catalog::ConnectionCatalog;

        let conn_id = ConnectionId("c_reset".into());
        let config = ConnectionConfig {
            id: conn_id.clone(),
            provider: ProviderKind::Anthropic,
            auth: AuthKind::ApiKey {
                env_var: "K".into(),
            },
            models: vec!["claude-*".into()],
            max_concurrent: 10,
            weight: 1,
            tags: vec![],
            endpoint: None,
            capabilities: vkdg_core::CapabilitySet::default(),
        };
        let catalog = ConnectionCatalog::new(vec![config]);
        {
            let conn_arc = catalog.get(&conn_id).unwrap();
            let mut conn = conn_arc.try_write().unwrap();
            conn.state = ConnectionState::Cooldown {
                until: chrono::Utc::now() + chrono::Duration::seconds(300),
                failure_count: 3,
            };
        }
        assert_eq!(
            catalog.eligible("claude-3-5-haiku-20241022", &[]),
            Vec::<ConnectionId>::new()
        );

        let reset = catalog.reset_cooldown(&conn_id);
        assert!(
            reset,
            "reset_cooldown should return true for existing connection"
        );

        let eligible = catalog.eligible("claude-3-5-haiku-20241022", &[]);
        assert_eq!(eligible.len(), 1);
        assert_eq!(eligible[0], conn_id);
    }

    // Plausible wrong impl: eligibility reads a cached Healthy/Cooldown
    // discriminant that is only updated by an explicit check_cooldown() call
    // (itself only invoked after a successful dispatch to that very
    // connection) instead of comparing `until` against the clock at
    // selection time. Reproduces production: a connection whose cooldown
    // has expired must become selectable again with no intervening write.
    #[test]
    fn connection_with_expired_cooldown_is_eligible_without_explicit_recovery() {
        use crate::catalog::ConnectionCatalog;

        let conn_id = ConnectionId("c".into());
        let config = ConnectionConfig {
            id: conn_id.clone(),
            provider: ProviderKind::Anthropic,
            auth: AuthKind::ApiKey {
                env_var: "K".into(),
            },
            models: vec!["claude-*".into()],
            max_concurrent: 10,
            weight: 1,
            tags: vec![],
            endpoint: None,
            capabilities: vkdg_core::CapabilitySet::default(),
        };
        let catalog = ConnectionCatalog::new(vec![config]);
        // Set the connection into an already-expired cooldown, as would be
        // observed long after `record_upstream_error` fired and nothing has
        // dispatched to this connection since (it was excluded from every
        // selection while unhealthy, so check_cooldown() never ran).
        {
            let conn_arc = catalog.get(&conn_id).unwrap();
            let mut conn = conn_arc.try_write().unwrap();
            conn.state = ConnectionState::Cooldown {
                until: chrono::Utc::now() - chrono::Duration::seconds(1),
                failure_count: 1,
            };
        }
        let eligible = catalog.eligible("claude-3-5-haiku-20241022", &[]);
        assert!(
            eligible.contains(&conn_id),
            "connection whose cooldown has already expired must be eligible \
             for routing without a prior explicit recovery call"
        );
    }

    fn serving(patterns: &[&str]) -> Connection {
        let mut conn = make_connection();
        conn.config.models = patterns.iter().map(|p| (*p).to_owned()).collect();
        conn
    }

    // Plausible wrong impl: comparing the raw strings, so a Kiro connection that
    // lists `claude-sonnet-4.6` is skipped for a client asking `claude-sonnet-4-6`
    // (and an Anthropic `-4-6` entry for a client asking `-4.6`): the route then
    // has no eligible target even though both connections serve the model.
    #[test]
    fn dots_and_dashes_between_version_digits_are_the_same_model_both_ways() {
        for (pattern, requested) in [
            ("claude-sonnet-4.6", "claude-sonnet-4-6"),
            ("claude-sonnet-4-6", "claude-sonnet-4.6"),
            ("claude-sonnet-4.6", "claude-sonnet-4.6"),
            ("claude-*", "claude-sonnet-4.6"),
            ("claude-*", "claude-sonnet-4-6"),
            ("claude-sonnet-4.*", "claude-sonnet-4-6"),
            ("gpt-5.6-*", "gpt-5-6-mini"),
            ("gpt-5-6-*", "gpt-5.6-mini"),
        ] {
            assert!(
                serving(&[pattern]).serves_model(requested),
                "pattern {pattern:?} must serve {requested:?}"
            );
        }
    }

    // Plausible wrong impl: normalising by turning the dot into a wildcard (or
    // dropping version digits), which would let `4.6` serve `4-5`, `4x6` or `4.60`.
    #[test]
    fn normalising_dots_does_not_loosen_the_match() {
        for (pattern, requested) in [
            ("claude-sonnet-4.6", "claude-sonnet-4-5"),
            ("claude-sonnet-4-6", "claude-sonnet-4.60"),
            ("gpt-5.6-*", "gpt-5x6-mini"),
            ("gpt-5.6-*", "gpt-5-7-mini"),
            ("claude-*", "gpt-4.1"),
            ("claude-sonnet-4.6", "claude-sonnet-4-6-20250101"),
        ] {
            assert!(
                !serving(&[pattern]).serves_model(requested),
                "pattern {pattern:?} must not serve {requested:?}"
            );
        }
        // `?` stays exactly one character.
        assert!(serving(&["o?-mini"]).serves_model("o3-mini"));
        assert!(!serving(&["o?-mini"]).serves_model("o33-mini"));
    }

    fn secs_until(until: DateTime<Utc>, from: DateTime<Utc>) -> i64 {
        (until - from).num_seconds()
    }

    // Plausible wrong impl: `retry_after` is ignored (the exponential backoff caps at
    // 300 s), so a Claude subscription limit resetting in 5 h is re-probed every
    // 5 minutes and burns a request each time.
    #[test]
    fn cooldown_lasts_at_least_the_upstream_retry_after() {
        let mut conn = make_connection();
        let before = chrono::Utc::now();
        let until = conn.record_upstream_error(429, Some(18_000));
        assert!(
            secs_until(until, before) >= 18_000,
            "a 429 carrying retry-after: 18000 must cool the connection for at least 18000 s"
        );
        assert!(!conn.state.is_healthy());
        match conn.state {
            ConnectionState::Cooldown {
                until: stored,
                failure_count,
            } => {
                assert_eq!(stored, until, "the returned deadline is the stored one");
                assert_eq!(failure_count, 1);
            }
            other => panic!("expected Cooldown, got {other:?}"),
        }
    }

    // Plausible wrong impl: trusting retry-after without a cap, so a hostile or
    // buggy `retry-after: 4294967295` parks the connection for 136 years.
    #[test]
    fn retry_after_is_capped_at_six_hours() {
        assert_eq!(MAX_RETRY_AFTER_COOLDOWN_SECS, 21_600);
        let mut conn = make_connection();
        let before = chrono::Utc::now();
        let until = conn.record_upstream_error(429, Some(u32::MAX));
        let after = chrono::Utc::now();
        assert!(secs_until(until, before) >= 21_600, "cap is a floor here");
        assert!(
            until <= after + chrono::Duration::seconds(21_600),
            "cooldown must not exceed 21600 s: until={until}"
        );
    }

    // Plausible wrong impl: retry-after *replaces* the backoff instead of raising
    // it, so a tiny `retry-after: 1` after eight straight failures shortens the
    // cooldown to 1 s and a flapping upstream is hammered.
    #[test]
    fn backoff_stays_the_floor_when_retry_after_is_absent_or_smaller() {
        // Ninth consecutive failure: 2^8 = 256 s base + 9 % 5 = 4 s jitter = 260 s.
        for retry_after in [None, Some(0), Some(1)] {
            let mut conn = make_connection();
            conn.state = ConnectionState::Cooldown {
                until: chrono::Utc::now(),
                failure_count: 8,
            };
            let before = chrono::Utc::now();
            let until = conn.record_upstream_error(503, retry_after);
            assert!(
                secs_until(until, before) >= 259,
                "retry_after={retry_after:?} must not shorten the 260 s backoff"
            );
        }
    }

    // Plausible wrong impl: applying a default retry-after (or the cap) when the
    // upstream sent none, turning a transient 429 into hours of outage.
    #[test]
    fn without_retry_after_the_first_cooldown_is_seconds_not_hours() {
        let mut conn = make_connection();
        let before = chrono::Utc::now();
        let until = conn.record_upstream_error(429, None);
        assert!(secs_until(until, before) <= 5);
    }

    // Plausible wrong impl: 402 (out of credits) treated like a transient 429, so a
    // Kiro account with no credits left rejoins routing after seconds and every
    // retry spends a request (and a failover hop) on a dead account.
    #[test]
    fn out_of_credits_holds_the_connection_for_the_maximum() {
        for retry_after in [None, Some(30)] {
            let mut conn = make_connection();
            let before = chrono::Utc::now();
            let until = conn.record_upstream_error(402, retry_after);
            let after = chrono::Utc::now();
            assert!(
                secs_until(until, before) >= 21_600,
                "402 with retry_after={retry_after:?} must cool for the full 6 h"
            );
            assert!(until <= after + chrono::Duration::seconds(21_600));
        }
    }
}
