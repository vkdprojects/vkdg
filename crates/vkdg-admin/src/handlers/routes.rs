use crate::{
    error::AdminError,
    handlers::{response as resp, session::get_session},
    router::AdminState,
};
use axum::{
    extract::{Path, Query, State},
    response::{IntoResponse, Response},
    Json,
};
use http::{HeaderMap, StatusCode};
use serde::{Deserialize, Serialize};
use vkdg_config::schema::RouteDef;

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

pub async fn list_routes(State(state): State<AdminState>, headers: HeaderMap) -> Response {
    if get_session(&state, &headers).is_none() {
        return resp::unauthorized();
    }
    let snapshot = state.config_rx.borrow().clone();
    let items: Vec<RouteSummary> = snapshot
        .routes
        .iter()
        .map(|r| RouteSummary {
            id: r.id.0.clone(),
            match_models: r.match_models.clone(),
            strategy: strategy_str(&r.strategy),
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
        return resp::unauthorized();
    }
    let snapshot = state.config_rx.borrow().clone();
    let combo_id = state
        .combos
        .as_ref()
        .and_then(|s| s.resolver().resolve(&q.model))
        .map(|c| c.id);
    let mut eligible = Vec::new();
    let mut excluded = Vec::new();
    if let Some(catalog) = &state.catalog {
        let eligible_ids =
            catalog.eligible_for_operation(&q.model, &[], &vkdg_core::CapabilitySet::default());
        let eligible_set: std::collections::HashSet<&str> =
            eligible_ids.iter().map(|c| c.0.as_str()).collect();
        for conn in snapshot.connections.iter() {
            if eligible_set.contains(conn.id.0.as_str()) {
                eligible.push(ConnectionPreview {
                    id: conn.id.0.clone(),
                    provider: conn.provider.as_str().to_string(),
                });
            } else {
                excluded.push(ExcludedConn {
                    id: conn.id.0.clone(),
                    reason: "model pattern does not match".into(),
                });
            }
        }
    } else {
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
                    provider: conn.provider.as_str().to_string(),
                });
            } else {
                excluded.push(ExcludedConn {
                    id: conn.id.0.clone(),
                    reason: "model pattern does not match".into(),
                });
            }
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

// ── CRUD ──────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct CreateRouteBody {
    pub id: String,
    pub match_models: Vec<String>,
    /// `"round_robin"` | `"fallback_chain"` | `"lowest_latency"` | `"power_of_two_choices"`
    pub strategy: String,
    /// Connection ids.
    pub targets: Vec<String>,
    #[serde(default)]
    pub hooks: vkdg_routing::PluginHooks,
}

impl CreateRouteBody {
    fn into_def(self, id: String) -> RouteDef {
        RouteDef {
            id,
            match_models: self.match_models,
            strategy: self.strategy,
            targets: self.targets,
            hooks: self.hooks,
        }
    }
}

pub async fn create_route(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Json(body): Json<CreateRouteBody>,
) -> Response {
    if get_session(&state, &headers).is_none() {
        return resp::unauthorized();
    }
    if body.id.trim().is_empty() {
        return resp::validation("id must not be empty");
    }
    if body.targets.is_empty() {
        return resp::validation("targets must not be empty");
    }
    let store = match resp::require_store(&state) {
        Ok(s) => s,
        Err(r) => return r,
    };
    let id = body.id.clone();
    let def = body.into_def(id.clone());
    if let Err(e) = store.upsert_route(&def) {
        return resp::store_error(e);
    }
    match crate::handlers::connections::rebuild_and_push(&state) {
        Err(e) => resp::internal(e),
        Ok(snap) => {
            let route = snap.routes.iter().find(|r| r.id.0 == id);
            match route {
                None => resp::internal(AdminError::new("not_found", "route missing after upsert")),
                Some(r) => (
                    StatusCode::CREATED,
                    Json(RouteSummary {
                        id: r.id.0.clone(),
                        match_models: r.match_models.clone(),
                        strategy: strategy_str(&r.strategy),
                        targets: r.targets.iter().map(|t| t.0.clone()).collect(),
                    }),
                )
                    .into_response(),
            }
        }
    }
}

pub async fn update_route(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<CreateRouteBody>,
) -> Response {
    if get_session(&state, &headers).is_none() {
        return resp::unauthorized();
    }
    if body.targets.is_empty() {
        return resp::validation("targets must not be empty");
    }
    // 404 if not in current snapshot
    {
        let snapshot = state.config_rx.borrow().clone();
        if !snapshot.routes.iter().any(|r| r.id.0 == id) {
            return resp::not_found(&id);
        }
    }
    let store = match resp::require_store(&state) {
        Ok(s) => s,
        Err(r) => return r,
    };
    let def = body.into_def(id.clone());
    if let Err(e) = store.upsert_route(&def) {
        return resp::store_error(e);
    }
    match crate::handlers::connections::rebuild_and_push(&state) {
        Err(e) => resp::internal(e),
        Ok(snap) => {
            let route = snap.routes.iter().find(|r| r.id.0 == id);
            match route {
                None => resp::internal(AdminError::new("not_found", "route missing after upsert")),
                Some(r) => Json(RouteSummary {
                    id: r.id.0.clone(),
                    match_models: r.match_models.clone(),
                    strategy: strategy_str(&r.strategy),
                    targets: r.targets.iter().map(|t| t.0.clone()).collect(),
                })
                .into_response(),
            }
        }
    }
}

pub async fn delete_route(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if get_session(&state, &headers).is_none() {
        return resp::unauthorized();
    }
    let store = match resp::require_store(&state) {
        Ok(s) => s,
        Err(r) => return r,
    };
    if let Err(e) = store.delete_route(&id) {
        return resp::store_error(e);
    }
    if let Err(e) = crate::handlers::connections::rebuild_and_push(&state) {
        return resp::internal(e);
    }
    StatusCode::NO_CONTENT.into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handlers::requests::RequestLog;
    use crate::session::SessionStore;
    use axum::extract::State;
    use http::HeaderMap;
    use std::sync::Arc;
    use tokio::sync::watch;
    use vkdg_config::{
        schema::{AuthDef, ConnectionDef, RouteDef},
        ConfigSnapshot, GatewayConfig,
    };

    fn make_state_empty() -> AdminState {
        let snap = vkdg_config::ConfigSnapshot::default_empty();
        let (_tx, rx) = tokio::sync::watch::channel(std::sync::Arc::new(snap));
        AdminState {
            sessions: SessionStore::new("tok".into()),
            config_rx: rx,
            started_at: std::sync::Arc::new(std::time::Instant::now()),
            key_store: std::sync::Arc::new(vkdg_governance::VirtualKeyStore::in_memory().unwrap()),
            request_log: RequestLog::new(),
            combos: None,
            reload_plugins: None,
            connection_tester: None,
            catalog: None,
            logins: None,
            gateway_store: None,
            config_tx: None,
        }
    }

    fn make_state_with_route() -> AdminState {
        let cfg = GatewayConfig {
            listen: "0.0.0.0:8080".into(),
            connections: vec![ConnectionDef {
                id: "conn-a".into(),
                provider: "anthropic".into(),
                auth: AuthDef::ApiKey {
                    env_var: "KEY".into(),
                },
                models: vec!["claude-*".into()],
                max_concurrent: None,
                weight: None,
                base_url: None,
                tags: vec![],
                endpoint: None,
            }],
            routes: vec![RouteDef {
                id: "r1".into(),
                match_models: vec!["claude-*".into()],
                strategy: "round_robin".into(),
                targets: vec!["conn-a".into()],
                hooks: Default::default(),
            }],
            limits: None,
            observe: None,
            global_system_prompt: None,
        };
        let snap = ConfigSnapshot::build(1, cfg).expect("build snapshot");
        let (_tx, rx) = watch::channel(Arc::new(snap));
        AdminState {
            sessions: SessionStore::new("tok".into()),
            config_rx: rx,
            started_at: std::sync::Arc::new(std::time::Instant::now()),
            key_store: std::sync::Arc::new(vkdg_governance::VirtualKeyStore::in_memory().unwrap()),
            request_log: RequestLog::new(),
            combos: None,
            reload_plugins: None,
            connection_tester: None,
            catalog: None,
            logins: None,
            gateway_store: None,
            config_tx: None,
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
    async fn list_routes_requires_session() {
        let state = make_state_empty();
        let resp = list_routes(State(state), HeaderMap::new()).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn preview_route_returns_eligible_and_excluded() {
        let state = make_state_with_route();
        let headers = authed_headers(&state);
        let q = PreviewQuery {
            model: "claude-3-opus".into(),
        };
        let resp = preview_route(State(state), headers, Query(q)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let val: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(val["eligible_connections"].as_array().unwrap().len(), 1);
        assert_eq!(val["eligible_connections"][0]["id"], "conn-a");
        assert_eq!(val["eligible_connections"][0]["provider"], "anthropic");
    }
}
