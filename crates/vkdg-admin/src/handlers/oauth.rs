//! Provider login (OAuth device code / PKCE / token import) and account management.
//!
//! Plugin login state (device codes, PKCE verifiers) stays server-side, keyed by an
//! opaque `login_id`; responses never contain access or refresh tokens.

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    extract::{Path, State},
    response::{IntoResponse, Response},
    Json,
};
use chrono::{DateTime, Utc};
use http::{HeaderMap, StatusCode};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use vkdg_connections::{Account, AccountStore, CredentialManager};
use vkdg_provider_sdk::{
    find_login_method, resolve_login_params, DevicePoll, LoginParams, LoginResult, LoginState,
    OAuthFlow, OAuthProvider, ProviderAdapter, ProviderError, ProviderRegistry, UsageSnapshot,
};

use crate::{
    error::{AdminError, AdminErrorResponse},
    handlers::session::get_session,
    router::AdminState,
};

/// Pending logins are discarded after this long even if the plugin said otherwise.
const MAX_PENDING_SECS: i64 = 30 * 60;

struct PendingLogin {
    provider: String,
    method: String,
    flow: OAuthFlow,
    state: LoginState,
    expires_at: DateTime<Utc>,
    /// Existing account a reconnect will overwrite.
    replaces: Option<String>,
}

/// Everything the login/account endpoints need.
pub struct LoginService {
    pub registry: Arc<ProviderRegistry>,
    pub store: Arc<AccountStore>,
    /// When present, removed/re-logged accounts are evicted from the live cache.
    pub credentials: Option<Arc<CredentialManager>>,
    pending: Mutex<HashMap<String, PendingLogin>>,
    /// Per-account credit usage, memoised so listing accounts does not hit the
    /// upstream on every call. Holds both hits and misses for [`USAGE_TTL`].
    usage_cache: Mutex<HashMap<String, CachedUsage>>,
}

/// How long a credit reading (success or failure) is reused before the next
/// list refetches it. Credit balances move slowly; a few minutes is plenty.
const USAGE_TTL: chrono::Duration = chrono::Duration::minutes(5);

#[derive(Clone)]
struct CachedUsage {
    checked_at: DateTime<Utc>,
    /// `Some` when the upstream reported usage, `None` when it did not.
    snapshot: Option<UsageSnapshot>,
}

impl LoginService {
    pub fn new(
        registry: Arc<ProviderRegistry>,
        store: Arc<AccountStore>,
        credentials: Option<Arc<CredentialManager>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            registry,
            store,
            credentials,
            pending: Mutex::new(HashMap::new()),
            usage_cache: Mutex::new(HashMap::new()),
        })
    }

    fn insert_pending(&self, p: PendingLogin) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();
        let mut pending = self.pending.lock();
        pending.retain(|_, v| v.expires_at > now);
        pending.insert(id.clone(), p);
        id
    }

    /// Drop the live cached credential, so a reconnected (or replaced) account
    /// is served from the store, not from a revoked cache entry. Also drops the
    /// cached credit reading so the next list reflects the new tokens.
    async fn evict(&self, account_id: &str) {
        self.usage_cache.lock().remove(account_id);
        if let Some(creds) = &self.credentials {
            creds.forget_account(account_id).await;
        }
    }

    /// Current-period credit usage for one account, memoised for [`USAGE_TTL`].
    ///
    /// Returns `Some(snapshot)` when the provider reported usage, `None` when it
    /// does not sell credits, has no live token, or the upstream failed — the
    /// caller renders the last two as `unavailable`, never a fabricated 0. Only
    /// the account's own credential is used; nothing is shared between accounts.
    async fn usage_for(&self, account: &Account) -> UsageOutcome {
        let Some(adapter) = self.registry.get(&account.provider) else {
            return UsageOutcome::NotCapable;
        };
        if adapter.usage().is_none() {
            return UsageOutcome::NotCapable;
        }
        let now = Utc::now();
        if let Some(hit) = self.usage_cache.lock().get(&account.id) {
            if now - hit.checked_at < USAGE_TTL {
                return UsageOutcome::Read(hit.checked_at, hit.snapshot.clone());
            }
        }
        let usage = adapter.usage().expect("checked above");
        let snapshot = usage
            .fetch_usage(&account.credential())
            .await
            .map_err(|e| eprintln!("credit usage fetch failed for {}: {e}", account.id))
            .ok();
        self.usage_cache.lock().insert(
            account.id.clone(),
            CachedUsage {
                checked_at: now,
                snapshot: snapshot.clone(),
            },
        );
        UsageOutcome::Read(now, snapshot)
    }

    fn save(
        &self,
        provider: &str,
        result: LoginResult,
        replaces: Option<&str>,
    ) -> Result<Account, Box<Response>> {
        let label = if result.label.is_empty() {
            provider
        } else {
            result.label.as_str()
        };
        let mut account = Account::from_token_pair(provider, label, result.tokens);
        // Reconnect: same id, so connections referencing it recover. The old row
        // must belong to this provider; anything else is a new account.
        if let Some(id) = replaces {
            match self.store.get(id) {
                Ok(Some(old)) if old.provider == provider => account.id = old.id,
                Ok(_) => {
                    return Err(Box::new(err(
                        StatusCode::BAD_REQUEST,
                        "invalid_input",
                        format!("account {id} is not a {provider} account"),
                    )))
                }
                Err(e) => {
                    return Err(Box::new(err(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "store_error",
                        e.to_string(),
                    )))
                }
            }
        }
        self.store.upsert(&account).map_err(|e| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "store_error",
                e.to_string(),
            )
        })?;
        Ok(account)
    }
}

/// What [`LoginService::usage_for`] found for one account.
enum UsageOutcome {
    /// The provider does not sell credits; the summary carries no credit fields.
    NotCapable,
    /// A usage read completed at the given time. `Some` = the upstream reported
    /// figures; `None` = it did not (surfaced as `unavailable`, never 0).
    Read(DateTime<Utc>, Option<UsageSnapshot>),
}

// ── Wire types ────────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct AccountSummary {
    id: String,
    provider: String,
    label: String,
    expires_at: Option<String>,
    has_refresh_token: bool,
    /// `active`, or `needs_login` once the upstream rejected the refresh token.
    status: &'static str,
    /// Upstream reason for the revocation (e.g. `401: ... Bad credentials`).
    #[serde(skip_serializing_if = "Option::is_none")]
    revoked_reason: Option<String>,
    /// Credit reporting for providers that sell credits. `reported` when the
    /// upstream answered, `unavailable` when it did not; absent for providers
    /// that bill some other way. A missing figure is `null`, never a fabricated 0.
    #[serde(skip_serializing_if = "Option::is_none")]
    credits_source: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    credits_used: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    credits_limit: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    credits_period_end: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    credits_plan: Option<String>,
    /// RFC3339 time the credit figures were last read from the upstream.
    #[serde(skip_serializing_if = "Option::is_none")]
    credits_checked_at: Option<String>,
    /// Opaque fingerprint of the upstream user, when reported.
    #[serde(skip_serializing_if = "Option::is_none")]
    credits_user_ref: Option<String>,
}

impl From<&Account> for AccountSummary {
    fn from(a: &Account) -> Self {
        Self {
            id: a.id.clone(),
            provider: a.provider.clone(),
            label: a.label.clone(),
            expires_at: a.expires_at.map(|t| t.to_rfc3339()),
            has_refresh_token: a.refresh_token.is_some(),
            status: if a.revoked.is_some() {
                "needs_login"
            } else {
                "active"
            },
            revoked_reason: a.revoked.clone(),
            credits_source: None,
            credits_used: None,
            credits_limit: None,
            credits_period_end: None,
            credits_plan: None,
            credits_checked_at: None,
            credits_user_ref: None,
        }
    }
}

impl AccountSummary {
    /// Fold a fresh usage read (or its absence) onto the summary. `Some(snap)`
    /// marks the source `reported` and copies every figure the provider gave;
    /// `None` marks it `unavailable` and leaves the figures `null` — the caller
    /// passes `None` on any upstream failure so a missing value is never 0.
    fn with_usage(mut self, checked_at: &str, usage: Option<&UsageSnapshot>) -> Self {
        match usage {
            Some(u) => {
                self.credits_source = Some("reported");
                self.credits_used = Some(u.credits_used);
                self.credits_limit = u.credits_limit;
                self.credits_period_end = u.credits_period_end;
                self.credits_plan.clone_from(&u.plan);
                self.credits_user_ref.clone_from(&u.upstream_user_ref);
            }
            None => {
                self.credits_source = Some("unavailable");
            }
        }
        self.credits_checked_at = Some(checked_at.to_owned());
        self
    }
}

#[derive(Deserialize, Default)]
pub struct StartBody {
    pub method: Option<String>,
    #[serde(default)]
    pub params: LoginParams,
    /// Reconnect: replace the tokens of this existing account and keep its id,
    /// so connections that reference it (`auth: { type: account }`) recover.
    #[serde(default)]
    pub account_id: Option<String>,
}

#[derive(Deserialize)]
pub struct PollBody {
    pub login_id: String,
    /// PKCE only: authorization code or pasted callback URL.
    pub code: Option<String>,
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn err(status: StatusCode, code: &str, msg: impl Into<String>) -> Response {
    AdminErrorResponse(status, AdminError::new(code, msg)).into_response()
}

fn provider_err(e: &ProviderError) -> Response {
    match e {
        ProviderError::UnsupportedOperation => err(
            StatusCode::BAD_REQUEST,
            "unsupported",
            "login method not supported by this provider",
        ),
        ProviderError::Config(m) => err(StatusCode::BAD_REQUEST, "invalid_input", m.clone()),
        other => err(StatusCode::BAD_GATEWAY, "provider_error", other.to_string()),
    }
}

type Resolved = (Arc<LoginService>, Arc<dyn ProviderAdapter>);

/// Session check + login service + OAuth-capable adapter.
fn resolve(
    state: &AdminState,
    headers: &HeaderMap,
    provider: &str,
) -> Result<Resolved, Box<Response>> {
    if get_session(state, headers).is_none() {
        return Err(Box::new(
            AdminErrorResponse(StatusCode::UNAUTHORIZED, AdminError::unauthorized())
                .into_response(),
        ));
    }
    let svc = state.logins.clone().ok_or_else(|| {
        Box::new(err(
            StatusCode::SERVICE_UNAVAILABLE,
            "accounts_disabled",
            "account store is not configured",
        ))
    })?;
    let adapter = svc
        .registry
        .get(provider)
        .filter(|a| a.oauth().is_some())
        .ok_or_else(|| {
            Box::new(err(
                StatusCode::NOT_FOUND,
                "not_found",
                format!("no OAuth-capable provider '{provider}'"),
            ))
        })?;
    Ok((svc, adapter))
}

fn oauth(adapter: &dyn ProviderAdapter) -> &dyn OAuthProvider {
    // `resolve` filters out adapters without OAuth.
    adapter.oauth().expect("resolve() checked oauth()")
}

fn done(account: &Account) -> Response {
    Json(serde_json::json!({ "status": "done", "account": AccountSummary::from(account) }))
        .into_response()
}

// ── Handlers ──────────────────────────────────────────────────────────────────

/// `GET /admin/v1/providers/oauth`: providers that support interactive login,
/// so the console lists what this gateway actually has, plugins included.
pub async fn list_oauth_providers(State(state): State<AdminState>, headers: HeaderMap) -> Response {
    if get_session(&state, &headers).is_none() {
        return AdminErrorResponse(StatusCode::UNAUTHORIZED, AdminError::unauthorized())
            .into_response();
    }
    let Some(svc) = state.logins else {
        return err(
            StatusCode::SERVICE_UNAVAILABLE,
            "accounts_disabled",
            "account store is not configured",
        );
    };
    let mut ids: Vec<String> = svc.registry.ids();
    ids.sort_unstable();
    let items: Vec<serde_json::Value> = ids
        .iter()
        .filter_map(|id| svc.registry.get(id))
        .filter(|a| a.oauth().is_some())
        .map(|a| {
            let meta = a.meta();
            serde_json::json!({
                "id": a.id(),
                "display_name": a.display_name(),
                "icon_char": meta.icon_char,
                "icon_color": meta.icon_color,
                "category": meta.category,
                "site_url": meta.site_url,
                "description": meta.description,
            })
        })
        .collect();
    Json(serde_json::json!({ "items": items })).into_response()
}

pub async fn list_login_methods(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Path(provider): Path<String>,
) -> Response {
    let (_svc, adapter) = match resolve(&state, &headers, &provider) {
        Ok(v) => v,
        Err(r) => return *r,
    };
    let items = oauth(adapter.as_ref()).login_methods();
    Json(serde_json::json!({ "provider": provider, "items": items })).into_response()
}

pub async fn start_login(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Path(provider): Path<String>,
    Json(body): Json<StartBody>,
) -> Response {
    let (svc, adapter) = match resolve(&state, &headers, &provider) {
        Ok(v) => v,
        Err(r) => return *r,
    };
    let oauth = oauth(adapter.as_ref());
    let method = match find_login_method(oauth, body.method.as_deref()) {
        Ok(m) => m,
        Err(e) => return provider_err(&e),
    };
    let params = match resolve_login_params(&method, &body.params) {
        Ok(p) => p,
        Err(e) => return provider_err(&e),
    };
    match method.flow {
        OAuthFlow::DeviceCode => match oauth.start_device_login(&method.id, &params).await {
            Ok(auth) => {
                let ttl = i64::try_from(auth.expires_in_secs)
                    .unwrap_or(MAX_PENDING_SECS)
                    .min(MAX_PENDING_SECS);
                let login_id = svc.insert_pending(PendingLogin {
                    provider: provider.clone(),
                    method: method.id.clone(),
                    flow: method.flow,
                    state: auth.state,
                    expires_at: Utc::now() + chrono::Duration::seconds(ttl),
                    replaces: body.account_id.clone(),
                });
                Json(serde_json::json!({
                    "login_id": login_id,
                    "flow": method.flow,
                    "verification_uri": auth.verification_uri,
                    "verification_uri_complete": auth.verification_uri_complete,
                    "user_code": auth.user_code,
                    "interval_secs": auth.interval_secs,
                    "expires_in_secs": auth.expires_in_secs,
                }))
                .into_response()
            }
            Err(e) => provider_err(&e),
        },
        OAuthFlow::AuthorizationCodePkce => {
            match oauth.start_pkce_login(&method.id, &params).await {
                Ok(auth) => {
                    let login_id = svc.insert_pending(PendingLogin {
                        provider: provider.clone(),
                        method: method.id.clone(),
                        flow: method.flow,
                        state: auth.state,
                        expires_at: Utc::now() + chrono::Duration::seconds(MAX_PENDING_SECS),
                        replaces: body.account_id.clone(),
                    });
                    Json(serde_json::json!({
                        "login_id": login_id,
                        "flow": method.flow,
                        "authorize_url": auth.authorize_url,
                    }))
                    .into_response()
                }
                Err(e) => provider_err(&e),
            }
        }
        OAuthFlow::ImportToken => err(
            StatusCode::BAD_REQUEST,
            "invalid_input",
            format!("method '{}' is an import; use /import", method.id),
        ),
    }
}

pub async fn poll_login(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Path(provider): Path<String>,
    Json(body): Json<PollBody>,
) -> Response {
    let (svc, adapter) = match resolve(&state, &headers, &provider) {
        Ok(v) => v,
        Err(r) => return *r,
    };
    let oauth = oauth(adapter.as_ref());
    let (method, flow, login_state, replaces) = {
        let pending = svc.pending.lock();
        match pending.get(&body.login_id) {
            Some(p) if p.provider == provider && p.expires_at > Utc::now() => (
                p.method.clone(),
                p.flow,
                p.state.clone(),
                p.replaces.clone(),
            ),
            _ => {
                return err(
                    StatusCode::NOT_FOUND,
                    "not_found",
                    "unknown or expired login_id",
                )
            }
        }
    };

    let outcome = match flow {
        OAuthFlow::AuthorizationCodePkce => {
            let Some(code) = body.code.as_deref().filter(|c| !c.is_empty()) else {
                return err(StatusCode::BAD_REQUEST, "invalid_input", "code is required");
            };
            oauth
                .finish_pkce_login(&method, &login_state, code)
                .await
                .map(DevicePoll::Done)
        }
        _ => oauth.poll_device_login(&method, &login_state).await,
    };

    match outcome {
        Ok(DevicePoll::Pending) => Json(serde_json::json!({ "status": "pending" })).into_response(),
        Ok(DevicePoll::SlowDown) => {
            Json(serde_json::json!({ "status": "slow_down" })).into_response()
        }
        Ok(DevicePoll::Failed(msg)) => {
            svc.pending.lock().remove(&body.login_id);
            Json(serde_json::json!({ "status": "failed", "message": msg })).into_response()
        }
        Ok(DevicePoll::Done(result)) => {
            svc.pending.lock().remove(&body.login_id);
            match svc.save(&provider, result, replaces.as_deref()) {
                Ok(account) => {
                    svc.evict(&account.id).await;
                    done(&account)
                }
                Err(r) => *r,
            }
        }
        Err(e) => provider_err(&e),
    }
}

pub async fn import_token(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Path(provider): Path<String>,
    Json(body): Json<StartBody>,
) -> Response {
    let (svc, adapter) = match resolve(&state, &headers, &provider) {
        Ok(v) => v,
        Err(r) => return *r,
    };
    let oauth = oauth(adapter.as_ref());
    let method = match find_login_method(oauth, body.method.as_deref()) {
        Ok(m) if m.flow == OAuthFlow::ImportToken => m,
        Ok(m) => {
            return err(
                StatusCode::BAD_REQUEST,
                "invalid_input",
                format!("method '{}' is not an import method", m.id),
            )
        }
        Err(e) => return provider_err(&e),
    };
    let params = match resolve_login_params(&method, &body.params) {
        Ok(p) => p,
        Err(e) => return provider_err(&e),
    };
    match oauth.import_token(&method.id, &params).await {
        Ok(result) => match svc.save(&provider, result, body.account_id.as_deref()) {
            Ok(account) => {
                svc.evict(&account.id).await;
                (StatusCode::CREATED, done(&account)).into_response()
            }
            Err(r) => *r,
        },
        Err(e) => provider_err(&e),
    }
}

fn accounts_service(
    state: &AdminState,
    headers: &HeaderMap,
) -> Result<Arc<LoginService>, Box<Response>> {
    if get_session(state, headers).is_none() {
        return Err(Box::new(
            AdminErrorResponse(StatusCode::UNAUTHORIZED, AdminError::unauthorized())
                .into_response(),
        ));
    }
    state.logins.clone().ok_or_else(|| {
        Box::new(err(
            StatusCode::SERVICE_UNAVAILABLE,
            "accounts_disabled",
            "account store is not configured",
        ))
    })
}

pub async fn list_accounts(State(state): State<AdminState>, headers: HeaderMap) -> Response {
    let svc = match accounts_service(&state, &headers) {
        Ok(s) => s,
        Err(r) => return *r,
    };
    match svc.store.list() {
        Ok(accounts) => {
            // Read every account's credit usage concurrently; each read uses only
            // that account's own credential and is memoised for USAGE_TTL.
            let svc = &svc;
            let items: Vec<AccountSummary> =
                futures::future::join_all(accounts.iter().map(|a| async move {
                    let summary = AccountSummary::from(a);
                    match svc.usage_for(a).await {
                        UsageOutcome::NotCapable => summary,
                        UsageOutcome::Read(checked_at, snapshot) => {
                            summary.with_usage(&checked_at.to_rfc3339(), snapshot.as_ref())
                        }
                    }
                }))
                .await;
            let total = items.len();
            Json(serde_json::json!({ "items": items, "total": total })).into_response()
        }
        Err(e) => err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "store_error",
            e.to_string(),
        ),
    }
}

pub async fn delete_account(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let svc = match accounts_service(&state, &headers) {
        Ok(s) => s,
        Err(r) => return *r,
    };
    match svc.store.remove(&id) {
        Ok(true) => {
            if let Some(creds) = &svc.credentials {
                creds.forget_account(&id).await;
            }
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(false) => {
            AdminErrorResponse(StatusCode::NOT_FOUND, AdminError::not_found(&id)).into_response()
        }
        Err(e) => err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "store_error",
            e.to_string(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handlers::requests::RequestLog;
    use crate::session::SessionStore;
    use futures::future::BoxFuture;
    use std::time::Instant;
    use tokio::sync::watch;
    use vkdg_config::ConfigSnapshot;
    use vkdg_connections::{ConnectionConfig, Credential, TokenPair};
    use vkdg_operations::Operation;
    use vkdg_provider_sdk::{
        DeviceAuthorization, LoginField, LoginMethod, OAuthConfig, PreparedRequest,
    };

    /// Fake device-code provider: first poll pending, second done.
    struct FakeDevice {
        polls: Mutex<u32>,
    }

    impl ProviderAdapter for FakeDevice {
        fn id(&self) -> &'static str {
            "fake"
        }
        fn display_name(&self) -> &'static str {
            "Fake"
        }
        fn prepare(
            &self,
            _: &Operation,
            _: &ConnectionConfig,
            _: &Credential,
        ) -> Result<PreparedRequest, ProviderError> {
            Err(ProviderError::UnsupportedOperation)
        }
        fn oauth(&self) -> Option<&dyn OAuthProvider> {
            Some(self)
        }
    }

    impl OAuthProvider for FakeDevice {
        fn oauth_config(&self) -> OAuthConfig {
            OAuthConfig {
                flow: OAuthFlow::DeviceCode,
                authorize_url: None,
                token_url: String::new(),
                client_id: String::new(),
                scopes: vec![],
                redirect_uri: None,
                extra_auth_params: HashMap::new(),
            }
        }
        fn login_methods(&self) -> Vec<LoginMethod> {
            vec![LoginMethod {
                id: "device".into(),
                label: "Device".into(),
                flow: OAuthFlow::DeviceCode,
                fields: vec![LoginField {
                    id: "region".into(),
                    label: "Region".into(),
                    required: true,
                    secret: false,
                    default: None,
                    placeholder: None,
                }],
                hint: None,
                icon_char: None,
            }]
        }
        fn start_device_login<'a>(
            &'a self,
            _: &'a str,
            params: &'a LoginParams,
        ) -> BoxFuture<'a, Result<DeviceAuthorization, ProviderError>> {
            Box::pin(async move {
                Ok(DeviceAuthorization {
                    verification_uri: "https://device.example".into(),
                    verification_uri_complete: None,
                    user_code: "WXYZ-1234".into(),
                    interval_secs: 5,
                    expires_in_secs: 600,
                    state: HashMap::from([
                        ("device_code".into(), "secret-device-code".into()),
                        ("region".into(), params["region"].clone()),
                    ]),
                })
            })
        }
        fn poll_device_login<'a>(
            &'a self,
            _: &'a str,
            state: &'a LoginState,
        ) -> BoxFuture<'a, Result<DevicePoll, ProviderError>> {
            Box::pin(async move {
                assert_eq!(state["device_code"], "secret-device-code");
                let mut n = self.polls.lock();
                *n += 1;
                if *n == 1 {
                    return Ok(DevicePoll::Pending);
                }
                Ok(DevicePoll::Done(LoginResult {
                    tokens: TokenPair {
                        access_token: "secret-access".into(),
                        refresh_token: Some("secret-refresh".into()),
                        expires_in_secs: Some(3600),
                        extra: HashMap::from([("region".into(), state["region"].clone())]),
                    },
                    label: "dev@example.com".into(),
                }))
            })
        }
        fn refresh_token<'a>(
            &'a self,
            _: &'a str,
            _: &'a HashMap<String, String>,
        ) -> BoxFuture<'a, Result<TokenPair, ProviderError>> {
            Box::pin(async { Err(ProviderError::UnsupportedOperation) })
        }
    }

    fn make_state() -> (AdminState, Arc<AccountStore>) {
        let (_tx, rx) = watch::channel(Arc::new(ConfigSnapshot::default_empty()));
        let mut registry = ProviderRegistry::empty();
        registry.register(Arc::new(FakeDevice {
            polls: Mutex::new(0),
        }));
        let store = Arc::new(AccountStore::in_memory().unwrap());
        let state = AdminState {
            sessions: SessionStore::new("tok".into()),
            config_rx: rx,
            started_at: Arc::new(Instant::now()),
            key_store: Arc::new(vkdg_governance::VirtualKeyStore::in_memory().unwrap()),
            request_log: RequestLog::new(),
            combos: None,
            reload_plugins: None,
            connection_tester: None,
            catalog: None,
            logins: Some(LoginService::new(
                Arc::new(registry),
                Arc::clone(&store),
                None,
            )),
            gateway_store: None,
            config_tx: None,
        };
        (state, store)
    }

    fn authed(state: &AdminState) -> HeaderMap {
        let s = state.sessions.bootstrap_login().unwrap();
        let mut h = HeaderMap::new();
        h.insert(
            http::header::COOKIE,
            http::HeaderValue::from_str(&format!("vkdg_session={}", s.session_id)).unwrap(),
        );
        h
    }

    async fn body_json(resp: Response) -> (StatusCode, serde_json::Value, String) {
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        (
            status,
            serde_json::from_str(&text).unwrap_or_default(),
            text,
        )
    }

    fn start_body(region: Option<&str>) -> Json<StartBody> {
        Json(StartBody {
            method: None,
            params: region
                .map(|r| HashMap::from([("region".to_string(), r.to_string())]))
                .unwrap_or_default(),
            account_id: None,
        })
    }

    // Plausible wrong impls: plugin state (device code) leaked to the browser; poll not
    // wired to the stored state; tokens returned in the response; account not persisted
    // with plugin extra; login_id reusable after completion.
    #[tokio::test]
    async fn device_login_start_poll_persists_account_without_leaking_secrets() {
        let (state, store) = make_state();
        let h = authed(&state);

        let (status, start, text) = body_json(
            start_login(
                State(state.clone()),
                h.clone(),
                Path("fake".into()),
                start_body(Some("eu-west-1")),
            )
            .await,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{text}");
        assert_eq!(start["user_code"], "WXYZ-1234");
        assert_eq!(start["flow"], "device_code");
        assert!(!text.contains("secret-device-code"), "{text}");
        let login_id = start["login_id"].as_str().unwrap().to_string();

        let poll = |id: String| {
            poll_login(
                State(state.clone()),
                h.clone(),
                Path("fake".into()),
                Json(PollBody {
                    login_id: id,
                    code: None,
                }),
            )
        };
        let (_, first, _) = body_json(poll(login_id.clone()).await).await;
        assert_eq!(first["status"], "pending");
        assert!(store.list().unwrap().is_empty());

        let (status, second, text) = body_json(poll(login_id.clone()).await).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(second["status"], "done");
        assert!(
            !text.contains("secret-access") && !text.contains("secret-refresh"),
            "{text}"
        );
        let id = second["account"]["id"].as_str().unwrap();
        assert!(id.starts_with("fake-"), "{id}");

        let acct = store.get(id).unwrap().expect("persisted");
        assert_eq!(acct.provider, "fake");
        assert_eq!(acct.label, "dev@example.com");
        assert_eq!(acct.access_token, "secret-access");
        assert_eq!(acct.extra["region"], "eu-west-1");

        let (status, _, _) = body_json(poll(login_id).await).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "login_id is single-use");

        let (status, list, text) =
            body_json(list_accounts(State(state.clone()), h.clone()).await).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(list["total"], 1);
        assert!(!text.contains("secret-"), "{text}");

        let resp = delete_account(State(state.clone()), h.clone(), Path(id.to_string())).await;
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
        assert!(store.list().unwrap().is_empty());
    }

    // Plausible wrong impls: missing required field forwarded to the plugin; unauthenticated
    // caller can start a login; non-OAuth / unknown provider accepted.
    #[tokio::test]
    async fn start_rejects_missing_field_unknown_provider_and_no_session() {
        let (state, _store) = make_state();
        let h = authed(&state);
        let resp = start_login(
            State(state.clone()),
            h.clone(),
            Path("fake".into()),
            start_body(None),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let resp = start_login(
            State(state.clone()),
            h.clone(),
            Path("nope".into()),
            start_body(Some("x")),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let resp = start_login(
            State(state.clone()),
            HeaderMap::new(),
            Path("fake".into()),
            start_body(Some("x")),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        let resp = list_login_methods(State(state.clone()), h.clone(), Path("fake".into())).await;
        let (_, methods, _) = body_json(resp).await;
        assert_eq!(methods["items"][0]["id"], "device");
        assert_eq!(methods["items"][0]["fields"][0]["id"], "region");
    }

    // The console hardcoded six providers, so OAuth plugins (and the e2e fake
    // provider) could never be connected from the UI.
    #[tokio::test]
    async fn oauth_providers_lists_what_the_registry_has() {
        let (state, _) = make_state();
        let h = authed(&state);
        let (status, v, text) = body_json(list_oauth_providers(State(state), h).await).await;
        assert_eq!(status, StatusCode::OK, "{text}");
        let ids: Vec<&str> = v["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["fake"]);
        assert!(v["items"][0]["display_name"].is_string());
    }

    // Found in review: Reconnect minted a new account id, so connections that
    // reference the revoked account stayed broken after a successful login.
    #[tokio::test]
    async fn reconnect_replaces_tokens_and_keeps_the_account_id() {
        let (state, store) = make_state();
        let h = authed(&state);
        let mut revoked = Account::from_token_pair(
            "fake",
            "dev@example.com",
            TokenPair {
                access_token: "old".into(),
                refresh_token: Some("old-refresh".into()),
                expires_in_secs: Some(0),
                extra: HashMap::new(),
            },
        );
        revoked.revoked = Some("401: Bad credentials".into());
        store.upsert(&revoked).unwrap();

        let mut body = start_body(Some("eu-west-1"));
        body.0.account_id = Some(revoked.id.clone());
        let (_, start, _) = body_json(
            start_login(State(state.clone()), h.clone(), Path("fake".into()), body).await,
        )
        .await;
        let login_id = start["login_id"].as_str().unwrap().to_owned();
        let mut done = serde_json::Value::Null;
        for _ in 0..3 {
            let (_, v, _) = body_json(
                poll_login(
                    State(state.clone()),
                    h.clone(),
                    Path("fake".into()),
                    Json(PollBody {
                        login_id: login_id.clone(),
                        code: None,
                    }),
                )
                .await,
            )
            .await;
            if v["status"] == "done" {
                done = v;
                break;
            }
        }
        assert_eq!(done["account"]["id"], revoked.id.as_str(), "{done}");
        assert_eq!(done["account"]["status"], "active", "{done}");
        let accounts = store.list().unwrap();
        assert_eq!(accounts.len(), 1, "no second account minted");
        assert!(accounts[0].revoked.is_none());
        assert_eq!(accounts[0].access_token, "secret-access");
    }

    /// Usage-capable provider: returns a fixed snapshot, or an error when
    /// `snapshot` is `None`, without touching the network.
    struct FakeCredits {
        snapshot: Option<UsageSnapshot>,
    }

    impl ProviderAdapter for FakeCredits {
        fn id(&self) -> &'static str {
            "credits"
        }
        fn display_name(&self) -> &'static str {
            "Credits"
        }
        fn prepare(
            &self,
            _: &Operation,
            _: &ConnectionConfig,
            _: &Credential,
        ) -> Result<PreparedRequest, ProviderError> {
            Err(ProviderError::UnsupportedOperation)
        }
        fn usage(&self) -> Option<&dyn vkdg_provider_sdk::UsageProvider> {
            Some(self)
        }
    }

    impl vkdg_provider_sdk::UsageProvider for FakeCredits {
        fn fetch_usage<'a>(
            &'a self,
            _: &'a Credential,
        ) -> BoxFuture<'a, Result<UsageSnapshot, ProviderError>> {
            let snap = self.snapshot.clone();
            Box::pin(async move { snap.ok_or_else(|| ProviderError::Http("boom".into())) })
        }
    }

    fn state_with(adapter: Arc<dyn ProviderAdapter>) -> (AdminState, Arc<AccountStore>) {
        let (_tx, rx) = watch::channel(Arc::new(ConfigSnapshot::default_empty()));
        let mut registry = ProviderRegistry::empty();
        registry.register(adapter);
        let store = Arc::new(AccountStore::in_memory().unwrap());
        let state = AdminState {
            sessions: SessionStore::new("tok".into()),
            config_rx: rx,
            started_at: Arc::new(Instant::now()),
            key_store: Arc::new(vkdg_governance::VirtualKeyStore::in_memory().unwrap()),
            request_log: RequestLog::new(),
            combos: None,
            reload_plugins: None,
            connection_tester: None,
            catalog: None,
            logins: Some(LoginService::new(
                Arc::new(registry),
                Arc::clone(&store),
                None,
            )),
            gateway_store: None,
            config_tx: None,
        };
        (state, store)
    }

    fn seed_account(store: &AccountStore, provider: &str) -> Account {
        let account = Account::from_token_pair(
            provider,
            "acct@example.com",
            TokenPair {
                access_token: "ksk_secret".into(),
                refresh_token: None,
                expires_in_secs: None,
                extra: HashMap::from([("auth_method".into(), "api_key".into())]),
            },
        );
        store.upsert(&account).unwrap();
        account
    }

    // Refutes: usage hook ignored; plan limit fabricated (e.g. 0 or overage cap);
    // source not marked "reported"; checked-at timestamp missing.
    #[tokio::test]
    async fn list_accounts_reports_credits_from_usage_provider() {
        let (state, store) = state_with(Arc::new(FakeCredits {
            snapshot: Some(UsageSnapshot {
                credits_used: 137.95,
                credits_limit: Some(10000.0),
                credits_period_end: Some(1_790_812_800),
                plan: Some("KIRO POWER".into()),
                upstream_user_ref: Some("d-9067c9".into()),
            }),
        }));
        seed_account(&store, "credits");
        let h = authed(&state);

        let (status, v, raw) = body_json(list_accounts(State(state), h).await).await;
        assert_eq!(status, StatusCode::OK, "{raw}");
        let item = &v["items"][0];
        assert_eq!(item["credits_source"], "reported", "{raw}");
        assert_eq!(item["credits_used"], 137.95, "{raw}");
        assert_eq!(item["credits_limit"], 10000.0, "{raw}");
        assert_eq!(item["credits_period_end"], 1_790_812_800_i64, "{raw}");
        assert_eq!(item["credits_plan"], "KIRO POWER", "{raw}");
        assert!(
            item["credits_checked_at"].is_string(),
            "checked-at timestamp must be recorded: {raw}"
        );
    }

    // Refutes: an upstream error swallowed into a fabricated 0; source not marked
    // "unavailable"; limit invented when none was reported.
    #[tokio::test]
    async fn list_accounts_marks_usage_unavailable_never_zero() {
        let (state, store) = state_with(Arc::new(FakeCredits { snapshot: None }));
        seed_account(&store, "credits");
        let h = authed(&state);

        let (status, v, raw) = body_json(list_accounts(State(state), h).await).await;
        assert_eq!(status, StatusCode::OK, "{raw}");
        let item = &v["items"][0];
        assert_eq!(item["credits_source"], "unavailable", "{raw}");
        assert!(
            item["credits_used"].is_null(),
            "no usage figure when upstream fails, never 0: {raw}"
        );
        assert!(item["credits_limit"].is_null(), "{raw}");
    }

    // Refutes: forcing credit fields (or a source) onto providers that sell no
    // credits at all.
    #[tokio::test]
    async fn list_accounts_omits_credits_for_provider_without_usage() {
        let (state, store) = make_state();
        seed_account(&store, "fake");
        let h = authed(&state);

        let (status, v, raw) = body_json(list_accounts(State(state), h).await).await;
        assert_eq!(status, StatusCode::OK, "{raw}");
        let item = &v["items"][0];
        assert!(item["credits_source"].is_null(), "{raw}");
        assert!(item["credits_used"].is_null(), "{raw}");
    }
}
