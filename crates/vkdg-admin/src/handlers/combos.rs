//! `/admin/v1/combos`: named routing plans, edited live.
//!
//! Every write goes through [`ComboService`], which validates, persists and
//! applies the change to the data plane before answering.

use axum::{
    extract::{Path, State},
    response::{IntoResponse, Response},
    Json,
};
use http::{HeaderMap, StatusCode};
use serde::{Deserialize, Serialize};
use vkdg_combos::{BudgetPolicy, Combo, ComboError, StrategyKind};
use vkdg_core::ConnectionId;

use crate::{
    error::{AdminError, AdminErrorResponse},
    handlers::session::get_session,
    router::AdminState,
};

#[derive(Serialize)]
pub struct ComboSummary {
    id: String,
    match_patterns: Vec<String>,
    strategy: StrategyKind,
    targets: Vec<String>,
    model: Option<String>,
    max_cost_microdollars: Option<u64>,
    has_compression: bool,
    has_cache: bool,
    has_budget: bool,
}

impl From<&Combo> for ComboSummary {
    fn from(c: &Combo) -> Self {
        Self {
            id: c.id.clone(),
            match_patterns: c.match_patterns.clone(),
            strategy: c.strategy.clone(),
            targets: c.targets.iter().map(|t| t.0.clone()).collect(),
            model: c.model.clone(),
            max_cost_microdollars: c.budget.as_ref().and_then(|b| b.max_cost_microdollars),
            has_compression: c.compression.is_some(),
            has_cache: c.cache.is_some(),
            has_budget: c.budget.is_some(),
        }
    }
}

#[derive(Serialize)]
struct ComboList {
    items: Vec<ComboSummary>,
    total: usize,
}

/// Body of POST (with `id`) and PUT (id from the path).
#[derive(Deserialize)]
pub struct ComboBody {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub match_patterns: Vec<String>,
    pub strategy: StrategyKind,
    pub targets: Vec<String>,
    pub model: Option<String>,
    /// Per-request cost cap in microdollars; absent = no cap.
    pub max_cost_microdollars: Option<u64>,
}

impl ComboBody {
    fn into_combo(self, id: String) -> Combo {
        Combo {
            id: id.trim().to_owned(),
            match_patterns: self
                .match_patterns
                .into_iter()
                .map(|p| p.trim().to_owned())
                .filter(|p| !p.is_empty())
                .collect(),
            strategy: self.strategy,
            targets: self
                .targets
                .into_iter()
                .map(|t| ConnectionId(t.trim().to_owned()))
                .collect(),
            model: self.model.map(|m| m.trim().to_owned()),
            compression: None,
            cache: None,
            budget: self.max_cost_microdollars.map(|max| BudgetPolicy {
                max_cost_microdollars: Some(max),
                overflow: "strict".into(),
            }),
            mode_pack: None,
        }
    }
}

fn err(status: StatusCode, code: &str, msg: impl Into<String>) -> Response {
    AdminErrorResponse(status, AdminError::new(code, msg)).into_response()
}

fn combo_err(e: ComboError, id: &str) -> Response {
    match e {
        ComboError::Invalid(m) => err(StatusCode::BAD_REQUEST, "invalid_input", m),
        ComboError::NotFound => {
            AdminErrorResponse(StatusCode::NOT_FOUND, AdminError::not_found(id)).into_response()
        }
        ComboError::Exists => err(
            StatusCode::CONFLICT,
            "conflict",
            format!("combo {id} already exists"),
        ),
        ComboError::Storage(m) => err(StatusCode::INTERNAL_SERVER_ERROR, "combo_store", m),
    }
}

/// Session check + the combo service. Combos are off when the gateway runs
/// without a data plane.
fn service(
    state: &AdminState,
    headers: &HeaderMap,
) -> Result<std::sync::Arc<vkdg_combos::ComboService>, Box<Response>> {
    if get_session(state, headers).is_none() {
        return Err(Box::new(
            AdminErrorResponse(StatusCode::UNAUTHORIZED, AdminError::unauthorized())
                .into_response(),
        ));
    }
    state.combos.clone().ok_or_else(|| {
        Box::new(err(
            StatusCode::SERVICE_UNAVAILABLE,
            "combos_disabled",
            "combos need a running data plane",
        ))
    })
}

/// A target that is not a connection would save fine and then fail every
/// request that routes to it.
fn unknown_target(state: &AdminState, combo: &Combo) -> Option<Response> {
    let catalog = state.catalog.as_ref()?;
    let known = catalog.connection_ids();
    combo.targets.iter().find(|t| !known.contains(t)).map(|t| {
        err(
            StatusCode::BAD_REQUEST,
            "invalid_input",
            format!("target {} is not a connection", t.0),
        )
    })
}

pub async fn list_combos(State(state): State<AdminState>, headers: HeaderMap) -> Response {
    if get_session(&state, &headers).is_none() {
        return AdminErrorResponse(StatusCode::UNAUTHORIZED, AdminError::unauthorized())
            .into_response();
    }
    let items: Vec<ComboSummary> = state
        .combos
        .as_ref()
        .map(|s| s.list().iter().map(ComboSummary::from).collect())
        .unwrap_or_default();
    let total = items.len();
    Json(ComboList { items, total }).into_response()
}

pub async fn create_combo(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Json(body): Json<ComboBody>,
) -> Response {
    let svc = match service(&state, &headers) {
        Ok(s) => s,
        Err(r) => return *r,
    };
    let id = body.id.clone();
    let combo = body.into_combo(id.clone());
    if let Some(r) = unknown_target(&state, &combo) {
        return r;
    }
    match svc.create(combo) {
        Ok(c) => (StatusCode::CREATED, Json(ComboSummary::from(&c))).into_response(),
        Err(e) => combo_err(e, &id),
    }
}

pub async fn update_combo(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<ComboBody>,
) -> Response {
    let svc = match service(&state, &headers) {
        Ok(s) => s,
        Err(r) => return *r,
    };
    let combo = body.into_combo(id.clone());
    if let Some(r) = unknown_target(&state, &combo) {
        return r;
    }
    match svc.update(&id, combo) {
        Ok(c) => Json(ComboSummary::from(&c)).into_response(),
        Err(e) => combo_err(e, &id),
    }
}

pub async fn delete_combo(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let svc = match service(&state, &headers) {
        Ok(s) => s,
        Err(r) => return *r,
    };
    match svc.delete(&id) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => combo_err(e, &id),
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
    use vkdg_connections::{AuthKind, ConnectionCatalog, ConnectionConfig, ProviderKind};

    fn state(dir: &std::path::Path) -> AdminState {
        let (_tx, rx) = watch::channel(Arc::new(ConfigSnapshot::default_empty()));
        let catalog = Arc::new(ConnectionCatalog::new(vec![ConnectionConfig {
            id: ConnectionId("c1".into()),
            provider: ProviderKind::Custom {
                base_url: "http://127.0.0.1:1".into(),
            },
            auth: AuthKind::ApiKey {
                env_var: "UNUSED".into(),
            },
            models: vec!["m-*".into()],
            max_concurrent: 1,
            weight: 1,
            tags: vec![],
            endpoint: None,
            capabilities: vkdg_core::CapabilitySet::default(),
        }]));
        let svc = vkdg_combos::ComboService::open(
            vkdg_combos::ComboStore::new(dir.join("combos.json")),
            Arc::new(vkdg_combos::ComboResolver::new(vec![])),
            None,
        )
        .unwrap();
        AdminState {
            sessions: SessionStore::new("tok".into()),
            config_rx: rx,
            started_at: Arc::new(Instant::now()),
            key_store: Arc::new(vkdg_governance::VirtualKeyStore::in_memory().unwrap()),
            request_log: RequestLog::new(),
            combos: Some(Arc::new(svc)),
            reload_plugins: None,
            connection_tester: None,
            catalog: Some(catalog),
            logins: None,
        }
    }

    fn authed(state: &AdminState) -> HeaderMap {
        let s = state.sessions.bootstrap_login().unwrap();
        let mut h = HeaderMap::new();
        h.insert(
            http::header::COOKIE,
            http::HeaderValue::from_str(&format!("vkdg_session={}", s.session_id)).unwrap(),
        );
        h
    }

    fn body(id: &str, targets: &[&str], model: Option<&str>) -> Json<ComboBody> {
        Json(ComboBody {
            id: id.into(),
            match_patterns: vec![],
            strategy: StrategyKind::RoundRobin,
            targets: targets.iter().map(|t| (*t).to_owned()).collect(),
            model: model.map(Into::into),
            max_cost_microdollars: None,
        })
    }

    #[tokio::test]
    async fn writes_need_a_session() {
        let dir = tempfile::tempdir().unwrap();
        let s = state(dir.path());
        let r = create_combo(
            State(s.clone()),
            HeaderMap::new(),
            body("a", &["c1"], Some("m-1")),
        )
        .await;
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
        let r = delete_combo(State(s.clone()), HeaderMap::new(), Path("a".into())).await;
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
        assert!(s.combos.unwrap().list().is_empty());
    }

    // A combo pointing at a connection that does not exist saves fine and then
    // fails every request it catches; one without a model sends its own id upstream.
    #[tokio::test]
    async fn unknown_target_and_missing_model_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let s = state(dir.path());
        let h = authed(&s);
        let r = create_combo(
            State(s.clone()),
            h.clone(),
            body("a", &["nope"], Some("m-1")),
        )
        .await;
        assert_eq!(r.status(), StatusCode::BAD_REQUEST);
        let r = create_combo(State(s.clone()), h.clone(), body("a", &["c1"], None)).await;
        assert_eq!(r.status(), StatusCode::BAD_REQUEST);
        assert!(s.combos.unwrap().list().is_empty());
    }

    #[tokio::test]
    async fn create_update_delete_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let s = state(dir.path());
        let h = authed(&s);
        let r = create_combo(State(s.clone()), h.clone(), body("a", &["c1"], Some("m-1"))).await;
        assert_eq!(r.status(), StatusCode::CREATED);
        let r = create_combo(State(s.clone()), h.clone(), body("a", &["c1"], Some("m-1"))).await;
        assert_eq!(r.status(), StatusCode::CONFLICT);
        let r = update_combo(
            State(s.clone()),
            h.clone(),
            Path("a".into()),
            body("", &["c1"], Some("m-2")),
        )
        .await;
        assert_eq!(r.status(), StatusCode::OK);
        let listed = s.combos.as_ref().unwrap().list();
        assert_eq!(listed[0].model.as_deref(), Some("m-2"));

        let r = delete_combo(State(s.clone()), h.clone(), Path("a".into())).await;
        assert_eq!(r.status(), StatusCode::NO_CONTENT);
        let r = delete_combo(State(s.clone()), h.clone(), Path("a".into())).await;
        assert_eq!(r.status(), StatusCode::NOT_FOUND);
        let r = update_combo(
            State(s),
            h,
            Path("a".into()),
            body("", &["c1"], Some("m-3")),
        )
        .await;
        assert_eq!(r.status(), StatusCode::NOT_FOUND);
    }
}
