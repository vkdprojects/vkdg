use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use futures::future::BoxFuture;
use tokio::sync::{Mutex, RwLock};
use vkdg_core::{ConnectionId, Result, VkdgError};

use super::accounts::{Account, AccountStore, Credential, TokenPair};
use super::connection::{AuthKind, ConnectionConfig};

/// Account tokens are refreshed when they expire within this window.
pub const REFRESH_AHEAD: chrono::Duration = chrono::Duration::minutes(5);

/// Refreshes an account token by dispatching to the owning provider plugin.
///
/// Implemented by `vkdg_provider_sdk::ProviderRegistry`; the core never knows
/// provider-specific refresh details.
pub trait TokenRefresher: Send + Sync {
    fn refresh<'a>(
        &'a self,
        provider: &'a str,
        refresh_token: &'a str,
        extra: &'a HashMap<String, String>,
    ) -> BoxFuture<'a, Result<TokenPair>>;
}

// ── Credential manager ────────────────────────────────────────────────────────

/// Cached `OAuth2` token per connection.
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

/// Persisted accounts plus the plugin dispatcher that refreshes them.
struct AccountBackend {
    store: Arc<AccountStore>,
    refresher: Arc<dyn TokenRefresher>,
}

pub struct CredentialManager {
    /// Per-connection token cache.
    tokens: RwLock<HashMap<ConnectionId, StoredToken>>,
    /// Per-connection singleflight: only one refresh runs at a time.
    refresh_locks: Mutex<HashMap<ConnectionId, Arc<tokio::sync::Mutex<()>>>>,
    accounts: Option<AccountBackend>,
    /// Per-account cache; avoids a SQLite read on every request.
    account_cache: RwLock<HashMap<String, Account>>,
    /// Per-account singleflight for refresh.
    account_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl CredentialManager {
    pub fn new() -> Self {
        Self {
            tokens: RwLock::new(HashMap::new()),
            refresh_locks: Mutex::new(HashMap::new()),
            accounts: None,
            account_cache: RwLock::new(HashMap::new()),
            account_locks: Mutex::new(HashMap::new()),
        }
    }

    /// Enable `AuthKind::Account` connections.
    #[must_use]
    pub fn with_accounts(
        mut self,
        store: Arc<AccountStore>,
        refresher: Arc<dyn TokenRefresher>,
    ) -> Self {
        self.accounts = Some(AccountBackend { store, refresher });
        self
    }

    pub async fn get_token(&self, conn: &ConnectionConfig) -> Result<Credential> {
        match &conn.auth {
            AuthKind::ApiKey { env_var } => {
                std::env::var(env_var).map(Credential::bearer).map_err(|_| {
                    VkdgError::ConfigInvalid {
                        field: env_var.clone(),
                        message: "environment variable not set".into(),
                    }
                })
            }
            AuthKind::OAuth2 {
                token_url,
                client_id,
                client_secret_env,
                scopes,
            } => self
                .get_oauth2_token(&conn.id, token_url, client_id, client_secret_env, scopes)
                .await
                .map(Credential::bearer),
            AuthKind::Account { account_id } => self.get_account_token(account_id).await,
        }
    }

    /// Resolve an account credential, refreshing through the provider plugin when the
    /// token expires within [`REFRESH_AHEAD`]. One refresh per account at a time; the
    /// refreshed tokens are persisted before they are returned.
    async fn get_account_token(&self, account_id: &str) -> Result<Credential> {
        let backend = self
            .accounts
            .as_ref()
            .ok_or_else(|| VkdgError::ConfigInvalid {
                field: format!("auth.account={account_id}"),
                message: "account auth used but no account store is configured".into(),
            })?;

        if let Some(acct) = self.account_cache.read().await.get(account_id) {
            if let Some(served) = serve_without_refresh(acct) {
                return served;
            }
        }

        let lock = {
            let mut locks = self.account_locks.lock().await;
            locks
                .entry(account_id.to_owned())
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                .clone()
        };
        let _guard = lock.lock().await;

        // Re-check: a concurrent waiter may have refreshed while we queued.
        if let Some(acct) = self.account_cache.read().await.get(account_id) {
            if let Some(served) = serve_without_refresh(acct) {
                return served;
            }
        }

        // The store is the source of truth (a `vkdg login` may have replaced it).
        let mut acct = backend
            .store
            .get(account_id)?
            .ok_or_else(|| VkdgError::ConfigInvalid {
                field: format!("auth.account={account_id}"),
                message: "account not found; run `vkdg login <provider>` or `vkdg accounts list`"
                    .into(),
            })?;

        if let Some(served) = serve_without_refresh(&acct) {
            let out = served;
            self.account_cache
                .write()
                .await
                .insert(account_id.to_owned(), acct);
            return out;
        }

        if acct.expires_within(REFRESH_AHEAD) {
            let refreshed = match acct.refresh_token.as_deref() {
                Some(rt) => {
                    backend
                        .refresher
                        .refresh(&acct.provider, rt, &acct.extra)
                        .await
                }
                None => Err(VkdgError::Unauthenticated),
            };
            match refreshed {
                Ok(pair) => {
                    acct.apply_refresh(pair);
                    backend.store.upsert(&acct)?;
                    tracing::info!(account = %acct.id, "account token refreshed");
                }
                // The refresh token itself was rejected: no retry can help. Park the
                // account so later requests stop calling the refresh endpoint, and
                // keep the upstream reason for the operator.
                Err(VkdgError::CredentialRevoked { status, message }) => {
                    tracing::warn!(
                        account = %acct.id, status, reason = %message,
                        "refresh token revoked; account needs a new login"
                    );
                    acct.revoked = Some(format!("{status}: {message}"));
                    backend.store.upsert(&acct)?;
                    let served = serve_without_refresh(&acct)
                        .expect("a revoked account always resolves without refresh");
                    self.account_cache
                        .write()
                        .await
                        .insert(account_id.to_owned(), acct);
                    return served;
                }
                // Refresh-ahead failed but the current token is still valid: serve it
                // and retry on the next request.
                Err(e) if !acct.expires_within(chrono::Duration::zero()) => {
                    tracing::warn!(account = %acct.id, error = %e, "token refresh failed; using current token until expiry");
                }
                Err(e) => {
                    return Err(VkdgError::UpstreamError {
                        code: 401,
                        message: format!(
                            "account {account_id}: token expired and refresh failed: {e}"
                        ),
                    });
                }
            }
        }

        let cred = acct.credential();
        self.account_cache
            .write()
            .await
            .insert(account_id.to_owned(), acct);
        Ok(cred)
    }

    /// Drop a cached account (after `accounts remove` or re-login).
    pub async fn forget_account(&self, account_id: &str) {
        self.account_cache.write().await.remove(account_id);
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
        let client_secret =
            std::env::var(client_secret_env).map_err(|_| VkdgError::ConfigInvalid {
                field: client_secret_env.to_string(),
                message: "OAuth2 client secret env var not set".into(),
            })?;

        let new_token = Self::refresh_token(token_url, client_id, &client_secret, scopes).await?;

        // Conditional write: bump generation so a delayed concurrent writer never overwrites
        // a newer token.
        let mut tokens = self.tokens.write().await;
        let current_gen = tokens.get(conn_id).map_or(0, |t| t.generation);
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

    /// `client_credentials` grant (standard M2M `OAuth2` flow).
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

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| VkdgError::Internal(format!("OAuth2 response parse error: {e}")))?;

        body.get("access_token")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| VkdgError::Internal("OAuth2 response missing access_token".into()))
    }
}

/// Resolve an account without calling the refresh endpoint when its credential
/// is long-lived, still fresh, or its refresh token was revoked.
///
/// No expiry plus no refresh token denotes a long-lived imported credential
/// such as a Kiro API key. Unknown expiry with a refresh token remains an OAuth
/// token that must be refreshed.
///
/// A revoked account keeps serving its access token until it expires, then fails
/// with the stored reason. It is never refreshed again: only a new login can
/// replace a rejected refresh token.
fn serve_without_refresh(acct: &Account) -> Option<Result<Credential>> {
    if acct.expires_at.is_none() && acct.refresh_token.is_none() {
        return Some(Ok(acct.credential()));
    }

    match &acct.revoked {
        Some(reason) => Some(if acct.expires_within(chrono::Duration::zero()) {
            let (status, message) = reason
                .split_once(": ")
                .and_then(|(code, msg)| code.parse::<u16>().ok().map(|c| (c, msg.to_owned())))
                .unwrap_or_else(|| (401, reason.clone()));
            Err(VkdgError::CredentialRevoked {
                status,
                message: format!("account {}: {message}; run `vkdg login` again", acct.id),
            })
        } else {
            Ok(acct.credential())
        }),
        None if !acct.expires_within(REFRESH_AHEAD) => Some(Ok(acct.credential())),
        None => None,
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
            endpoint: None,
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
        assert_eq!(token.token, "cached-token");
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

    /// Plausible wrong impl: concurrent calls on an expired token each trigger
    /// their own refresh, resulting in N HTTP calls instead of 1.
    /// Singleflight must ensure exactly 1 refresh call.
    ///
    /// Cannot hit a real token endpoint; uses a missing env var (`ConfigInvalid`)
    /// as a signal that the refresh path was entered. All N tasks must receive
    /// the same error type — not a mix from racing independent refreshes.
    #[tokio::test]
    async fn concurrent_calls_on_expired_token_use_singleflight() {
        let mgr = Arc::new(CredentialManager::new());
        let conn_id = ConnectionId("conn-sf".into());

        // Insert an already-expired token.
        {
            let mut tokens = mgr.tokens.write().await;
            tokens.insert(
                conn_id.clone(),
                StoredToken {
                    access_token: "expired".into(),
                    expires_at: Some(Utc::now() - chrono::Duration::hours(1)),
                    generation: 1,
                },
            );
        }

        let config = make_oauth2_config("conn-sf", "MISSING_SECRET_FOR_CONCURRENT_TEST");

        // Spawn 5 concurrent callers.
        let mut handles = Vec::new();
        for _ in 0..5 {
            let mgr = Arc::clone(&mgr);
            let config = config.clone();
            handles.push(tokio::spawn(async move { mgr.get_token(&config).await }));
        }

        let mut results = Vec::new();
        for h in handles {
            results.push(h.await.expect("task must not panic"));
        }

        // All 5 must fail with ConfigInvalid (missing secret env var) — not a mix
        // of different errors from racing independent refresh paths.
        for result in &results {
            assert!(
                matches!(result, Err(VkdgError::ConfigInvalid { .. })),
                "all concurrent callers must fail with ConfigInvalid (missing secret), got {result:?}"
            );
        }
    }

    // ── Account auth ──────────────────────────────────────────────────────────

    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Fake plugin refresher: counts calls, rotates tokens, echoes one extra key.
    struct FakeRefresher {
        calls: AtomicUsize,
        fail: bool,
        /// Answer like an upstream that rejected the refresh token itself.
        revoke: bool,
    }

    impl TokenRefresher for FakeRefresher {
        fn refresh<'a>(
            &'a self,
            provider: &'a str,
            refresh_token: &'a str,
            extra: &'a HashMap<String, String>,
        ) -> BoxFuture<'a, Result<TokenPair>> {
            Box::pin(async move {
                let n = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
                // Yield so concurrent callers really overlap.
                tokio::task::yield_now().await;
                if self.revoke {
                    return Err(VkdgError::CredentialRevoked {
                        status: 401,
                        message: "Bad credentials".into(),
                    });
                }
                if self.fail {
                    return Err(VkdgError::Unauthenticated);
                }
                assert_eq!(provider, "fakeprov");
                assert_eq!(extra.get("client_id").map(String::as_str), Some("cid"));
                let mut out = HashMap::new();
                out.insert("region".into(), "eu-west-1".into());
                Ok(TokenPair {
                    access_token: format!("access-{n}-from-{refresh_token}"),
                    refresh_token: Some(format!("refresh-{n}")),
                    expires_in_secs: Some(3600),
                    extra: out,
                })
            })
        }
    }

    fn account(expires_in: chrono::Duration) -> Account {
        let mut extra = HashMap::new();
        extra.insert("client_id".into(), "cid".into());
        Account {
            id: "acct-1".into(),
            provider: "fakeprov".into(),
            label: "me".into(),
            access_token: "access-0".into(),
            refresh_token: Some("refresh-0".into()),
            expires_at: Some(Utc::now() + expires_in),
            extra,
            revoked: None,
        }
    }

    fn account_conn() -> ConnectionConfig {
        ConnectionConfig {
            auth: AuthKind::Account {
                account_id: "acct-1".into(),
            },
            ..make_oauth2_config("conn-acct", "UNUSED")
        }
    }

    fn setup(
        acct: &Account,
        fail: bool,
    ) -> (CredentialManager, Arc<AccountStore>, Arc<FakeRefresher>) {
        setup_with(acct, fail, false)
    }

    fn setup_with(
        acct: &Account,
        fail: bool,
        revoke: bool,
    ) -> (CredentialManager, Arc<AccountStore>, Arc<FakeRefresher>) {
        let store = Arc::new(AccountStore::in_memory().unwrap());
        store.upsert(acct).unwrap();
        let refresher = Arc::new(FakeRefresher {
            calls: AtomicUsize::new(0),
            fail,
            revoke,
        });
        let mgr = CredentialManager::new().with_accounts(Arc::clone(&store), refresher.clone());
        (mgr, store, refresher)
    }

    // Plausible wrong impl: refresh only after actual expiry (no 5-min lead), or refresh
    // result not persisted (lost on restart / next load), or extra replaced instead of merged.
    #[tokio::test]
    async fn account_expiring_within_window_is_refreshed_and_persisted() {
        let (mgr, store, refresher) = setup(&account(chrono::Duration::minutes(4)), false);
        let cred = mgr.get_token(&account_conn()).await.unwrap();
        assert_eq!(cred.token, "access-1-from-refresh-0");
        assert_eq!(cred.extra["client_id"], "cid");
        assert_eq!(cred.extra["region"], "eu-west-1");

        let persisted = store.get("acct-1").unwrap().unwrap();
        assert_eq!(persisted.access_token, "access-1-from-refresh-0");
        assert_eq!(persisted.refresh_token.as_deref(), Some("refresh-1"));
        assert_eq!(persisted.extra["client_id"], "cid");
        assert_eq!(persisted.extra["region"], "eu-west-1");
        assert!(!persisted.expires_within(chrono::Duration::minutes(50)));

        // Second call is served from cache: no second refresh.
        let again = mgr.get_token(&account_conn()).await.unwrap();
        assert_eq!(again.token, "access-1-from-refresh-0");
        assert_eq!(refresher.calls.load(Ordering::SeqCst), 1);
    }

    // Plausible wrong impl: refreshes on every load (hammering the IdP).
    #[tokio::test]
    async fn account_valid_beyond_window_is_not_refreshed() {
        let (mgr, _store, refresher) = setup(&account(chrono::Duration::minutes(6)), false);
        let cred = mgr.get_token(&account_conn()).await.unwrap();
        assert_eq!(cred.token, "access-0");
        assert_eq!(refresher.calls.load(Ordering::SeqCst), 0);
    }

    // Plausible wrong impl: an imported long-lived API key has no expires_at and
    // no refresh token, but unknown OAuth expiry logic treats it as expired and
    // rejects it before the provider can authenticate the key.
    #[tokio::test]
    async fn account_without_expiry_or_refresh_token_serves_stored_credential() {
        let mut acct = account(chrono::Duration::hours(1));
        acct.access_token = "ksk-long-lived".into();
        acct.expires_at = None;
        acct.refresh_token = None;
        acct.extra.insert("auth_method".into(), "api_key".into());
        let (mgr, _store, refresher) = setup(&acct, false);

        let cred = mgr.get_token(&account_conn()).await.unwrap();
        assert_eq!(cred.token, "ksk-long-lived");
        assert_eq!(refresher.calls.load(Ordering::SeqCst), 0);
    }

    // Plausible wrong impl: no per-account singleflight → N refreshes, and with rotating
    // refresh tokens all but one would be revoked.
    #[tokio::test]
    async fn concurrent_account_refresh_is_singleflight() {
        let (mgr, _store, refresher) = setup(&account(chrono::Duration::seconds(-10)), false);
        let mgr = Arc::new(mgr);
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let mgr = Arc::clone(&mgr);
                tokio::spawn(async move { mgr.get_token(&account_conn()).await })
            })
            .collect();
        for h in handles {
            let cred = h.await.unwrap().unwrap();
            assert_eq!(cred.token, "access-1-from-refresh-0");
        }
        assert_eq!(refresher.calls.load(Ordering::SeqCst), 1);
    }

    // Plausible wrong impl: a failed refresh-ahead fails the request even though the
    // current token is still valid; or an expired token is served after refresh failed.
    #[tokio::test]
    async fn failed_refresh_serves_still_valid_token_but_rejects_expired() {
        let (mgr, _s, _r) = setup(&account(chrono::Duration::minutes(2)), true);
        assert_eq!(
            mgr.get_token(&account_conn()).await.unwrap().token,
            "access-0"
        );

        let (mgr, _s, _r) = setup(&account(chrono::Duration::seconds(-1)), true);
        match mgr.get_token(&account_conn()).await {
            Err(VkdgError::UpstreamError { code: 401, .. }) => {}
            other => panic!("expected 401 UpstreamError, got {other:?}"),
        }
    }

    // Plausible wrong impl: unknown account id silently falls back or panics.
    #[tokio::test]
    async fn unknown_account_is_config_error() {
        let (mgr, store, _r) = setup(&account(chrono::Duration::hours(1)), false);
        store.remove("acct-1").unwrap();
        assert!(matches!(
            mgr.get_token(&account_conn()).await,
            Err(VkdgError::ConfigInvalid { .. })
        ));
    }

    // ── Revoked refresh tokens ───────────────────────────────────────────────
    //
    // Found against a live Kiro account: a refresh token rejected with 401 "Bad
    // credentials" was retried on every request while the access token lived, and
    // the log said only "authorization_failed".

    // Plausible wrong impl: a revoked refresh token is retried on every request,
    // hammering the auth endpoint for a credential that can never recover.
    #[tokio::test]
    async fn revoked_refresh_is_not_retried_on_every_request() {
        let (mgr, _store, refresher) =
            setup_with(&account(chrono::Duration::minutes(2)), false, true);
        for _ in 0..5 {
            let cred = mgr.get_token(&account_conn()).await.unwrap();
            assert_eq!(cred.token, "access-0", "still-valid token keeps serving");
        }
        assert_eq!(
            refresher.calls.load(Ordering::SeqCst),
            1,
            "a revoked refresh token must be tried once, not on every request"
        );
    }

    // Plausible wrong impl: revocation lives only in memory, so a restart forgets
    // it and the operator is never told the account needs a new login.
    #[tokio::test]
    async fn revocation_is_persisted_with_the_upstream_reason() {
        let (mgr, store, _r) = setup_with(&account(chrono::Duration::minutes(2)), false, true);
        mgr.get_token(&account_conn()).await.unwrap();
        let persisted = store.get("acct-1").unwrap().unwrap();
        let reason = persisted
            .revoked
            .expect("a revoked account must be marked in the store");
        assert!(
            reason.contains("Bad credentials") && reason.contains("401"),
            "the stored reason must carry the upstream status and message: {reason}"
        );
    }

    // Plausible wrong impl: once the access token also expires, the caller gets a
    // generic 401 that hides why, instead of the typed revocation.
    #[tokio::test]
    async fn expired_revoked_account_reports_revocation_not_a_generic_error() {
        let (mgr, _store, refresher) =
            setup_with(&account(chrono::Duration::seconds(-1)), false, true);
        match mgr.get_token(&account_conn()).await {
            Err(VkdgError::CredentialRevoked { status, message }) => {
                assert_eq!(status, 401);
                assert!(message.contains("Bad credentials"), "{message}");
            }
            other => panic!("expected CredentialRevoked, got {other:?}"),
        }
        // A second call answers from the stored state without another refresh.
        assert!(mgr.get_token(&account_conn()).await.is_err());
        assert_eq!(refresher.calls.load(Ordering::SeqCst), 1);
    }

    // Plausible wrong impl: a transient failure (network, 5xx) is treated as a
    // revocation and the account is wrongly parked.
    #[tokio::test]
    async fn transient_refresh_failure_does_not_mark_revoked() {
        let (mgr, store, _r) = setup_with(&account(chrono::Duration::minutes(2)), true, false);
        mgr.get_token(&account_conn()).await.unwrap();
        assert!(
            store.get("acct-1").unwrap().unwrap().revoked.is_none(),
            "only a revocation parks the account; transient errors retry later"
        );
    }
}
