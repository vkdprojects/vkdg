use axum::{
    extract::{Query, State},
    response::{IntoResponse, Response},
    Json,
};
use http::{HeaderMap, StatusCode};
use serde::{Deserialize, Serialize};
use crate::{
    error::{AdminError, AdminErrorResponse},
    handlers::session::get_session,
    router::AdminState,
};

#[derive(Serialize)]
struct RouteSummary {
    id: String,
    match_models: Vec<String>,
    strategy: String,
    targets: Vec<String>,
}

#[derive(Serialize)]
struct RouteList {
    items: Vec<RouteSummary>,
}

#[derive(Serialize)]
struct ExcludedConn {
    id: String,
    reason: String,
}

#[derive(Serialize)]
struct ConnectionPreview {
    id: String,
    provider: String,
}

#[derive(Serialize)]
struct RoutePreview {
    model: String,
    combo_id: Option<String>,
    eligible_connections: Vec<ConnectionPreview>,
    excluded_connections: Vec<ExcludedConn>,
}

#[derive(Deserialize)]
pub struct PreviewQuery {
    model: String,
}

fn strategy_str(s: &impl serde::Serialize) -> String {
    serde_json::to_value(s)
        .ok()
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "unknown".to_string())
}

fn provider_str(kind: &vkdg_connections::ProviderKind) -> String {
    match kind {
        vkdg_connections::ProviderKind::Anthropic => "anthropic".into(),
        vkdg_connections::ProviderKind::OpenAI => "openai".into(),
        vkdg_connections::ProviderKind::Google => "google".into(),
        vkdg_connections::ProviderKind::Custom { .. } => "custom".into(),
    }
}

pub async fn list_routes(State(state): State<AdminState>, headers: HeaderMap) -> Response {
    if get_session(&state, &headers).is_none() {
        return AdminErrorResponse(StatusCode::UNAUTHORIZED, AdminError::unauthorized())
            .into_response();
    }
    let snapshot = state.config_rx.borrow().clone();
    let items: Vec<RouteSummary> = snapshot
        .routes
        .iter()
        .map(|r| RouteSummary {
            id: r.id.0.clone(),
            match_models: r.match_models.clone(),
            strategy: strategy_str(&r.strategy).to_string(),
            targets: r.targets.iter().map(|t| t.0.clone()).collect(),
        })
        .collect();
    Json(RouteList { items }).into_response()
}

pub async fn preview_route(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Query(q): Query<PreviewQuery>,
) -> Response {
    if get_session(&state, &headers).is_none() {
        return AdminErrorResponse(StatusCode::UNAUTHORIZED, AdminError::unauthorized())
            .into_response();
    }
    let snapshot = state.config_rx.borrow().clone();
    let combo_id = state.combo_resolver.as_ref()
        .and_then(|r| r.resolve(&q.model))
        .map(|c| c.id.clone());
    let mut eligible = Vec::new();
    let mut excluded = Vec::new();
    for conn in snapshot.connections.iter() {
        let matches = conn.models.iter().any(|pattern| {
            if let Some(prefix) = pattern.strip_suffix('*') {
                q.model.starts_with(prefix)
            } else {
                pattern == &q.model
            }
        });
        if matches {
            eligible.push(ConnectionPreview {
                id: conn.id.0.clone(),
                provider: provider_str(&conn.provider),
            });
        } else {
            excluded.push(ExcludedConn {
                id: conn.id.0.clone(),
                reason: "model pattern does not match".into(),
            });
        }
    }
    Json(RoutePreview {
        model: q.model,
        combo_id,
        eligible_connections: eligible,
        excluded_connections: excluded,
    })
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Instant;
    use axum::extract::State;
    use http::HeaderMap;
    use tokio::sync::watch;
    use vkdg_config::{ConfigSnapshot, GatewayConfig, schema::{AuthDef, ConnectionDef, RouteDef}};
    use crate::session::{KeyStore, SessionStore};
    use crate::handlers::requests::RequestLog;

    fn make_state_empty() -> AdminState {
        let snap = ConfigSnapshot::default_empty();
        let (_tx, rx) = watch::channel(Arc::new(snap));
        AdminState {
            sessions: SessionStore::new("tok".into()),
            config_rx: rx,
            started_at: Arc::new(Instant::now()),
            key_store: KeyStore::new(),
            request_log: RequestLog::new(),
            combo_resolver: None,
        }
    }

    fn make_state_with_route() -> AdminState {
        let cfg = GatewayConfig {
            listen: "0.0.0.0:8080".into(),
            connections: vec![ConnectionDef {
                id: "conn-a".into(),
                provider: "anthropic".into(),
                auth: AuthDef::ApiKey { env_var: "KEY".into() },
                models: vec!["claude-*".into()],
                max_concurrent: None,
                weight: None,
            }],
            routes: vec![RouteDef {
                id: "r1".into(),
                match_models: vec!["claude-*".into()],
                strategy: "round_robin".into(),
                targets: vec!["conn-a".into()],
            }],
            limits: None,
            observe: None,
        };
        let snap = ConfigSnapshot::build(1, cfg).expect("build snapshot");
        let (_tx, rx) = watch::channel(Arc::new(snap));
        AdminState {
            sessions: SessionStore::new("tok".into()),
            config_rx: rx,
            started_at: Arc::new(Instant::now()),
            key_store: KeyStore::new(),
            request_log: RequestLog::new(),
            combo_resolver: None,
        }
    }

    fn authed_headers(state: &AdminState) -> HeaderMap {
        let session = state.sessions.bootstrap_login().unwrap();
        let mut h = HeaderMap::new();
        let val = format!("vkdg_session={}", session.session_id);
        h.insert(http::header::COOKIE, http::HeaderValue::from_str(&val).unwrap());
        h
    }

    #[tokio::test]
    async fn list_routes_requires_session() {
        let state = make_state_empty();
        let resp = list_routes(State(state), HeaderMap::new()).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn preview_route_returns_eligible_and_excluded() {
        let state = make_state_with_route();
        let headers = authed_headers(&state);
        let q = PreviewQuery { model: "claude-3-opus".into() };
        let resp = preview_route(State(state), headers, Query(q)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let val: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(val["eligible_connections"].as_array().unwrap().len(), 1);
        assert_eq!(val["eligible_connections"][0]["id"], "conn-a");
        assert_eq!(val["eligible_connections"][0]["provider"], "anthropic");
    }
}
