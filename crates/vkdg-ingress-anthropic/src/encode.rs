//! Encode internal `ConversationEvent` types into Anthropic SSE wire format.

use axum::response::sse::Event;
use axum::response::{IntoResponse, Response, Sse};
use futures::Stream;
use http::{HeaderMap, HeaderValue, StatusCode};
use std::convert::Infallible;

use vkdg_core::VkdgError;
use vkdg_operations::ConversationEvent;

use crate::wire::AnthropicErrorBody;

/// Format a single `ConversationEvent` as an SSE data line.
/// `Completed` emits the event json **and** a trailing `[DONE]` frame.
pub fn encode_event(event: &ConversationEvent) -> String {
    let json = serde_json::to_string(event).unwrap_or_else(|_| "{}".to_string());
    match event {
        ConversationEvent::Completed { .. } => {
            format!("data: {json}\n\ndata: [DONE]\n\n")
        }
        _ => format!("data: {json}\n\n"),
    }
}

/// Wrap a `ConversationEvent` stream in an SSE `axum::response::Response`.
pub fn events_to_sse_stream(
    events: impl Stream<Item = ConversationEvent> + Send + 'static,
) -> Response {
    use futures::StreamExt;

    let sse_stream = events.map(|event| {
        let json = serde_json::to_string(&event).unwrap_or_else(|_| "{}".to_string());
        let data = match event {
            ConversationEvent::Completed { .. } => format!("{json}\n\ndata: [DONE]"),
            _ => json,
        };
        Ok::<Event, Infallible>(Event::default().data(data))
    });

    Sse::new(sse_stream)
        .keep_alive(axum::response::sse::KeepAlive::default())
        .into_response()
}

/// Map a `VkdgError` to the appropriate HTTP status code + Anthropic error JSON body.
pub fn vkdg_error_to_anthropic_response(err: VkdgError) -> Response {
    let (status, error_type) = match &err {
        VkdgError::Unauthenticated => (StatusCode::UNAUTHORIZED, "authentication_error"),
        VkdgError::Unauthorized => (StatusCode::FORBIDDEN, "permission_error"),
        VkdgError::AdmissionRejected { .. } => (StatusCode::TOO_MANY_REQUESTS, "overloaded_error"),
        VkdgError::CapabilityUnsupported { .. } => {
            (StatusCode::BAD_REQUEST, "invalid_request_error")
        }
        VkdgError::NoEligibleConnection => (StatusCode::SERVICE_UNAVAILABLE, "api_error"),
        VkdgError::UpstreamError { code, .. } => {
            let s = StatusCode::from_u16(*code).unwrap_or(StatusCode::BAD_GATEWAY);
            (s, "api_error")
        }
        VkdgError::PluginError { .. } => (StatusCode::INTERNAL_SERVER_ERROR, "api_error"),
        VkdgError::ConfigInvalid { .. } => (StatusCode::BAD_REQUEST, "invalid_request_error"),
        VkdgError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "api_error"),
    };

    let body = AnthropicErrorBody::new(error_type, err.to_string());
    let json = serde_json::to_vec(&body).unwrap_or_else(|_| b"{}".to_vec());

    let mut headers = HeaderMap::new();
    headers.insert(
        http::header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );

    (status, headers, json).into_response()
}
