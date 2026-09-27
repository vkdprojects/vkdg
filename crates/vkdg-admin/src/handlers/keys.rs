//! `/admin/v1/keys`: data-plane API keys, backed by the same
//! [`VirtualKeyStore`](vkdg_governance::VirtualKeyStore) that `/v1/*` checks.
//! A key created here works on the gateway immediately; the raw key is returned
//! once, and neither it nor its hash is ever listed.

use crate::{
    error::{AdminError, AdminErrorResponse},
    handlers::session::get_session,
    router::AdminState,
};
use axum::{
    extract::{Path, State},
    response::{IntoResponse, Response},
    Json,
};
use http::{HeaderMap, StatusCode};
use serde::{Deserialize, Serialize};
use vkdg_governance::{KeyPatch, KeyScope, NewKey, VirtualKey, VirtualKeyId};

#[derive(Deserialize)]
pub struct CreateKeyBody {
    pub name: String,
    /// Defaults to every data-plane scope.
    pub scopes: Option<Vec<KeyScope>>,
    pub tenant_id: Option<String>,
    /// RFC 3339 instant after which the key stops working.
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Model patterns (`claude-*`); empty or absent = every model.
    #[serde(default)]
    pub allowed_models: Vec<String>,
    /// Addresses or CIDR ranges; empty or absent = anywhere.
    #[serde(default)]
    pub allowed_ips: Vec<String>,
    /// Tokens (input + output) per calendar month (UTC); absent = unlimited.
    pub monthly_token_limit: Option<u64>,
    /// Requests per minute; absent = unlimited.
    pub requests_per_minute: Option<u32>,
}

#[derive(Serialize)]
struct CreatedKeyResponse {
    /// The raw key. Shown once; not recoverable afterwards.
    key: String,
    #[serde(flatten)]
    summary: KeySummary,
}

#[derive(Serialize)]
struct KeySummary {
    id: String,
    name: String,
    tenant_id: String,
    /// Safe-to-show start of the key, e.g. `vkdg_1a2b3c4d`.
    prefix: String,
    scopes: Vec<KeyScope>,
    created_at: String,
    last_used_at: Option<String>,
    revoked_at: Option<String>,
    expires_at: Option<String>,
    allowed_models: Vec<String>,
    allowed_ips: Vec<String>,
    monthly_token_limit: Option<u64>,
    requests_per_minute: Option<u32>,
    /// Tokens and requests this calendar month (UTC); absent if unreadable.
    usage_this_month: Option<UsageSummary>,
    /// `active`, `expired`, or `revoked`.
    status: &'static str,
}

impl From<&VirtualKey> for KeySummary {
    fn from(k: &VirtualKey) -> Self {
        Self {
            id: k.id.0.clone(),
            name: k.name.clone(),
            tenant_id: k.tenant_id.clone(),
            prefix: k.prefix.clone(),
            scopes: k.scopes.clone(),
            created_at: k.created_at.to_rfc3339(),
            last_used_at: k.last_used_at.map(|t| t.to_rfc3339()),
            revoked_at: k.revoked_at.map(|t| t.to_rfc3339()),
            expires_at: k.expires_at.map(|t| t.to_rfc3339()),
            allowed_models: k.allowed_models.clone(),
            allowed_ips: k.allowed_ips.iter().map(ToString::to_string).collect(),
            monthly_token_limit: k.monthly_token_limit,
            requests_per_minute: k.requests_per_minute,
            usage_this_month: None,
            status: if k.is_revoked() {
                "revoked"
            } else if k.is_disabled() {
                "disabled"
            } else if k.is_expired_at(chrono::Utc::now()) {
                "expired"
            } else {
                "active"
            },
        }
    }
}

#[derive(Serialize)]
struct UsageSummary {
    input_tokens: u64,
    output_tokens: u64,
    requests: u64,
}

#[derive(Serialize)]
struct KeyListResponse {
    items: Vec<KeySummary>,
    total: usize,
}

fn unauthorized() -> Response {
    AdminErrorResponse(StatusCode::UNAUTHORIZED, AdminError::unauthorized()).into_response()
}

fn invalid(message: impl Into<String>) -> Response {
    AdminErrorResponse(
        StatusCode::BAD_REQUEST,
        AdminError::new("invalid_input", message.into()),
    )
    .into_response()
}

fn store_failure(e: &vkdg_governance::KeyStoreError) -> Response {
    AdminErrorResponse(
        StatusCode::INTERNAL_SERVER_ERROR,
        AdminError::new("key_store", e.to_string()),
    )
    .into_response()
}

pub async fn list_keys(State(state): State<AdminState>, headers: HeaderMap) -> Response {
    if get_session(&state, &headers).is_none() {
        return unauthorized();
    }
    match state.key_store.list() {
        Ok(keys) => {
            let items: Vec<KeySummary> =
                keys.iter()
                    .map(|k| {
                        let mut summary = KeySummary::from(k);
                        summary.usage_this_month = state
                            .key_store
                            .usage_this_month(&k.id)
                            .ok()
                            .map(|u| UsageSummary {
                                input_tokens: u.input_tokens,
                                output_tokens: u.output_tokens,
                                requests: u.requests,
                            });
                        summary
                    })
                    .collect();
            let total = items.len();
            Json(KeyListResponse { items, total }).into_response()
        }
        Err(e) => store_failure(&e),
    }
}

pub async fn create_key(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Json(body): Json<CreateKeyBody>,
) -> Response {
    if get_session(&state, &headers).is_none() {
        return unauthorized();
    }
    let name = body.name.trim();
    if name.is_empty() {
        return AdminErrorResponse(
            StatusCode::BAD_REQUEST,
            AdminError::new("invalid_input", "name must not be empty"),
        )
        .into_response();
    }
    let scopes = body.scopes.unwrap_or_else(|| KeyScope::DEFAULT.to_vec());
    if scopes.is_empty() {
        return AdminErrorResponse(
            StatusCode::BAD_REQUEST,
            AdminError::new("invalid_input", "a key needs at least one scope"),
        )
        .into_response();
    }
    let allowed_ips = match vkdg_core::net::parse_ip_list("allowed_ips", &body.allowed_ips) {
        Ok(ips) => ips,
        Err(msg) => return invalid(msg),
    };
    if body.expires_at.is_some_and(|t| t <= chrono::Utc::now()) {
        return invalid("expires_at must be in the future");
    }
    let spec = NewKey {
        name: name.to_owned(),
        tenant_id: body.tenant_id.unwrap_or_else(|| "default".into()),
        scopes,
        expires_at: body.expires_at,
        allowed_models: body.allowed_models,
        allowed_ips,
        monthly_token_limit: body.monthly_token_limit,
        requests_per_minute: body.requests_per_minute,
    };
    match state.key_store.create(spec) {
        Ok((key, raw)) => (
            StatusCode::CREATED,
            Json(CreatedKeyResponse {
                key: raw,
                summary: KeySummary::from(&key),
            }),
        )
            .into_response(),
        Err(e) => store_failure(&e),
    }
}

/// Body of `PATCH /admin/v1/keys/{id}`. Absent fields are unchanged; `null`
/// clears an optional limit.
#[derive(Deserialize, Default)]
pub struct UpdateKeyBody {
    pub name: Option<String>,
    pub scopes: Option<Vec<KeyScope>>,
    #[serde(default, deserialize_with = "present")]
    pub expires_at: Option<Option<chrono::DateTime<chrono::Utc>>>,
    pub allowed_models: Option<Vec<String>>,
    pub allowed_ips: Option<Vec<String>>,
    #[serde(default, deserialize_with = "present")]
    pub monthly_token_limit: Option<Option<u64>>,
    #[serde(default, deserialize_with = "present")]
    pub requests_per_minute: Option<Option<u32>>,
}

/// Distinguishes an absent field (`None`) from an explicit `null` (`Some(None)`).
fn present<'de, D, T>(d: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(d).map(Some)
}

fn not_found(id: &str) -> Response {
    AdminErrorResponse(StatusCode::NOT_FOUND, AdminError::not_found(id)).into_response()
}

pub async fn update_key(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<UpdateKeyBody>,
) -> Response {
    if get_session(&state, &headers).is_none() {
        return unauthorized();
    }
    if body.name.as_deref().is_some_and(|n| n.trim().is_empty()) {
        return invalid("name must not be empty");
    }
    if body.scopes.as_ref().is_some_and(Vec::is_empty) {
        return invalid("a key needs at least one scope");
    }
    if body
        .expires_at
        .flatten()
        .is_some_and(|t| t <= chrono::Utc::now())
    {
        return invalid("expires_at must be in the future");
    }
    let allowed_ips = match body.allowed_ips.as_deref() {
        Some(list) => match vkdg_core::net::parse_ip_list("allowed_ips", list) {
            Ok(ips) => Some(ips),
            Err(msg) => return invalid(msg),
        },
        None => None,
    };
    let patch = KeyPatch {
        name: body.name.map(|n| n.trim().to_owned()),
        scopes: body.scopes,
        expires_at: body.expires_at,
        allowed_models: body.allowed_models,
        allowed_ips,
        monthly_token_limit: body.monthly_token_limit,
        requests_per_minute: body.requests_per_minute,
    };
    match state.key_store.update(&VirtualKeyId(id.clone()), patch) {
        Ok(Some(key)) => Json(KeySummary::from(&key)).into_response(),
        Ok(None) => not_found(&id),
        Err(e) => store_failure(&e),
    }
}

/// `POST /admin/v1/keys/{id}/regenerate`: new secret, same id, policy and
/// usage. The old secret stops working at once. The new one is shown once.
pub async fn regenerate_key(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if get_session(&state, &headers).is_none() {
        return unauthorized();
    }
    match state.key_store.regenerate(&VirtualKeyId(id.clone())) {
        Ok(Some((key, raw))) => Json(CreatedKeyResponse {
            key: raw,
            summary: KeySummary::from(&key),
        })
        .into_response(),
        Ok(None) => not_found(&id),
        Err(e) => store_failure(&e),
    }
}

async fn set_disabled(
    state: AdminState,
    headers: HeaderMap,
    id: String,
    disabled: bool,
) -> Response {
    if get_session(&state, &headers).is_none() {
        return unauthorized();
    }
    match state
        .key_store
        .set_disabled(&VirtualKeyId(id.clone()), disabled)
    {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => not_found(&id),
        Err(e) => store_failure(&e),
    }
}

/// `POST /admin/v1/keys/{id}/disable`: reversible; the key stops authenticating.
pub async fn disable_key(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    set_disabled(state, headers, id, true).await
}

/// `POST /admin/v1/keys/{id}/enable`: undoes a disable. Never un-revokes.
pub async fn enable_key(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    set_disabled(state, headers, id, false).await
}

pub async fn revoke_key(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if get_session(&state, &headers).is_none() {
        return unauthorized();
    }
    match state.key_store.revoke(&VirtualKeyId(id.clone())) {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => {
            AdminErrorResponse(StatusCode::NOT_FOUND, AdminError::not_found(&id)).into_response()
        }
        Err(e) => store_failure(&e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handlers::requests::RequestLog;
    use crate::session::SessionStore;
    use std::sync::Arc;
    use std::time::Instant;
    use tokio::sync::watch;
    use vkdg_config::ConfigSnapshot;

    fn make_state() -> AdminState {
        let snap = ConfigSnapshot::default_empty();
        let (_tx, rx) = watch::channel(Arc::new(snap));
        AdminState {
            sessions: SessionStore::new("tok".into()),
            config_rx: rx,
            started_at: Arc::new(Instant::now()),
            key_store: Arc::new(vkdg_governance::VirtualKeyStore::in_memory().unwrap()),
            request_log: RequestLog::new(),
            combo_resolver: None,
            catalog: None,
            logins: None,
        }
    }

    fn authed_headers(state: &AdminState) -> HeaderMap {
        let session = state.sessions.bootstrap_login().unwrap();
        let mut h = HeaderMap::new();
        let val = format!("vkdg_session={}", session.session_id);
        h.insert(
            http::header::COOKIE,
            http::HeaderValue::from_str(&val).unwrap(),
        );
        h
    }

    async fn json(resp: Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn body(name: &str, scopes: Option<Vec<KeyScope>>) -> Json<CreateKeyBody> {
        Json(CreateKeyBody {
            name: name.into(),
            scopes,
            tenant_id: None,
            expires_at: None,
            allowed_models: vec![],
            allowed_ips: vec![],
            monthly_token_limit: None,
            requests_per_minute: None,
        })
    }

    #[tokio::test]
    async fn every_endpoint_requires_a_session() {
        let state = make_state();
        let no = HeaderMap::new();
        assert_eq!(
            list_keys(State(state.clone()), no.clone()).await.status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            create_key(State(state.clone()), no.clone(), body("k", None))
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            revoke_key(State(state), no, Path("x".into()))
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }

    // Plausible wrong impl: the admin writes to a store the data plane never
    // reads, so console-created keys get 401 on /v1/* (the old plaintext store).
    #[tokio::test]
    async fn key_created_in_admin_authenticates_on_the_data_plane_store() {
        let state = make_state();
        let h = authed_headers(&state);
        let resp = create_key(State(state.clone()), h.clone(), body("ci", None)).await;
        assert_eq!(resp.status(), StatusCode::CREATED);
        let created = json(resp).await;
        let raw = created["key"].as_str().unwrap();
        assert!(raw.starts_with(vkdg_governance::TOKEN_PREFIX));
        assert_eq!(created["status"], "active");

        let key = state.key_store.authenticate(raw).unwrap().expect("usable");
        assert_eq!(key.id.0, created["id"].as_str().unwrap());

        let id = created["id"].as_str().unwrap().to_owned();
        let resp = revoke_key(State(state.clone()), h, Path(id)).await;
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
        assert!(state.key_store.authenticate(raw).unwrap().is_none());
    }

    // Plausible wrong impl: the list leaks the raw key or its hash.
    #[tokio::test]
    async fn list_never_exposes_the_secret_and_shows_revoked_keys() {
        let state = make_state();
        let h = authed_headers(&state);
        let created =
            json(create_key(State(state.clone()), h.clone(), body("ci", None)).await).await;
        let raw = created["key"].as_str().unwrap().to_owned();
        let id = created["id"].as_str().unwrap().to_owned();
        let hash = vkdg_governance::hash_token(&raw);
        revoke_key(State(state.clone()), h.clone(), Path(id)).await;

        let listed = list_keys(State(state), h).await;
        let text = String::from_utf8(
            axum::body::to_bytes(listed.into_body(), usize::MAX)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap();
        assert!(!text.contains(&raw), "raw key leaked: {text}");
        assert!(!text.contains(&hash), "hash leaked: {text}");
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["total"], 1);
        assert_eq!(v["items"][0]["status"], "revoked");
        assert!(v["items"][0]["key"].is_null());
    }

    #[tokio::test]
    async fn invalid_input_is_rejected() {
        let state = make_state();
        let h = authed_headers(&state);
        let blank = create_key(State(state.clone()), h.clone(), body("  ", None)).await;
        assert_eq!(blank.status(), StatusCode::BAD_REQUEST);
        // A key with no scopes could never call anything: reject, don't mint.
        let none = create_key(State(state.clone()), h.clone(), body("k", Some(vec![]))).await;
        assert_eq!(none.status(), StatusCode::BAD_REQUEST);
        let missing = revoke_key(State(state), h, Path("no-such-id".into())).await;
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn limits_are_validated_stored_and_listed() {
        let state = make_state();
        let h = authed_headers(&state);
        let mk = |ips: Vec<&str>, exp: Option<chrono::DateTime<chrono::Utc>>| {
            Json(CreateKeyBody {
                name: "scoped".into(),
                scopes: None,
                tenant_id: None,
                expires_at: exp,
                allowed_models: vec!["claude-*".into()],
                allowed_ips: ips.into_iter().map(Into::into).collect(),
                monthly_token_limit: None,
                requests_per_minute: None,
            })
        };
        let bad_ip = create_key(State(state.clone()), h.clone(), mk(vec!["192.168."], None)).await;
        assert_eq!(bad_ip.status(), StatusCode::BAD_REQUEST);
        let past = chrono::Utc::now() - chrono::Duration::hours(1);
        let bad_exp = create_key(State(state.clone()), h.clone(), mk(vec![], Some(past))).await;
        assert_eq!(bad_exp.status(), StatusCode::BAD_REQUEST);

        let future = chrono::Utc::now() + chrono::Duration::days(7);
        let ok = create_key(
            State(state.clone()),
            h.clone(),
            mk(vec!["10.0.0.0/8"], Some(future)),
        )
        .await;
        assert_eq!(ok.status(), StatusCode::CREATED);
        let v = json(list_keys(State(state), h).await).await;
        let item = &v["items"][0];
        assert_eq!(item["allowed_models"][0], "claude-*");
        assert_eq!(item["allowed_ips"][0], "10.0.0.0/8");
        assert!(item["expires_at"].is_string());
        assert_eq!(item["status"], "active");
        assert_eq!(item["usage_this_month"]["requests"], 0);
    }

    #[tokio::test]
    async fn update_regenerate_and_disable_round_trip() {
        let state = make_state();
        let h = authed_headers(&state);
        let created =
            json(create_key(State(state.clone()), h.clone(), body("k", None)).await).await;
        let id = created["id"].as_str().unwrap().to_owned();
        let old = created["key"].as_str().unwrap().to_owned();

        let patch: UpdateKeyBody = serde_json::from_value(serde_json::json!({
            "allowed_models": ["claude-*"], "monthly_token_limit": 500
        }))
        .unwrap();
        let resp = update_key(
            State(state.clone()),
            h.clone(),
            Path(id.clone()),
            Json(patch),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let v = json(resp).await;
        assert_eq!(v["allowed_models"][0], "claude-*");
        assert_eq!(v["monthly_token_limit"], 500);

        // null clears a limit; absent leaves it.
        let clear: UpdateKeyBody =
            serde_json::from_value(serde_json::json!({ "monthly_token_limit": null })).unwrap();
        let v = json(
            update_key(
                State(state.clone()),
                h.clone(),
                Path(id.clone()),
                Json(clear),
            )
            .await,
        )
        .await;
        assert!(v["monthly_token_limit"].is_null());
        assert_eq!(v["allowed_models"][0], "claude-*");

        let bad: UpdateKeyBody =
            serde_json::from_value(serde_json::json!({ "allowed_ips": ["192.168."] })).unwrap();
        let resp = update_key(State(state.clone()), h.clone(), Path(id.clone()), Json(bad)).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let regen =
            json(regenerate_key(State(state.clone()), h.clone(), Path(id.clone())).await).await;
        let new = regen["key"].as_str().unwrap();
        assert_ne!(new, old);
        assert!(state.key_store.authenticate(&old).unwrap().is_none());
        assert!(state.key_store.authenticate(new).unwrap().is_some());

        let resp = disable_key(State(state.clone()), h.clone(), Path(id.clone())).await;
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
        let listed = json(list_keys(State(state.clone()), h.clone()).await).await;
        assert_eq!(listed["items"][0]["status"], "disabled");
        assert!(state.key_store.authenticate(new).unwrap().is_none());
        let resp = enable_key(State(state.clone()), h.clone(), Path(id.clone())).await;
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
        assert!(state.key_store.authenticate(new).unwrap().is_some());

        let missing = regenerate_key(State(state), h, Path("nope".into())).await;
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    }
}
