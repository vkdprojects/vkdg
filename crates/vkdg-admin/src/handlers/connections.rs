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
use std::sync::Arc;
use vkdg_config::schema::{AuthDef, ConnectionDef};

#[derive(Serialize)]
pub struct ConnectionSummary {
    id: String,
    provider: String,
    /// `healthy`, `degraded`, `circuit_open`, `cooldown`; `unknown` when the
    /// data plane is not running (no catalog).
    status: &'static str,
    model_count: usize,
    /// Model patterns this connection serves (`claude-*`, exact ids).
    models: Vec<String>,
    weight: u32,
    active_requests: u32,
    max_concurrent: u32,
    /// Set while the connection is cooling down or its circuit is open.
    #[serde(skip_serializing_if = "Option::is_none")]
    cooldown_until: Option<String>,
    /// The provider account backing this connection, when it authenticates via
    /// a persisted account (`AuthKind::Account`). Omitted for API-key/OAuth2.
    #[serde(skip_serializing_if = "Option::is_none")]
    account_id: Option<String>,
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
    let account_id = match &conn.auth {
        vkdg_connections::AuthKind::Account { account_id } => Some(account_id.clone()),
        _ => None,
    };
    let mut summary = ConnectionSummary {
        id: conn.id.0.clone(),
        provider: conn.provider.as_str().to_string(),
        status: "unknown",
        model_count: conn.models.len(),
        models: conn.models.clone(),
        weight: conn.weight,
        active_requests: 0,
        max_concurrent: conn.max_concurrent,
        cooldown_until: None,
        account_id,
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

pub async fn test_connection(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if get_session(&state, &headers).is_none() {
        return AdminErrorResponse(StatusCode::UNAUTHORIZED, AdminError::unauthorized())
            .into_response();
    }
    let Some(tester) = &state.connection_tester else {
        return AdminErrorResponse(
            StatusCode::SERVICE_UNAVAILABLE,
            AdminError::new(
                "no_pipeline",
                "connection testing needs a running data plane",
            ),
        )
        .into_response();
    };
    let result = tester(id).await;
    Json(result).into_response()
}

// ── CRUD body types ────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct CreateConnectionBody {
    pub id: String,
    pub provider: String,
    pub base_url: Option<String>,
    pub endpoint: Option<String>,
    pub auth: AuthDef,
    pub models: Vec<String>,
    pub max_concurrent: Option<u32>,
    pub weight: Option<u32>,
    #[serde(default)]
    pub tags: Vec<String>,
}

impl CreateConnectionBody {
    fn into_def(self, id: String) -> ConnectionDef {
        ConnectionDef {
            id,
            provider: self.provider,
            base_url: self.base_url,
            endpoint: self.endpoint,
            auth: self.auth,
            models: self.models,
            max_concurrent: self.max_concurrent,
            weight: self.weight,
            tags: self.tags,
        }
    }
}

/// Rebuild snapshot from store data, preserving limits/observe, and push.
pub fn rebuild_and_push(
    state: &AdminState,
) -> Result<Arc<vkdg_config::ConfigSnapshot>, AdminError> {
    let store = state
        .gateway_store
        .as_ref()
        .ok_or_else(|| AdminError::new("no_store", "gateway store not available"))?;
    let (conn_defs, route_defs) = store
        .load()
        .map_err(|e| AdminError::new("store_error", e.to_string()))?;
    let current = state.config_rx.borrow().clone();
    let mut gateway = (*current.gateway).clone();
    gateway.connections = conn_defs;
    gateway.routes = route_defs;
    let new_version = current.version + 1;
    let snap = vkdg_config::ConfigSnapshot::build(new_version, gateway)
        .map_err(|e| AdminError::new("config_invalid", e.to_string()))?;
    let snap = Arc::new(snap);
    if let Some(tx) = &state.config_tx {
        let _ = tx.send(snap.clone());
    }
    Ok(snap)
}

fn store_error(e: impl std::fmt::Display) -> AdminError {
    AdminError::new("store_error", e.to_string())
}

/// The connection that authenticates with `account_id`, whoever created it
/// (console, YAML seed, hand-edited store).
fn connection_for_account(conns: &[ConnectionDef], account_id: &str) -> Option<String> {
    conns
        .iter()
        .find(|c| matches!(&c.auth, AuthDef::Account { account } if account == account_id))
        .map(|c| c.id.clone())
}

/// Id of the connection serving `account_id`, if any.
pub fn connection_id_for_account(
    state: &AdminState,
    account_id: &str,
) -> Result<Option<String>, AdminError> {
    let Some(store) = &state.gateway_store else {
        return Ok(None);
    };
    let (conns, _) = store.load().map_err(store_error)?;
    Ok(connection_for_account(&conns, account_id))
}

/// Make sure `account_id` has a connection, creating one with the provider's
/// default model patterns when none exists, and push it to the live snapshot.
/// Returns the connection id, or `None` when the provider declares no default
/// models (it cannot serve requests yet, so a connection would only steal
/// traffic). An existing connection is never touched: a reconnect keeps the
/// operator's models, weight and limits.
pub fn ensure_account_connection(
    state: &AdminState,
    provider: &dyn vkdg_provider_sdk::ProviderAdapter,
    account_id: &str,
) -> Result<Option<String>, AdminError> {
    let Some(store) = &state.gateway_store else {
        return Ok(None);
    };
    let (conns, _) = store.load().map_err(store_error)?;
    if let Some(existing) = connection_for_account(&conns, account_id) {
        return Ok(Some(existing));
    }
    let models = provider.default_models();
    if models.is_empty() {
        return Ok(None);
    }
    // Account ids are unique, so their id is the natural connection id; fall
    // back to a suffix when the operator already used that name.
    let taken = |id: &str| conns.iter().any(|c| c.id == id);
    let mut id = account_id.to_owned();
    let mut n = 1;
    while taken(&id) {
        n += 1;
        id = format!("{account_id}-{n}");
    }
    store
        .upsert_connection(&ConnectionDef {
            id: id.clone(),
            provider: provider.id().to_owned(),
            base_url: None,
            endpoint: None,
            auth: AuthDef::Account {
                account: account_id.to_owned(),
            },
            models,
            max_concurrent: None,
            weight: None,
            tags: vec![],
        })
        .map_err(store_error)?;
    if let Err(e) = rebuild_and_push(state) {
        // Do not leave a connection the snapshot rejects: it would make every
        // later console edit fail the same way.
        let _ = store.delete_connection(&id);
        return Err(e);
    }
    Ok(Some(id))
}

/// Drop connection `id` from the store and from every route that named it; a
/// route left without targets goes too. A route naming a missing connection
/// makes the whole snapshot invalid, and every later console edit would fail.
pub fn remove_connection(store: &vkdg_config::GatewayStore, id: &str) -> Result<(), AdminError> {
    store.delete_connection(id).map_err(store_error)?;
    let (_, routes) = store.load().map_err(store_error)?;
    for mut route in routes
        .into_iter()
        .filter(|r| r.targets.iter().any(|t| t == id))
    {
        route.targets.retain(|t| t != id);
        if route.targets.is_empty() {
            store.delete_route(&route.id).map_err(store_error)?;
        } else {
            store.upsert_route(&route).map_err(store_error)?;
        }
    }
    Ok(())
}

/// Remove every connection that authenticates with `account_id` and push the
/// result to the live snapshot.
pub fn remove_account_connections(state: &AdminState, account_id: &str) -> Result<(), AdminError> {
    let Some(store) = &state.gateway_store else {
        return Ok(());
    };
    let (conns, _) = store.load().map_err(store_error)?;
    let ids: Vec<String> = conns
        .iter()
        .filter(|c| matches!(&c.auth, AuthDef::Account { account } if account == account_id))
        .map(|c| c.id.clone())
        .collect();
    if ids.is_empty() {
        return Ok(());
    }
    for id in &ids {
        remove_connection(store, id)?;
    }
    rebuild_and_push(state).map(|_| ())
}

pub async fn create_connection(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Json(body): Json<CreateConnectionBody>,
) -> Response {
    if get_session(&state, &headers).is_none() {
        return AdminErrorResponse(StatusCode::UNAUTHORIZED, AdminError::unauthorized())
            .into_response();
    }
    if body.id.trim().is_empty() {
        return AdminErrorResponse(
            StatusCode::UNPROCESSABLE_ENTITY,
            AdminError::new("validation_error", "id must not be empty"),
        )
        .into_response();
    }
    if body.models.is_empty() {
        return AdminErrorResponse(
            StatusCode::UNPROCESSABLE_ENTITY,
            AdminError::new("validation_error", "models must not be empty"),
        )
        .into_response();
    }
    let id = body.id.clone();
    let def = body.into_def(id.clone());
    let Some(store) = &state.gateway_store else {
        return AdminErrorResponse(
            StatusCode::SERVICE_UNAVAILABLE,
            AdminError::new("no_store", "gateway store not available"),
        )
        .into_response();
    };
    if let Err(e) = store.upsert_connection(&def) {
        return AdminErrorResponse(
            StatusCode::INTERNAL_SERVER_ERROR,
            AdminError::new("store_error", e.to_string()),
        )
        .into_response();
    }
    match rebuild_and_push(&state) {
        Err(e) => AdminErrorResponse(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
        Ok(snap) => {
            let conn = snap.connections.iter().find(|c| c.id.0 == def.id);
            match conn {
                None => AdminErrorResponse(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    AdminError::new("not_found", "connection missing after upsert"),
                )
                .into_response(),
                Some(c) => (StatusCode::CREATED, Json(summarize(&state, c).await)).into_response(),
            }
        }
    }
}

pub async fn update_connection(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<CreateConnectionBody>,
) -> Response {
    if get_session(&state, &headers).is_none() {
        return AdminErrorResponse(StatusCode::UNAUTHORIZED, AdminError::unauthorized())
            .into_response();
    }
    if body.models.is_empty() {
        return AdminErrorResponse(
            StatusCode::UNPROCESSABLE_ENTITY,
            AdminError::new("validation_error", "models must not be empty"),
        )
        .into_response();
    }
    // 404 if not in current snapshot
    {
        let snapshot = state.config_rx.borrow().clone();
        if !snapshot.connections.iter().any(|c| c.id.0 == id) {
            return AdminErrorResponse(StatusCode::NOT_FOUND, AdminError::not_found(&id))
                .into_response();
        }
    }
    let def = body.into_def(id.clone());
    let Some(store) = &state.gateway_store else {
        return AdminErrorResponse(
            StatusCode::SERVICE_UNAVAILABLE,
            AdminError::new("no_store", "gateway store not available"),
        )
        .into_response();
    };
    if let Err(e) = store.upsert_connection(&def) {
        return AdminErrorResponse(
            StatusCode::INTERNAL_SERVER_ERROR,
            AdminError::new("store_error", e.to_string()),
        )
        .into_response();
    }
    match rebuild_and_push(&state) {
        Err(e) => AdminErrorResponse(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
        Ok(snap) => {
            let conn = snap.connections.iter().find(|c| c.id.0 == id);
            match conn {
                None => AdminErrorResponse(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    AdminError::new("not_found", "connection missing after upsert"),
                )
                .into_response(),
                Some(c) => Json(summarize(&state, c).await).into_response(),
            }
        }
    }
}

/// Partial update from the console: only the fields it edits. Everything else
/// (auth, endpoint, `base_url`, tags) stays as stored, which a PUT would have to
/// resend and could silently drop.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchConnectionBody {
    models: Option<Vec<String>>,
    max_concurrent: Option<u32>,
    weight: Option<u32>,
}

fn validation(message: &str) -> Response {
    AdminErrorResponse(
        StatusCode::UNPROCESSABLE_ENTITY,
        AdminError::new("validation_error", message),
    )
    .into_response()
}

pub async fn patch_connection(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<PatchConnectionBody>,
) -> Response {
    if get_session(&state, &headers).is_none() {
        return AdminErrorResponse(StatusCode::UNAUTHORIZED, AdminError::unauthorized())
            .into_response();
    }
    if body.models.is_none() && body.max_concurrent.is_none() && body.weight.is_none() {
        return validation("nothing to change: send models, max_concurrent or weight");
    }
    if body.max_concurrent == Some(0) || body.weight == Some(0) {
        return validation("max_concurrent and weight must be at least 1");
    }
    let models = body.models.map(|m| {
        m.into_iter()
            .map(|p| p.trim().to_owned())
            .filter(|p| !p.is_empty())
            .collect::<Vec<_>>()
    });
    if models.as_ref().is_some_and(Vec::is_empty) {
        return validation("models must not be empty");
    }
    let Some(store) = &state.gateway_store else {
        return AdminErrorResponse(
            StatusCode::SERVICE_UNAVAILABLE,
            AdminError::new("no_store", "gateway store not available"),
        )
        .into_response();
    };
    let (conns, _) = match store.load() {
        Ok(v) => v,
        Err(e) => {
            return AdminErrorResponse(StatusCode::INTERNAL_SERVER_ERROR, store_error(e))
                .into_response()
        }
    };
    let Some(old) = conns.into_iter().find(|c| c.id == id) else {
        return AdminErrorResponse(StatusCode::NOT_FOUND, AdminError::not_found(&id))
            .into_response();
    };
    let mut def = old.clone();
    if let Some(models) = models {
        def.models = models;
    }
    if body.max_concurrent.is_some() {
        def.max_concurrent = body.max_concurrent;
    }
    if body.weight.is_some() {
        def.weight = body.weight;
    }
    if let Err(e) = store.upsert_connection(&def) {
        return AdminErrorResponse(StatusCode::INTERNAL_SERVER_ERROR, store_error(e))
            .into_response();
    }
    match rebuild_and_push(&state) {
        Err(e) => {
            // Keep the stored def the snapshot accepted.
            let _ = store.upsert_connection(&old);
            AdminErrorResponse(StatusCode::UNPROCESSABLE_ENTITY, e).into_response()
        }
        Ok(snap) => match snap.connections.iter().find(|c| c.id.0 == id) {
            Some(c) => Json(summarize(&state, c).await).into_response(),
            None => AdminErrorResponse(
                StatusCode::INTERNAL_SERVER_ERROR,
                AdminError::new("not_found", "connection missing after update"),
            )
            .into_response(),
        },
    }
}

pub async fn delete_connection(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if get_session(&state, &headers).is_none() {
        return AdminErrorResponse(StatusCode::UNAUTHORIZED, AdminError::unauthorized())
            .into_response();
    }
    let Some(store) = &state.gateway_store else {
        return AdminErrorResponse(
            StatusCode::SERVICE_UNAVAILABLE,
            AdminError::new("no_store", "gateway store not available"),
        )
        .into_response();
    };
    if let Err(e) = remove_connection(store, &id) {
        return AdminErrorResponse(StatusCode::INTERNAL_SERVER_ERROR, e).into_response();
    }
    if let Err(e) = rebuild_and_push(&state) {
        return AdminErrorResponse(StatusCode::INTERNAL_SERVER_ERROR, e).into_response();
    }
    StatusCode::NO_CONTENT.into_response()
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
            connection_tester: None,
            catalog: None,
            logins: None,
            gateway_store: None,
            config_tx: None,
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
                base_url: None,
                tags: vec![],
                endpoint: None,
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
            connection_tester: None,
            catalog: None,
            logins: None,
            gateway_store: None,
            config_tx: None,
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

    #[tokio::test]
    async fn list_reports_account_id_for_account_auth_and_omits_it_otherwise() {
        use vkdg_config::{schema::AuthDef, ConfigSnapshot, ConnectionDef, GatewayConfig};

        let gateway_cfg = GatewayConfig {
            listen: "0.0.0.0:8080".into(),
            connections: vec![
                ConnectionDef {
                    id: "acct-conn".into(),
                    provider: "kiro".into(),
                    auth: AuthDef::Account {
                        account: "kiro-ab12cd34".into(),
                    },
                    models: vec!["*".into()],
                    max_concurrent: None,
                    weight: None,
                    base_url: None,
                    tags: vec![],
                    endpoint: None,
                },
                ConnectionDef {
                    id: "key-conn".into(),
                    provider: "anthropic".into(),
                    auth: AuthDef::ApiKey {
                        env_var: "ANTHROPIC_API_KEY".into(),
                    },
                    models: vec!["claude-*".into()],
                    max_concurrent: None,
                    weight: None,
                    base_url: None,
                    tags: vec![],
                    endpoint: None,
                },
            ],
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
            connection_tester: None,
            catalog: None,
            logins: None,
            gateway_store: None,
            config_tx: None,
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
        let items = list["items"].as_array().unwrap();
        let acct = items.iter().find(|i| i["id"] == "acct-conn").unwrap();
        assert_eq!(acct["account_id"], "kiro-ab12cd34", "{acct}");
        let key = items.iter().find(|i| i["id"] == "key-conn").unwrap();
        assert!(
            key.get("account_id").is_none(),
            "api-key connection must not serialize account_id: {key}"
        );
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
                base_url: None,
                tags: vec![],
                endpoint: None,
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
            connection_tester: None,
            catalog: Some(catalog),
            logins: None,
            gateway_store: None,
            config_tx: None,
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

    fn make_state_with_store() -> (
        AdminState,
        tokio::sync::watch::Sender<std::sync::Arc<vkdg_config::ConfigSnapshot>>,
    ) {
        use vkdg_config::GatewayStore;
        let snap = vkdg_config::ConfigSnapshot::default_empty();
        let (tx, rx) = tokio::sync::watch::channel(std::sync::Arc::new(snap));
        let store = Arc::new(GatewayStore::in_memory().unwrap());
        let state = AdminState {
            sessions: crate::session::SessionStore::new("tok".into()),
            config_rx: rx,
            started_at: Arc::new(Instant::now()),
            key_store: Arc::new(vkdg_governance::VirtualKeyStore::in_memory().unwrap()),
            request_log: crate::handlers::requests::RequestLog::new(),
            combos: None,
            reload_plugins: None,
            connection_tester: None,
            catalog: None,
            logins: None,
            gateway_store: Some(store),
            config_tx: Some(tx.clone()),
        };
        (state, tx)
    }

    fn authed_headers(state: &AdminState) -> HeaderMap {
        let session = state.sessions.bootstrap_login().unwrap();
        let mut h = HeaderMap::new();
        h.insert(
            http::header::COOKIE,
            http::HeaderValue::from_str(&format!("vkdg_session={}", session.session_id)).unwrap(),
        );
        h
    }

    #[tokio::test]
    async fn create_connection_returns_201() {
        let (state, _tx) = make_state_with_store();
        let headers = authed_headers(&state);
        let body = CreateConnectionBody {
            id: "new-conn".into(),
            provider: "anthropic".into(),
            base_url: None,
            endpoint: None,
            auth: vkdg_config::schema::AuthDef::ApiKey {
                env_var: "ANTHROPIC_API_KEY".into(),
            },
            models: vec!["claude-*".into()],
            max_concurrent: None,
            weight: None,
            tags: vec![],
        };
        let resp = create_connection(State(state), headers, Json(body)).await;
        assert_eq!(resp.status(), StatusCode::CREATED);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let val: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(val["id"], "new-conn");
        assert_eq!(val["provider"], "anthropic");
    }

    #[tokio::test]
    async fn delete_connection_returns_204() {
        use vkdg_config::{schema::AuthDef as AD, ConfigSnapshot, ConnectionDef, GatewayConfig};
        let cfg = GatewayConfig {
            listen: "0.0.0.0:8080".into(),
            connections: vec![ConnectionDef {
                id: "del-me".into(),
                provider: "anthropic".into(),
                auth: AD::ApiKey {
                    env_var: "KEY".into(),
                },
                models: vec!["*".into()],
                max_concurrent: None,
                weight: None,
                base_url: None,
                tags: vec![],
                endpoint: None,
            }],
            routes: vec![],
            limits: None,
            observe: None,
            global_system_prompt: None,
        };
        let snap = ConfigSnapshot::build(1, cfg).unwrap();
        let (tx, rx) = tokio::sync::watch::channel(Arc::new(snap));
        let store = Arc::new(vkdg_config::GatewayStore::in_memory().unwrap());
        // seed store so delete has something to remove
        store
            .upsert_connection(&ConnectionDef {
                id: "del-me".into(),
                provider: "anthropic".into(),
                auth: AD::ApiKey {
                    env_var: "KEY".into(),
                },
                models: vec!["*".into()],
                max_concurrent: None,
                weight: None,
                base_url: None,
                tags: vec![],
                endpoint: None,
            })
            .unwrap();
        let state = AdminState {
            sessions: crate::session::SessionStore::new("tok".into()),
            config_rx: rx,
            started_at: Arc::new(Instant::now()),
            key_store: Arc::new(vkdg_governance::VirtualKeyStore::in_memory().unwrap()),
            request_log: crate::handlers::requests::RequestLog::new(),
            combos: None,
            reload_plugins: None,
            connection_tester: None,
            catalog: None,
            logins: None,
            gateway_store: Some(store),
            config_tx: Some(tx),
        };
        let headers = authed_headers(&state);
        let resp = delete_connection(State(state), headers, Path("del-me".into())).await;
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    }

    /// State whose store and live snapshot hold one account connection with
    /// every optional field set, so a patch that rebuilds the def from scratch
    /// shows up as lost data.
    fn state_with_account_connection() -> (AdminState, Arc<vkdg_config::GatewayStore>) {
        let (state, _tx) = make_state_with_store();
        let store = state.gateway_store.clone().unwrap();
        store
            .upsert_connection(&ConnectionDef {
                id: "kiro-1".into(),
                provider: "kiro".into(),
                base_url: None,
                endpoint: Some("runtime".into()),
                auth: AuthDef::Account {
                    account: "kiro-1".into(),
                },
                models: vec!["claude-*".into()],
                max_concurrent: Some(10),
                weight: Some(1),
                tags: vec!["x".into()],
            })
            .unwrap();
        rebuild_and_push(&state).unwrap();
        (state, store)
    }

    async fn patch(
        state: &AdminState,
        headers: &HeaderMap,
        id: &str,
        body: serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        let resp = patch_connection(
            State(state.clone()),
            headers.clone(),
            Path(id.into()),
            Json(serde_json::from_value(body).unwrap()),
        )
        .await;
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or_default())
    }

    // Plausible wrong impls: PUT-style rebuild drops endpoint/tags/auth; the
    // change reaches the store but not the live snapshot; fields not sent are reset.
    #[tokio::test]
    async fn patch_changes_only_the_fields_sent_and_applies_live() {
        let (state, store) = state_with_account_connection();

        let h = authed_headers(&state);
        let (status, v) = patch(
            &state,
            &h,
            "kiro-1",
            serde_json::json!({ "models": ["claude-*", " auto "], "weight": 3 }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{v}");
        assert_eq!(v["models"], serde_json::json!(["claude-*", "auto"]));
        assert_eq!(v["weight"], 3);
        assert_eq!(v["max_concurrent"], 10);

        let (conns, _) = store.load().unwrap();
        let c = &conns[0];
        assert_eq!(c.models, ["claude-*", "auto"]);
        assert_eq!((c.weight, c.max_concurrent), (Some(3), Some(10)));
        assert_eq!(c.endpoint.as_deref(), Some("runtime"));
        assert_eq!(c.tags, ["x"]);
        assert!(matches!(&c.auth, AuthDef::Account { account } if account == "kiro-1"));

        let live = state.config_rx.borrow().clone();
        assert_eq!(live.connections[0].weight, 3);
        assert_eq!(live.connections[0].models, ["claude-*", "auto"]);
    }

    // Plausible wrong impls: an empty model list silently makes the connection
    // unroutable; zero weight/concurrency starves it; an empty patch "succeeds".
    #[tokio::test]
    async fn patch_rejects_values_that_would_disable_the_connection() {
        let (state, store) = state_with_account_connection();
        let h = authed_headers(&state);
        let before = store.load().unwrap().0;

        for body in [
            serde_json::json!({ "models": [] }),
            serde_json::json!({ "models": ["  "] }),
            serde_json::json!({ "weight": 0 }),
            serde_json::json!({ "max_concurrent": 0 }),
            serde_json::json!({}),
        ] {
            let (status, v) = patch(&state, &h, "kiro-1", body.clone()).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body} -> {v}");
        }
        assert_eq!(
            serde_json::to_string(&store.load().unwrap().0).unwrap(),
            serde_json::to_string(&before).unwrap()
        );

        let (status, _) = patch(&state, &h, "nope", serde_json::json!({ "weight": 2 })).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let resp = patch_connection(
            State(state.clone()),
            HeaderMap::new(),
            Path("kiro-1".into()),
            Json(serde_json::from_value(serde_json::json!({ "weight": 2 })).unwrap()),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // The console shows and edits models and weight, so the list must carry them.
    #[tokio::test]
    async fn list_reports_models_and_weight() {
        let (state, _store) = state_with_account_connection();
        let resp = list_connections(State(state.clone()), authed_headers(&state)).await;
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["items"][0]["models"], serde_json::json!(["claude-*"]));
        assert_eq!(v["items"][0]["weight"], 1);
    }
}
