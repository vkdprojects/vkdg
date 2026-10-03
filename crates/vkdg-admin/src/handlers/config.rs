//! `GET /admin/v1/config/export` — serialise the current gateway state to YAML.
//!
//! The output is a valid `config.yaml` that, if written to disk and used as
//! the gateway's config file, will reproduce the same connections, routes and
//! limits as are running right now. Useful for infrastructure-as-code, backups and migration.
use axum::{extract::State, response::IntoResponse};
use http::{HeaderMap, HeaderValue, StatusCode};

use crate::{error::AdminError, handlers::session::get_session, router::AdminState};

pub async fn export_config(
    State(state): State<AdminState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if get_session(&state, &headers).is_none() {
        return (
            StatusCode::UNAUTHORIZED,
            [(
                axum::http::header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            )],
            serde_json::to_string(&AdminError::unauthorized()).unwrap_or_default(),
        )
            .into_response();
    }

    let snapshot = state.config_rx.borrow().clone();

    // Build a GatewayConfig from the live snapshot.
    let gateway = (*snapshot.gateway).clone();

    match serde_yaml::to_string(&gateway) {
        Ok(yaml) => (
            StatusCode::OK,
            [(
                axum::http::header::CONTENT_TYPE,
                HeaderValue::from_static("application/yaml"),
            )],
            yaml,
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(
                axum::http::header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            )],
            format!("{{\"error\":\"serialization failed: {e}\"}}"),
        )
            .into_response(),
    }
}
