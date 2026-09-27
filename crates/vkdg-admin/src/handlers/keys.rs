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
use vkdg_governance::{KeyScope, VirtualKey, VirtualKeyId};

#[derive(Deserialize)]
pub struct CreateKeyBody {
    pub name: String,
    /// Defaults to every data-plane scope.
    pub scopes: Option<Vec<KeyScope>>,
    pub tenant_id: Option<String>,
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
    /// `active` or `revoked`.
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
            status: if k.is_revoked() { "revoked" } else { "active" },
        }
    }
}

#[derive(Serialize)]
struct KeyListResponse {
    items: Vec<KeySummary>,
    total: usize,
}

fn unauthorized() -> Response {
    AdminErrorResponse(StatusCode::UNAUTHORIZED, AdminError::unauthorized()).into_response()
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
            let items: Vec<KeySummary> = keys.iter().map(KeySummary::from).collect();
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
    let tenant = body.tenant_id.as_deref().unwrap_or("default");
    match state.key_store.create(name, tenant, scopes) {
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
}
