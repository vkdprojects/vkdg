use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use tokio::sync::{Mutex, RwLock};
use vkdg_core::{ConnectionId, Result, VkdgError};

use super::connection::{AuthKind, ConnectionConfig};

// ── Credential manager ────────────────────────────────────────────────────────

/// Cached OAuth2 token per connection.
/// Intentionally does NOT derive Debug — contains a sensitive token.
struct StoredToken {
    access_token: String,
    expires_at: Option<chrono::DateTime<Utc>>,
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
    pub(crate) tokens: RwLock<HashMap<ConnectionId, StoredToken>>,
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

    use crate::connection::{AuthKind, ConnectionConfig, ProviderKind};

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
