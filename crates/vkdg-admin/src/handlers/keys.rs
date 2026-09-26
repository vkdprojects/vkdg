use crate::{
    error::{AdminError, AdminErrorResponse},
    handlers::session::get_session,
    router::AdminState,
    session::Role,
};
use axum::{
    extract::{Path, State},
    response::{IntoResponse, Response},
    Json,
};
use http::{HeaderMap, StatusCode};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct CreateKeyBody {
    pub name: String,
    pub role: Option<String>,
    pub scopes: Option<Vec<String>>,
}

#[derive(Serialize)]
struct CreatedKeyResponse {
    key: String,
    id: String,
    name: String,
}

#[derive(Serialize)]
struct KeyResponse {
    id: String,
    name: String,
    role: String,
    created_at: String,
    last_used_at: Option<String>,
    scopes: Vec<String>,
}

#[derive(Serialize)]
struct KeyListResponse {
    items: Vec<KeyResponse>,
    total: usize,
}

fn role_from_str(s: &str) -> Role {
    match s {
        "admin" => Role::Admin,
        "operator" => Role::Operator,
        _ => Role::Viewer,
    }
}

fn role_to_str(r: &Role) -> &'static str {
    match r {
        Role::Admin => "admin",
        Role::Operator => "operator",
        Role::Viewer => "viewer",
    }
}

pub async fn list_keys(State(state): State<AdminState>, headers: HeaderMap) -> Response {
    if get_session(&state, &headers).is_none() {
        return AdminErrorResponse(StatusCode::UNAUTHORIZED, AdminError::unauthorized())
            .into_response();
    }
    let keys: Vec<KeyResponse> = state
        .key_store
        .list()
        .into_iter()
        .map(|k| KeyResponse {
            id: k.id,
            name: k.name,
            role: role_to_str(&k.role).to_string(),
            created_at: k.created_at.to_rfc3339(),
            last_used_at: k.last_used_at.map(|t| t.to_rfc3339()),
            scopes: k.scopes,
        })
        .collect();
    let total = keys.len();
    Json(KeyListResponse { items: keys, total }).into_response()
}

pub async fn create_key(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Json(body): Json<CreateKeyBody>,
) -> Response {
    if get_session(&state, &headers).is_none() {
        return AdminErrorResponse(StatusCode::UNAUTHORIZED, AdminError::unauthorized())
            .into_response();
    }
    if body.name.trim().is_empty() {
        return AdminErrorResponse(
            StatusCode::BAD_REQUEST,
            AdminError::new("invalid_input", "name must not be empty"),
        )
        .into_response();
    }
    let role = role_from_str(body.role.as_deref().unwrap_or("viewer"));
    let scopes = body.scopes.unwrap_or_default();
    let (key_meta, raw) = state.key_store.create(body.name, role, scopes);
    (
        StatusCode::CREATED,
        Json(CreatedKeyResponse {
            key: raw,
            id: key_meta.id,
            name: key_meta.name,
        }),
    )
        .into_response()
}

pub async fn revoke_key(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if get_session(&state, &headers).is_none() {
        return AdminErrorResponse(StatusCode::UNAUTHORIZED, AdminError::unauthorized())
            .into_response();
    }
    if state.key_store.revoke(&id) {
        StatusCode::NO_CONTENT.into_response()
    } else {
        AdminErrorResponse(StatusCode::NOT_FOUND, AdminError::not_found(&id)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handlers::requests::RequestLog;
    use crate::session::{KeyStore, SessionStore};
    use axum::extract::State;
    use http::HeaderMap;
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
            key_store: KeyStore::new(),
            request_log: RequestLog::new(),
            combo_resolver: None,
            catalog: None,
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

    #[tokio::test]
    async fn list_keys_requires_session() {
        let state = make_state();
        let resp = list_keys(State(state), HeaderMap::new()).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn create_key_returns_201_with_raw_token() {
        let state = make_state();
        let headers = authed_headers(&state);
        let body = CreateKeyBody {
            name: "my-key".into(),
            role: Some("operator".into()),
            scopes: None,
        };
        let resp = create_key(State(state), headers, Json(body)).await;
        assert_eq!(resp.status(), StatusCode::CREATED);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let val: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(val["key"].as_str().is_some_and(|s| !s.is_empty()));
        assert_eq!(val["name"], "my-key");
    }

    #[tokio::test]
    async fn create_key_empty_name_returns_400() {
        let state = make_state();
        let headers = authed_headers(&state);
        let body = CreateKeyBody {
            name: "  ".into(),
            role: None,
            scopes: None,
        };
        let resp = create_key(State(state), headers, Json(body)).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn revoke_key_not_found_returns_404() {
        let state = make_state();
        let headers = authed_headers(&state);
        let resp = revoke_key(State(state), headers, Path("no-such-id".into())).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}
