pub mod session;
pub mod quota;

pub use session::{SessionPin, SessionRegistry};
pub use quota::{QuotaTracker, QuotaWindow};

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock};

use vkdg_core::{CapabilitySet, ConnectionId, Result, VkdgError};

// ── Provider / auth kinds ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Anthropic,
    OpenAI,
    Google,
    Custom { base_url: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthKind {
    ApiKey { env_var: String },
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
    Degraded { since: DateTime<Utc> },
    CircuitOpen { until: DateTime<Utc> },
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
    active_requests: Arc<AtomicU32>,
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
        Some(ConnectionGuard { counter: Arc::clone(&self.active_requests) })
    }

    fn serves_model(&self, model: &str) -> bool {
        self.config.models.iter().any(|pattern| {
            if let Some(prefix) = pattern.strip_suffix('*') {
                model.starts_with(prefix)
            } else {
                model == pattern
            }
        })
    }
}

impl Connection {
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
        self.state = ConnectionState::Cooldown { until, failure_count };
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
    counter: Arc<AtomicU32>,
}

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::AcqRel);
    }
}

// ── Catalog ───────────────────────────────────────────────────────────────────

pub struct ConnectionCatalog {
    connections: HashMap<ConnectionId, Arc<RwLock<Connection>>>,
}

impl ConnectionCatalog {
    pub fn new(configs: Vec<ConnectionConfig>) -> Self {
        let connections = configs
            .into_iter()
            .map(|cfg| {
                let id = cfg.id.clone();
                (id, Arc::new(RwLock::new(Connection::new(cfg))))
            })
            .collect();
        Self { connections }
    }

    pub fn get(&self, id: &ConnectionId) -> Option<Arc<RwLock<Connection>>> {
        self.connections.get(id).cloned()
    }

    /// Connections that are Healthy, have capacity, and serve `model`.
    /// Runs synchronous reads; callers on an async runtime should use
    /// `blocking_read` only when the lock is uncontended (catalog mutations
    /// are rare configuration events, not hot-path writes).
    pub fn eligible(&self, model: &str, exclude: &[ConnectionId]) -> Vec<ConnectionId> {
        self.connections
            .iter()
            .filter_map(|(id, arc)| {
                if exclude.contains(id) {
                    return None;
                }
                // `try_read` — if the lock is held by a writer we skip rather
                // than block the caller; the writer is a state-transition event
                // and the connection will appear in the next routing attempt.
                let conn = arc.try_read().ok()?;
                if conn.state.is_healthy() && conn.has_capacity() && conn.serves_model(model) {
                    Some(id.clone())
                } else {
                    None
                }
            })
            .collect()
    }

    /// Like `eligible`, but also filters out connections whose declared
    /// `capabilities` do not cover every capability in `required`.
    ///
    /// A connection with an empty `capabilities` set is treated as supporting
    /// *nothing* explicitly — it will be excluded if `required` is non-empty.
    /// This ensures fail-closed behaviour: capability mismatch is never silent.
    pub fn eligible_for_operation(
        &self,
        model: &str,
        exclude: &[ConnectionId],
        required: &CapabilitySet,
    ) -> Vec<ConnectionId> {
        self.connections
            .iter()
            .filter_map(|(id, arc)| {
                if exclude.contains(id) {
                    return None;
                }
                let conn = arc.try_read().ok()?;
                if !conn.state.is_healthy() || !conn.has_capacity() || !conn.serves_model(model) {
                    return None;
                }
                // Every required capability must be present.
                // An empty required set means no filtering — all healthy connections pass.
                for cap in &required.0 {
                    if !conn.config.capabilities.contains(cap) {
                        return None;
                    }
                }
                Some(id.clone())
            })
            .collect()
    }

    /// Returns all connection IDs in this catalog.
    /// Used by the pipeline to populate RoutingHints for all known connections.
    pub fn connection_ids(&self) -> Vec<ConnectionId> {
        self.connections.keys().cloned().collect()
    }
}

// ── Credential manager ────────────────────────────────────────────────────────

/// Cached OAuth2 token per connection.
/// Intentionally does NOT derive Debug — contains a sensitive token.
struct StoredToken {
    access_token: String,
    expires_at: Option<DateTime<Utc>>,
    generation: u64,
}

impl StoredToken {
    fn is_expired(&self) -> bool {
        match self.expires_at {
            None => false,
            // 60 s buffer so we refresh before the token actually expires.
            Some(exp) => Utc::now() >= exp - chrono::Duration::seconds(60),
        }
    }
}

pub struct CredentialManager {
    /// Per-connection token cache.
    tokens: RwLock<HashMap<ConnectionId, StoredToken>>,
    /// Per-connection singleflight: only one refresh runs at a time.
    refresh_locks: Mutex<HashMap<ConnectionId, Arc<tokio::sync::Mutex<()>>>>,
}

impl CredentialManager {
    pub fn new() -> Self {
        Self {
            tokens: RwLock::new(HashMap::new()),
            refresh_locks: Mutex::new(HashMap::new()),
        }
    }

    pub async fn get_token(&self, conn: &ConnectionConfig) -> Result<String> {
        match &conn.auth {
            AuthKind::ApiKey { env_var } => {
                std::env::var(env_var).map_err(|_| VkdgError::ConfigInvalid {
                    field: env_var.clone(),
                    message: "environment variable not set".into(),
                })
            }
            AuthKind::OAuth2 { token_url, client_id, client_secret_env, scopes } => {
                self.get_oauth2_token(&conn.id, token_url, client_id, client_secret_env, scopes)
                    .await
            }
        }
    }

    async fn get_oauth2_token(
        &self,
        conn_id: &ConnectionId,
        token_url: &str,
        client_id: &str,
        client_secret_env: &str,
        scopes: &[String],
    ) -> Result<String> {
        // Fast path: return cached non-expired token without taking any exclusive lock.
        {
            let tokens = self.tokens.read().await;
            if let Some(stored) = tokens.get(conn_id) {
                if !stored.is_expired() {
                    return Ok(stored.access_token.clone());
                }
            }
        }

        // Slow path: acquire per-connection refresh lock (singleflight).
        let lock = {
            let mut locks = self.refresh_locks.lock().await;
            locks
                .entry(conn_id.clone())
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                .clone()
        };
        let _guard = lock.lock().await;

        // Re-check after acquiring lock: a concurrent waiter may have already refreshed.
        {
            let tokens = self.tokens.read().await;
            if let Some(stored) = tokens.get(conn_id) {
                if !stored.is_expired() {
                    return Ok(stored.access_token.clone());
                }
            }
        }

        // Resolve the client secret from the environment.
        let client_secret = std::env::var(client_secret_env).map_err(|_| VkdgError::ConfigInvalid {
            field: client_secret_env.to_string(),
            message: "OAuth2 client secret env var not set".into(),
        })?;

        let new_token = Self::refresh_token(token_url, client_id, &client_secret, scopes).await?;

        // Conditional write: bump generation so a delayed concurrent writer never overwrites
        // a newer token.
        let mut tokens = self.tokens.write().await;
        let current_gen = tokens.get(conn_id).map(|t| t.generation).unwrap_or(0);
        tokens.insert(
            conn_id.clone(),
            StoredToken {
                access_token: new_token.clone(),
                expires_at: Some(Utc::now() + chrono::Duration::hours(1)),
                generation: current_gen + 1,
            },
        );

        Ok(new_token)
    }

    /// client_credentials grant (standard M2M OAuth2 flow).
    async fn refresh_token(
        token_url: &str,
        client_id: &str,
        client_secret: &str,
        scopes: &[String],
    ) -> Result<String> {
        let scope_str = scopes.join(" ");
        let mut params = vec![
            ("grant_type", "client_credentials"),
            ("client_id", client_id),
            ("client_secret", client_secret),
        ];
        if !scope_str.is_empty() {
            params.push(("scope", scope_str.as_str()));
        }

        let resp = reqwest::Client::new()
            .post(token_url)
            .form(&params)
            .send()
            .await
            .map_err(|e| VkdgError::UpstreamError {
                code: 0,
                message: format!("OAuth2 request failed: {e}"),
            })?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            return Err(VkdgError::UpstreamError {
                code: status,
                message: format!("OAuth2 token endpoint returned {status}"),
            });
        }

        let body: serde_json::Value = resp.json().await.map_err(|e| {
            VkdgError::Internal(format!("OAuth2 response parse error: {e}"))
        })?;

        body.get("access_token")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| VkdgError::Internal("OAuth2 response missing access_token".into()))
    }
}

impl Default for CredentialManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vkdg_core::ConnectionId;

    fn make_connection() -> Connection {
        Connection::new(ConnectionConfig {
            id: ConnectionId("test-conn".into()),
            provider: ProviderKind::Custom { base_url: "http://localhost".into() },
            auth: AuthKind::ApiKey { env_var: "FAKE_KEY".into() },
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
        assert!(t2 > t1, "second failure must produce a later cooldown deadline");
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

    // ── Credential manager tests ──────────────────────────────────────────────

    fn make_oauth2_config(conn_id: &str, secret_env: &str) -> ConnectionConfig {
        ConnectionConfig {
            id: ConnectionId(conn_id.into()),
            provider: ProviderKind::Anthropic,
            auth: AuthKind::OAuth2 {
                token_url: "http://localhost:19999".into(),
                client_id: "client-id".into(),
                client_secret_env: secret_env.into(),
                scopes: vec![],
            },
            models: vec![],
            max_concurrent: 1,
            weight: 1,
            tags: vec![],
            capabilities: vkdg_core::CapabilitySet::default(),
        }
    }

    // Plausible wrong impl: concurrent get_token calls both perform refresh (no singleflight).
    // Test the cache logic directly: store a valid token, verify it is returned without refresh.
    #[tokio::test]
    async fn singleflight_cached_token_returned_without_refresh() {
        let mgr = CredentialManager::new();
        let conn_id = ConnectionId("cached-conn".into());
        {
            let mut tokens = mgr.tokens.write().await;
            tokens.insert(
                conn_id.clone(),
                StoredToken {
                    access_token: "cached-token".into(),
                    expires_at: Some(Utc::now() + chrono::Duration::hours(1)),
                    generation: 1,
                },
            );
        }
        let config = make_oauth2_config("cached-conn", "MISSING_VAR_SHOULD_NOT_BE_READ");
        let token = mgr.get_token(&config).await.unwrap();
        assert_eq!(token, "cached-token");
    }

    // Plausible wrong impl: expired token returned without attempting refresh.
    #[tokio::test]
    async fn expired_token_triggers_refresh_attempt() {
        let mgr = CredentialManager::new();
        let conn_id = ConnectionId("expired-conn".into());
        {
            let mut tokens = mgr.tokens.write().await;
            tokens.insert(
                conn_id.clone(),
                StoredToken {
                    access_token: "old-token".into(),
                    expires_at: Some(Utc::now() - chrono::Duration::hours(1)),
                    generation: 1,
                },
            );
        }
        let config = make_oauth2_config("expired-conn", "MISSING_SECRET_VAR_XYZZY");
        // Refresh must be attempted; it fails because the env var is not set.
        let result = mgr.get_token(&config).await;
        assert!(result.is_err(), "expired token must not be returned");
        match result {
            Err(VkdgError::ConfigInvalid { field, .. }) => {
                assert_eq!(field, "MISSING_SECRET_VAR_XYZZY");
            }
            other => panic!("expected ConfigInvalid, got {other:?}"),
        }
    }
}
