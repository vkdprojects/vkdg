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
use serde::Serialize;

#[derive(Serialize)]
pub struct ConnectionSummary {
    id: String,
    provider: String,
    /// `healthy`, `degraded`, `circuit_open`, `cooldown`; `unknown` when the
    /// data plane is not running (no catalog).
    status: &'static str,
    model_count: usize,
    active_requests: u32,
    max_concurrent: u32,
    /// Set while the connection is cooling down or its circuit is open.
    #[serde(skip_serializing_if = "Option::is_none")]
    cooldown_until: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    failure_count: Option<u32>,
}

#[derive(Serialize)]
pub struct ConnectionList {
    items: Vec<ConnectionSummary>,
    total: usize,
}

/// Summary with live state from the data plane's catalog when it has this id.
async fn summarize(
    state: &AdminState,
    conn: &vkdg_connections::ConnectionConfig,
) -> ConnectionSummary {
    use vkdg_connections::ConnectionState as S;
    let mut summary = ConnectionSummary {
        id: conn.id.0.clone(),
        provider: conn.provider.as_str().to_string(),
        status: "unknown",
        model_count: conn.models.len(),
        active_requests: 0,
        max_concurrent: conn.max_concurrent,
        cooldown_until: None,
        failure_count: None,
    };
    let Some(live) = state.catalog.as_ref().and_then(|c| c.get(&conn.id)) else {
        return summary;
    };
    let live = live.read().await;
    summary.active_requests = live.active_requests();
    summary.status = match &live.state {
        S::Healthy => "healthy",
        S::Degraded { .. } => "degraded",
        S::CircuitOpen { until } => {
            summary.cooldown_until = Some(until.to_rfc3339());
            "circuit_open"
        }
        S::Cooldown {
            until,
            failure_count,
        } => {
            summary.cooldown_until = Some(until.to_rfc3339());
            summary.failure_count = Some(*failure_count);
            "cooldown"
        }
    };
    summary
}

pub async fn list_connections(State(state): State<AdminState>, headers: HeaderMap) -> Response {
    if get_session(&state, &headers).is_none() {
        return AdminErrorResponse(StatusCode::UNAUTHORIZED, AdminError::unauthorized())
            .into_response();
    }
    let snapshot = state.config_rx.borrow().clone();
    let mut items = Vec::with_capacity(snapshot.connections.len());
    for conn in snapshot.connections.iter() {
        items.push(summarize(&state, conn).await);
    }
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
        Some(c) => Json(summarize(&state, c).await).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::State;
    use std::sync::Arc;
    use std::time::Instant;
    use tokio::sync::watch;
    use vkdg_config::ConfigSnapshot;

    fn make_state() -> AdminState {
        let snap = ConfigSnapshot::default_empty();
        let (_tx, rx) = watch::channel(Arc::new(snap));
        AdminState {
            sessions: crate::session::SessionStore::new("tok".into()),
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
    async fn list_connections_without_session_returns_401() {
        let state = make_state();
        let resp = list_connections(State(state), HeaderMap::new()).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn list_connections_returns_items() {
        use vkdg_config::{schema::AuthDef, ConfigSnapshot, ConnectionDef, GatewayConfig};

        let gateway_cfg = GatewayConfig {
            listen: "0.0.0.0:8080".into(),
            connections: vec![ConnectionDef {
                id: "test-conn".into(),
                provider: "anthropic".into(),
                auth: AuthDef::ApiKey {
                    env_var: "ANTHROPIC_API_KEY".into(),
                },
                models: vec!["claude-*".into()],
                max_concurrent: None,
                weight: None,
            }],
            routes: vec![],
            limits: None,
            observe: None,
            global_system_prompt: None,
        };
        let snap = ConfigSnapshot::build(1, gateway_cfg).expect("build snapshot");
        let (_tx, rx) = watch::channel(Arc::new(snap));

        let sessions = crate::session::SessionStore::new("tok".into());
        let session = sessions.bootstrap_login().unwrap();
        let state = AdminState {
            sessions,
            config_rx: rx,
            started_at: Arc::new(Instant::now()),
            key_store: Arc::new(vkdg_governance::VirtualKeyStore::in_memory().unwrap()),
            request_log: crate::handlers::requests::RequestLog::new(),
            combos: None,
            reload_plugins: None,
            catalog: None,
            logins: None,
        };

        let mut headers = HeaderMap::new();
        let cookie_val = format!("vkdg_session={}", session.session_id);
        headers.insert(
            http::header::COOKIE,
            http::HeaderValue::from_str(&cookie_val).unwrap(),
        );

        let resp = list_connections(State(state), headers).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let list: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(list["total"], 1);
        assert_eq!(list["items"].as_array().unwrap().len(), 1);
    }

    // The list said "healthy" and 0 in-flight for every connection, whatever
    // the circuit state: the console could not show an account in cooldown.
    #[tokio::test]
    async fn list_reports_live_state_and_in_flight_from_the_catalog() {
        use vkdg_config::{schema::AuthDef, ConnectionDef, GatewayConfig};
        let cfg = GatewayConfig {
            listen: "0.0.0.0:8080".into(),
            connections: vec![ConnectionDef {
                id: "c1".into(),
                provider: "anthropic".into(),
                auth: AuthDef::ApiKey {
                    env_var: "UNUSED".into(),
                },
                models: vec!["*".into()],
                max_concurrent: None,
                weight: None,
            }],
            routes: vec![],
            limits: None,
            observe: None,
            global_system_prompt: None,
        };
        let snap = ConfigSnapshot::build(1, cfg).unwrap();
        let catalog = Arc::new(vkdg_connections::ConnectionCatalog::new(
            snap.connections.as_ref().clone(),
        ));
        let conn = catalog.get(&vkdg_core::ConnectionId("c1".into())).unwrap();
        let guard = conn.read().await.acquire().unwrap();
        conn.write().await.record_upstream_error(429);

        let (_tx, rx) = watch::channel(Arc::new(snap));
        let sessions = crate::session::SessionStore::new("tok".into());
        let session = sessions.bootstrap_login().unwrap();
        let state = AdminState {
            sessions,
            config_rx: rx,
            started_at: Arc::new(Instant::now()),
            key_store: Arc::new(vkdg_governance::VirtualKeyStore::in_memory().unwrap()),
            request_log: crate::handlers::requests::RequestLog::new(),
            combos: None,
            reload_plugins: None,
            catalog: Some(catalog),
            logins: None,
        };
        let mut headers = HeaderMap::new();
        headers.insert(
            http::header::COOKIE,
            http::HeaderValue::from_str(&format!("vkdg_session={}", session.session_id)).unwrap(),
        );
        let resp = list_connections(State(state), headers).await;
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let item = &v["items"][0];
        assert_eq!(item["status"], "cooldown", "{v}");
        assert_eq!(item["active_requests"], 1, "{v}");
        assert!(item["cooldown_until"].is_string(), "{v}");
        drop(guard);
    }
}
