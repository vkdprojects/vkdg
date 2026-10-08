use crate::{
    error::{AdminError, AdminErrorResponse},
    handlers::{response as resp, session::get_session},
    router::AdminState,
};
use axum::{
    extract::{Path, State},
    response::{IntoResponse, Response},
    Json,
};
use chrono::Utc;
use http::{HeaderMap, StatusCode};
use serde::{Deserialize, Serialize};
use vkdg_config::CatalogModel;

// ── response shapes ──────────────────────────────────────────────────────────

#[derive(Serialize)]
struct CatalogResponse {
    provider: String,
    models: Vec<CatalogModel>,
    count: usize,
}

#[derive(Serialize)]
struct PutCatalogResponse {
    provider: String,
    count: usize,
    updated_at: String,
}

#[derive(Serialize)]
struct ImportUrlResponse {
    provider: String,
    count: usize,
    source_url: String,
}

// ── request shapes ───────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct PutCatalogBody {
    models: Vec<CatalogModel>,
}

#[derive(Deserialize)]
pub struct ImportUrlBody {
    url: String,
}

/// The two JSON shapes accepted from a remote registry URL.
#[derive(Deserialize)]
#[serde(untagged)]
enum RemoteCatalog {
    /// `{ "provider": "...", "models": [...] }`
    Object {
        #[allow(dead_code)]
        provider: Option<String>,
        models: Vec<CatalogModel>,
    },
    /// `[{ "id": "..." }, ...]`
    Array(Vec<CatalogModel>),
}

impl RemoteCatalog {
    fn into_models(self) -> Vec<CatalogModel> {
        match self {
            RemoteCatalog::Object { models, .. } => models,
            RemoteCatalog::Array(v) => v,
        }
    }
}

// ── handlers ─────────────────────────────────────────────────────────────────

/// GET /admin/v1/catalog/{provider}
pub async fn get_catalog(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Path(provider): Path<String>,
) -> Response {
    if get_session(&state, &headers).is_none() {
        return resp::unauthorized();
    }
    let Some(store) = state.gateway_store.as_ref() else {
        return AdminErrorResponse(
            StatusCode::NOT_FOUND,
            AdminError::new("no_store", "persistent store not available"),
        )
        .into_response();
    };

    let models = match store.catalog_list(&provider) {
        Ok(m) => m,
        Err(e) => return resp::store_error(e),
    };
    let count = models.len();
    Json(CatalogResponse {
        provider,
        models,
        count,
    })
    .into_response()
}

/// PUT /admin/v1/catalog/{provider}
pub async fn put_catalog(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Path(provider): Path<String>,
    Json(body): Json<PutCatalogBody>,
) -> Response {
    if get_session(&state, &headers).is_none() {
        return resp::unauthorized();
    }
    let Some(store) = state.gateway_store.as_ref() else {
        return AdminErrorResponse(
            StatusCode::NOT_FOUND,
            AdminError::new("no_store", "persistent store not available"),
        )
        .into_response();
    };

    let count = body.models.len();
    if let Err(e) = store.catalog_replace(&provider, &body.models) {
        return resp::store_error(e);
    }
    Json(PutCatalogResponse {
        provider,
        count,
        updated_at: Utc::now().to_rfc3339(),
    })
    .into_response()
}

/// POST /admin/v1/catalog/{provider}/import-url
pub async fn import_catalog_url(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Path(provider): Path<String>,
    Json(body): Json<ImportUrlBody>,
) -> Response {
    if get_session(&state, &headers).is_none() {
        return resp::unauthorized();
    }
    let Some(store) = state.gateway_store.as_ref() else {
        return AdminErrorResponse(
            StatusCode::NOT_FOUND,
            AdminError::new("no_store", "persistent store not available"),
        )
        .into_response();
    };

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap_or_default();
    let raw = match client.get(&body.url).send().await {
        Ok(resp) => match resp.text().await {
            Ok(t) => t,
            Err(e) => {
                return AdminErrorResponse(
                    StatusCode::BAD_GATEWAY,
                    AdminError::new("fetch_error", format!("reading response body: {e}")),
                )
                .into_response();
            }
        },
        Err(e) => {
            return AdminErrorResponse(
                StatusCode::BAD_GATEWAY,
                AdminError::new("fetch_error", format!("fetching {}: {e}", body.url)),
            )
            .into_response();
        }
    };

    let remote: RemoteCatalog = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            return AdminErrorResponse(
                StatusCode::UNPROCESSABLE_ENTITY,
                AdminError::new("parse_error", format!("invalid catalog JSON: {e}")),
            )
            .into_response();
        }
    };

    let models = remote.into_models();
    let count = models.len();

    if let Err(e) = store.catalog_replace(&provider, &models) {
        return resp::store_error(e);
    }
    Json(ImportUrlResponse {
        provider,
        count,
        source_url: body.url,
    })
    .into_response()
}
