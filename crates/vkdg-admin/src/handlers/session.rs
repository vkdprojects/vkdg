use crate::{
    error::{AdminError, AdminErrorResponse},
    router::AdminState,
    session::Role,
};
use axum::{
    extract::State,
    response::{IntoResponse, Response},
    Json,
};
use http::{HeaderMap, HeaderValue, StatusCode};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct LoginRequest {
    pub token: String,
}

#[derive(Serialize)]
pub struct SessionResponse {
    pub user_id: String,
    pub role: String,
}

fn role_str(role: &Role) -> &'static str {
    match role {
        Role::Admin => "admin",
        Role::Operator => "operator",
        Role::Viewer => "viewer",
    }
}

pub async fn login(State(state): State<AdminState>, Json(body): Json<LoginRequest>) -> Response {
    if body.token != state.sessions.bootstrap_token() {
        return AdminErrorResponse(
            StatusCode::UNAUTHORIZED,
            AdminError::new("invalid_token", "invalid or expired token"),
        )
        .into_response();
    }
    match state.sessions.bootstrap_login() {
        Err(e) => AdminErrorResponse(StatusCode::UNAUTHORIZED, AdminError::new("token_used", e))
            .into_response(),
        Ok(session) => {
            let cookie = format!(
                "vkdg_session={}; HttpOnly; SameSite=Lax; Path=/",
                session.session_id
            );
            let mut headers = HeaderMap::new();
            headers.insert(
                http::header::SET_COOKIE,
                HeaderValue::from_str(&cookie).unwrap_or_else(|_| HeaderValue::from_static("")),
            );
            let body = Json(SessionResponse {
                user_id: session.user_id,
                role: role_str(&session.role).to_string(),
            });
            (StatusCode::OK, headers, body).into_response()
        }
    }
}

pub async fn logout(State(state): State<AdminState>, headers: HeaderMap) -> Response {
    if let Some(session_id) = extract_session_id(&headers) {
        state.sessions.revoke(&session_id);
    }
    let clear = "vkdg_session=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0";
    let mut response_headers = HeaderMap::new();
    response_headers.insert(http::header::SET_COOKIE, HeaderValue::from_static(clear));
    (StatusCode::NO_CONTENT, response_headers).into_response()
}

pub async fn me(State(state): State<AdminState>, headers: HeaderMap) -> Response {
    match get_session(&state, &headers) {
        None => {
            AdminErrorResponse(StatusCode::UNAUTHORIZED, AdminError::unauthorized()).into_response()
        }
        Some(session) => Json(SessionResponse {
            user_id: session.user_id,
            role: role_str(&session.role).to_string(),
        })
        .into_response(),
    }
}

pub fn extract_session_id(headers: &HeaderMap) -> Option<String> {
    headers
        .get(http::header::COOKIE)?
        .to_str()
        .ok()
        .and_then(|s| {
            s.split(';').find_map(|pair| {
                let pair = pair.trim();
                pair.strip_prefix("vkdg_session=").map(|v| v.to_string())
            })
        })
}

pub fn get_session(state: &AdminState, headers: &HeaderMap) -> Option<crate::session::Session> {
    let id = extract_session_id(headers)?;
    state.sessions.get(&id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::State;
    use std::sync::Arc;
    use std::time::Instant;
    use tokio::sync::watch;
    use vkdg_config::ConfigSnapshot;

    fn make_state_with_token(token: &str) -> AdminState {
        let snap = ConfigSnapshot::default_empty();
        let (_tx, rx) = watch::channel(Arc::new(snap));
        AdminState {
            sessions: crate::session::SessionStore::new(token.into()),
            config_rx: rx,
            started_at: Arc::new(Instant::now()),
            key_store: crate::session::KeyStore::new(),
            request_log: crate::handlers::requests::RequestLog::new(),
            combo_resolver: None,
            catalog: None,
        }
    }

    #[tokio::test]
    async fn login_with_valid_token_sets_cookie() {
        let state = make_state_with_token("secret");
        let resp = login(
            State(state),
            Json(LoginRequest {
                token: "secret".into(),
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(resp.headers().contains_key(http::header::SET_COOKIE));
        let cookie = resp
            .headers()
            .get(http::header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(cookie.contains("vkdg_session="));
        assert!(cookie.contains("HttpOnly"));
    }

    #[tokio::test]
    async fn login_with_invalid_token_returns_401() {
        let state = make_state_with_token("secret");
        let resp = login(
            State(state),
            Json(LoginRequest {
                token: "wrong".into(),
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn login_twice_with_same_bootstrap_fails() {
        let state = make_state_with_token("secret");
        let resp1 = login(
            State(state.clone()),
            Json(LoginRequest {
                token: "secret".into(),
            }),
        )
        .await;
        assert_eq!(resp1.status(), StatusCode::OK);

        let resp2 = login(
            State(state),
            Json(LoginRequest {
                token: "secret".into(),
            }),
        )
        .await;
        assert_eq!(resp2.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn me_without_session_returns_401() {
        let state = make_state_with_token("secret");
        let resp = me(State(state), HeaderMap::new()).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn logout_clears_cookie() {
        let state = make_state_with_token("secret");
        let resp = logout(State(state), HeaderMap::new()).await;
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
        let cookie = resp
            .headers()
            .get(http::header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(cookie.contains("Max-Age=0"));
    }
}
