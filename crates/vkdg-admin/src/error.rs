use axum::response::{IntoResponse, Response};
use axum::Json;
use http::StatusCode;
use serde::Serialize;
use uuid::Uuid;

#[derive(Debug, Serialize)]
pub struct AdminError {
    pub code: String,
    pub message: String,
    pub request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

impl AdminError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            request_id: Uuid::new_v4().to_string(),
            details: None,
        }
    }

    pub fn unauthorized() -> Self {
        Self::new("unauthorized", "no active session")
    }

    pub fn not_found(id: &str) -> Self {
        Self::new("not_found", format!("{id} not found"))
    }
}

pub struct AdminErrorResponse(pub StatusCode, pub AdminError);

impl IntoResponse for AdminErrorResponse {
    fn into_response(self) -> Response {
        (self.0, Json(self.1)).into_response()
    }
}
