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
