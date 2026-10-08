//! Shared response helpers for admin handlers.
//!
//! Centralises the handful of `AdminErrorResponse(...).into_response()` patterns
//! that appear verbatim across multiple handlers.

use crate::{
    error::{AdminError, AdminErrorResponse},
    router::AdminState,
};
use axum::response::{IntoResponse, Response};
use http::StatusCode;
use std::sync::Arc;
use vkdg_config::GatewayStore;

/// 401 Unauthorized — no active session.
#[inline]
pub fn unauthorized() -> Response {
    AdminErrorResponse(StatusCode::UNAUTHORIZED, AdminError::unauthorized()).into_response()
}

/// 404 Not Found — `{id} not found`.
#[inline]
pub fn not_found(id: &str) -> Response {
    AdminErrorResponse(StatusCode::NOT_FOUND, AdminError::not_found(id)).into_response()
}

/// 422 Unprocessable Entity — validation failure.
#[inline]
pub fn validation(message: &str) -> Response {
    AdminErrorResponse(
        StatusCode::UNPROCESSABLE_ENTITY,
        AdminError::new("validation_error", message),
    )
    .into_response()
}

/// 500 Internal Server Error wrapping a store/IO error.
#[inline]
pub fn store_error(e: impl std::fmt::Display) -> Response {
    AdminErrorResponse(
        StatusCode::INTERNAL_SERVER_ERROR,
        AdminError::new("store_error", e.to_string()),
    )
    .into_response()
}

/// 500 Internal Server Error from a pre-built [`AdminError`].
#[inline]
pub fn internal(e: AdminError) -> Response {
    AdminErrorResponse(StatusCode::INTERNAL_SERVER_ERROR, e).into_response()
}

/// Extract `&Arc<GatewayStore>` from state, or return 503 Service Unavailable.
///
/// ```ignore
/// let store = require_store!(state)?;
/// ```
///
/// Typical usage inside an async handler that returns `Response`:
///
/// ```ignore
/// let Ok(store) = require_store(&state) else { return resp; };
/// ```
#[allow(clippy::result_large_err)]
pub fn require_store(state: &AdminState) -> Result<&Arc<GatewayStore>, Response> {
    state.gateway_store.as_ref().ok_or_else(|| {
        AdminErrorResponse(
            StatusCode::SERVICE_UNAVAILABLE,
            AdminError::new("no_store", "gateway store not available"),
        )
        .into_response()
    })
}
