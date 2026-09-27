use crate::{
    error::{AdminError, AdminErrorResponse},
    router::AdminState,
    session::Role,
};
use axum::{
    extract::{ConnectInfo, State},
    response::{IntoResponse, Response},
    Extension, Json,
};
use http::{HeaderMap, HeaderValue, StatusCode};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;

/// `POST /admin/v1/session`: the password once one is set, else the
/// bootstrap token (first run only).
#[derive(Deserialize, Default)]
pub struct LoginRequest {
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
}

#[derive(Deserialize)]
pub struct SetupRequest {
    pub password: String,
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

fn denied(code: &str, message: &str) -> Response {
    AdminErrorResponse(StatusCode::UNAUTHORIZED, AdminError::new(code, message)).into_response()
}

fn session_response(session: crate::session::Session) -> Response {
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

pub async fn login(
    State(state): State<AdminState>,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
    headers: HeaderMap,
    Json(body): Json<LoginRequest>,
) -> Response {
    let ip = state.sessions.client_ip(
        peer.map(|Extension(ConnectInfo(addr))| addr.ip()),
        headers
            .get_all("x-forwarded-for")
            .iter()
            .filter_map(|v| v.to_str().ok()),
    );
    if let Some(wait) = state.sessions.throttled(ip) {
        let mut resp = AdminErrorResponse(
            StatusCode::TOO_MANY_REQUESTS,
            AdminError::new(
                "too_many_attempts",
                "too many failed sign-ins; try again later",
            ),
        )
        .into_response();
        let secs = wait.as_secs().max(1).to_string();
        if let Ok(v) = HeaderValue::from_str(&secs) {
            resp.headers_mut().insert(http::header::RETRY_AFTER, v);
        }
        return resp;
    }

    if let Some(password) = body.password.filter(|p| !p.is_empty()) {
        if !state.sessions.password_set() {
            return AdminErrorResponse(
                StatusCode::BAD_REQUEST,
                AdminError::new(
                    "no_password",
                    "no admin password is set yet; sign in with the bootstrap token",
                ),
            )
            .into_response();
        }
        // argon2 is slow on purpose; keep it off the async workers.
        let sessions = std::sync::Arc::clone(&state.sessions);
        let ok = tokio::task::spawn_blocking(move || sessions.verify_password(&password))
            .await
            .unwrap_or(false);
        if !ok {
            state.sessions.record_failure(ip);
            return denied("invalid_credentials", "wrong password");
        }
        state.sessions.clear_failures(ip);
        return session_response(state.sessions.issue());
    }

    let token = body.token.unwrap_or_default();
    if !state.sessions.token_matches(&token) {
        state.sessions.record_failure(ip);
        return denied("invalid_token", "invalid or expired token");
    }
    if state.sessions.password_set() {
        return denied(
            "password_required",
            "an admin password is set; sign in with the password",
        );
    }
    match state.sessions.bootstrap_login() {
        Err(e) => denied("token_used", e),
        Ok(session) => {
            state.sessions.clear_failures(ip);
            session_response(session)
        }
    }
}

/// `GET /admin/v1/setup`: public. Tells the sign-in page which field to show.
pub async fn setup_status(State(state): State<AdminState>) -> Response {
    Json(serde_json::json!({ "password_set": state.sessions.password_set() })).into_response()
}

/// `POST /admin/v1/setup`: set the admin password, once, from a session opened
/// with the bootstrap token. After this the token no longer signs in. To change
/// the password later, run `vkdg admin set-password` on the host.
pub async fn setup_password(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Json(body): Json<SetupRequest>,
) -> Response {
    if get_session(&state, &headers).is_none() {
        return AdminErrorResponse(StatusCode::UNAUTHORIZED, AdminError::unauthorized())
            .into_response();
    }
    if state.sessions.password_set() {
        return AdminErrorResponse(
            StatusCode::CONFLICT,
            AdminError::new("password_set", "an admin password is already set; use `vkdg admin set-password` on the host to change it"),
        )
        .into_response();
    }
    let sessions = std::sync::Arc::clone(&state.sessions);
    let result = tokio::task::spawn_blocking(move || sessions.set_password(&body.password))
        .await
        .unwrap_or_else(|e| Err(e.to_string()));
    match result {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(msg) => AdminErrorResponse(
            StatusCode::BAD_REQUEST,
            AdminError::new("invalid_input", msg),
        )
        .into_response(),
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
            key_store: Arc::new(vkdg_governance::VirtualKeyStore::in_memory().unwrap()),
            request_log: crate::handlers::requests::RequestLog::new(),
            combos: None,
            reload_plugins: None,
            catalog: None,
            logins: None,
        }
    }

    #[tokio::test]
    async fn login_with_valid_token_sets_cookie() {
        let state = make_state_with_token("secret");
        let resp = login(
            State(state),
            None,
            HeaderMap::new(),
            Json(LoginRequest {
                token: Some("secret".into()),
                password: None,
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
            None,
            HeaderMap::new(),
            Json(LoginRequest {
                token: Some("wrong".into()),
                password: None,
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
            None,
            HeaderMap::new(),
            Json(LoginRequest {
                token: Some("secret".into()),
                password: None,
            }),
        )
        .await;
        assert_eq!(resp1.status(), StatusCode::OK);

        let resp2 = login(
            State(state),
            None,
            HeaderMap::new(),
            Json(LoginRequest {
                token: Some("secret".into()),
                password: None,
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

    fn password_state(dir: &std::path::Path) -> AdminState {
        let mut state = make_state_with_token("boot");
        state.sessions = crate::session::SessionStore::with_password_file(
            "boot".into(),
            dir.join("admin.password"),
            vec![],
        );
        state
    }

    fn token_body(t: &str) -> Json<LoginRequest> {
        Json(LoginRequest {
            token: Some(t.into()),
            password: None,
        })
    }

    fn password_body(p: &str) -> Json<LoginRequest> {
        Json(LoginRequest {
            token: None,
            password: Some(p.into()),
        })
    }

    fn cookie_of(resp: &Response) -> HeaderMap {
        let set = resp.headers()[http::header::SET_COOKIE].to_str().unwrap();
        let mut h = HeaderMap::new();
        h.insert(
            http::header::COOKIE,
            HeaderValue::from_str(set.split(';').next().unwrap()).unwrap(),
        );
        h
    }

    // After one sign-in the token was spent: signing out locked the admin out
    // until a restart. With a password, sign-in works any number of times.
    #[tokio::test]
    async fn setup_then_password_signs_in_repeatedly_and_retires_the_token() {
        let dir = tempfile::tempdir().unwrap();
        let state = password_state(dir.path());
        let first = login(
            State(state.clone()),
            None,
            HeaderMap::new(),
            token_body("boot"),
        )
        .await;
        assert_eq!(first.status(), StatusCode::OK);
        let h = cookie_of(&first);

        let short = setup_password(
            State(state.clone()),
            h.clone(),
            Json(SetupRequest {
                password: "short".into(),
            }),
        )
        .await;
        assert_eq!(short.status(), StatusCode::BAD_REQUEST);
        let ok = setup_password(
            State(state.clone()),
            h.clone(),
            Json(SetupRequest {
                password: "a long admin password".into(),
            }),
        )
        .await;
        assert_eq!(ok.status(), StatusCode::NO_CONTENT);
        let again = setup_password(
            State(state.clone()),
            h,
            Json(SetupRequest {
                password: "another long password".into(),
            }),
        )
        .await;
        assert_eq!(again.status(), StatusCode::CONFLICT);

        for _ in 0..2 {
            let resp = login(
                State(state.clone()),
                None,
                HeaderMap::new(),
                password_body("a long admin password"),
            )
            .await;
            assert_eq!(resp.status(), StatusCode::OK);
        }
        let token = login(
            State(state.clone()),
            None,
            HeaderMap::new(),
            token_body("boot"),
        )
        .await;
        assert_eq!(token.status(), StatusCode::UNAUTHORIZED);
        let wrong = login(
            State(state),
            None,
            HeaderMap::new(),
            password_body("not the password"),
        )
        .await;
        assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn repeated_failures_from_one_address_get_429_with_retry_after() {
        let dir = tempfile::tempdir().unwrap();
        let state = password_state(dir.path());
        state
            .sessions
            .set_password("a long admin password")
            .unwrap();
        let peer = || {
            Some(Extension(ConnectInfo(SocketAddr::from((
                [203, 0, 113, 9],
                5000,
            )))))
        };
        for _ in 0..crate::session::MAX_FAILURES {
            let r = login(
                State(state.clone()),
                peer(),
                HeaderMap::new(),
                password_body("wrong wrong wrong"),
            )
            .await;
            assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
        }
        // Even the right password is refused while throttled.
        let r = login(
            State(state.clone()),
            peer(),
            HeaderMap::new(),
            password_body("a long admin password"),
        )
        .await;
        assert_eq!(r.status(), StatusCode::TOO_MANY_REQUESTS);
        assert!(r.headers().contains_key(http::header::RETRY_AFTER));
        // Another address is unaffected.
        let other = Some(Extension(ConnectInfo(SocketAddr::from((
            [198, 51, 100, 1],
            5000,
        )))));
        let r = login(
            State(state),
            other,
            HeaderMap::new(),
            password_body("a long admin password"),
        )
        .await;
        assert_eq!(r.status(), StatusCode::OK);
    }
}
