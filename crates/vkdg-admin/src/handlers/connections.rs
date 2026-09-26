use axum::{
    extract::{Path, State},
    response::{IntoResponse, Response},
    Json,
};
use http::{HeaderMap, StatusCode};
use serde::Serialize;
use crate::{
    error::{AdminError, AdminErrorResponse},
    handlers::session::get_session,
    router::AdminState,
};

#[derive(Serialize)]
pub struct ConnectionSummary {
    id: String,
    provider: String,
    status: &'static str,
    model_count: usize,
    active_requests: u32,
}

#[derive(Serialize)]
pub struct ConnectionList {
    items: Vec<ConnectionSummary>,
    total: usize,
}

fn provider_str(kind: &vkdg_connections::ProviderKind) -> String {
    match kind {
        vkdg_connections::ProviderKind::Anthropic => "anthropic".into(),
        vkdg_connections::ProviderKind::OpenAI => "openai".into(),
        vkdg_connections::ProviderKind::Google => "google".into(),
        vkdg_connections::ProviderKind::Custom { .. } => "custom".into(),
    }
}

fn conn_to_summary(conn: &vkdg_connections::ConnectionConfig) -> ConnectionSummary {
    ConnectionSummary {
        id: conn.id.0.clone(),
        provider: provider_str(&conn.provider),
        status: "healthy",
        model_count: conn.models.len(),
        active_requests: 0,
    }
}

pub async fn list_connections(
    State(state): State<AdminState>,
    headers: HeaderMap,
) -> Response {
    if get_session(&state, &headers).is_none() {
        return AdminErrorResponse(StatusCode::UNAUTHORIZED, AdminError::unauthorized())
            .into_response();
    }
    let snapshot = state.config_rx.borrow().clone();
    let items: Vec<ConnectionSummary> = snapshot.connections.iter().map(conn_to_summary).collect();
    let total = items.len();
    Json(ConnectionList { items, total }).into_response()
}

pub async fn get_connection(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if get_session(&state, &headers).is_none() {
        return AdminErrorResponse(StatusCode::UNAUTHORIZED, AdminError::unauthorized())
            .into_response();
    }
    let snapshot = state.config_rx.borrow().clone();
    match snapshot.connections.iter().find(|c| c.id.0 == id) {
        None => {
            AdminErrorResponse(StatusCode::NOT_FOUND, AdminError::not_found(&id)).into_response()
        }
        Some(c) => Json(conn_to_summary(c)).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Instant;
    use axum::extract::State;
    use tokio::sync::watch;
    use vkdg_config::ConfigSnapshot;

    fn make_state() -> AdminState {
        let snap = ConfigSnapshot::default_empty();
        let (_tx, rx) = watch::channel(Arc::new(snap));
        AdminState {
            sessions: crate::session::SessionStore::new("tok".into()),
            config_rx: rx,
            started_at: Arc::new(Instant::now()),
        }
    }

    #[tokio::test]
    async fn list_connections_without_session_returns_401() {
        let state = make_state();
        let resp = list_connections(State(state), HeaderMap::new()).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn list_connections_returns_items() {
        use vkdg_config::{ConfigSnapshot, GatewayConfig, ConnectionDef, schema::AuthDef};

        let gateway_cfg = GatewayConfig {
            listen: "0.0.0.0:8080".into(),
            connections: vec![ConnectionDef {
                id: "test-conn".into(),
                provider: "anthropic".into(),
                auth: AuthDef::ApiKey { env_var: "ANTHROPIC_API_KEY".into() },
                models: vec!["claude-*".into()],
                max_concurrent: None,
                weight: None,
            }],
            routes: vec![],
            limits: None,
            observe: None,
        };
        let snap = ConfigSnapshot::build(1, gateway_cfg).expect("build snapshot");
        let (_tx, rx) = watch::channel(Arc::new(snap));

        let sessions = crate::session::SessionStore::new("tok".into());
        let session = sessions.bootstrap_login().unwrap();
        let state = AdminState {
            sessions,
            config_rx: rx,
            started_at: Arc::new(Instant::now()),
        };

        let mut headers = HeaderMap::new();
        let cookie_val = format!("vkdg_session={}", session.session_id);
        headers.insert(
            http::header::COOKIE,
            http::HeaderValue::from_str(&cookie_val).unwrap(),
        );

        let resp = list_connections(State(state), headers).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let list: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(list["total"], 1);
        assert_eq!(list["items"].as_array().unwrap().len(), 1);
    }
}
