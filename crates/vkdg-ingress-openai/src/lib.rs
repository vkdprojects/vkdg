//! `OpenAI` Chat Completions and Images API ingress: decode wire requests, encode events back.

pub mod decode;
pub mod encode;

pub use decode::{decode_image_generate, decode_request, decode_responses_request};
pub use encode::{
    encode_event_to_oai_chunk, encode_event_to_responses_chunk, events_to_sse_stream,
};

use axum::{
    extract::{Request, State},
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use http::{HeaderMap, HeaderValue, StatusCode};
use serde::Serialize;

use vkdg_core::pipeline::PipelineCtx;
use vkdg_core::{ApiType, ClientId, RequestEnvelope, RequestId, TenantId, VkdgError};
use vkdg_http::AppState;
use vkdg_http::{extract_vkdg_overrides, pipeline::run_conversation_pipeline};

// ── OpenAI error wire types ───────────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct OaiErrorBody {
    error: OaiErrorDetail,
}

#[derive(Debug, Serialize)]
struct OaiErrorDetail {
    message: String,
    #[serde(rename = "type")]
    type_: String,
    code: Option<String>,
}

impl OaiErrorBody {
    fn new(type_: &str, message: impl Into<String>) -> Self {
        Self {
            error: OaiErrorDetail {
                message: message.into(),
                type_: type_.to_string(),
                code: None,
            },
        }
    }
}

// ── Error mapping ─────────────────────────────────────────────────────────────

/// Map a `VkdgError` to the appropriate HTTP status + `OpenAI` error JSON body.
pub fn vkdg_error_to_oai_response(err: &VkdgError) -> Response {
    let (status, error_type) = match err {
        VkdgError::Unauthenticated => (StatusCode::UNAUTHORIZED, "authentication_error"),
        VkdgError::Unauthorized => (StatusCode::FORBIDDEN, "permission_error"),
        VkdgError::AdmissionRejected { .. } => (StatusCode::TOO_MANY_REQUESTS, "overloaded_error"),
        VkdgError::CapabilityUnsupported { .. } => {
            (StatusCode::BAD_REQUEST, "invalid_request_error")
        }
        VkdgError::NoEligibleConnection | VkdgError::NoRouteMatched => {
            (StatusCode::SERVICE_UNAVAILABLE, "api_error")
        }
        VkdgError::UpstreamError { code, .. } => {
            let s = StatusCode::from_u16(*code).unwrap_or(StatusCode::BAD_GATEWAY);
            (s, "api_error")
        }
        VkdgError::PluginError { .. } => (StatusCode::INTERNAL_SERVER_ERROR, "api_error"),
        VkdgError::ConfigInvalid { .. } => (StatusCode::BAD_REQUEST, "invalid_request_error"),
        // OpenAI has no dedicated type; the 413 status carries the meaning.
        VkdgError::BodyTooLarge { .. } => (StatusCode::PAYLOAD_TOO_LARGE, "invalid_request_error"),
        // Stored login is dead; the client request is fine.
        VkdgError::CredentialRevoked { .. } => (StatusCode::UNAUTHORIZED, "authentication_error"),
        VkdgError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "api_error"),
        VkdgError::BudgetExceeded { .. } => (StatusCode::PAYMENT_REQUIRED, "budget_exceeded"),
    };

    let body = OaiErrorBody::new(error_type, err.to_string());
    let json = serde_json::to_vec(&body).unwrap_or_else(|_| b"{}".to_vec());

    let mut headers = HeaderMap::new();
    headers.insert(
        http::header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );

    (status, headers, json).into_response()
}

// ── Handler ───────────────────────────────────────────────────────────────────

/// Axum handler for `POST /v1/chat/completions`.
///
/// Decodes the `OpenAI` Chat Completions request, builds a pipeline context, and
/// dispatches to `run_conversation_pipeline`. Returns 501 when the pipeline is
/// not configured on `AppState`. The pipeline performs passthrough of upstream
/// bytes, so no re-encoding is needed for the OpenAI-compatible path.
pub async fn handle_chat_completions(State(state): State<AppState>, req: Request) -> Response {
    // 1. Split request to access headers and body separately.
    let (parts, body) = req.into_parts();
    let limit = state.front_door.server_config.max_body_bytes;
    let bytes: Bytes = match vkdg_http::read_body(body, limit).await {
        Ok(b) => b,
        Err(e) => return vkdg_error_to_oai_response(&e),
    };

    // 2. Decode OpenAI JSON → (model, Operation).
    let (model, operation) = match decode_request(&bytes) {
        Ok(v) => v,
        Err(e) => return vkdg_error_to_oai_response(&e),
    };

    // 3. Build request envelope with per-request override headers.
    let headers = &parts.headers;
    // Set by `vkdg_http::require_api_key`; a route mounted without it is a wiring
    // bug, so refuse rather than serve an unidentified caller.
    let Some(identity) = parts.extensions.get::<vkdg_http::ClientIdentity>() else {
        tracing::error!("data-plane route mounted without require_api_key");
        return vkdg_error_to_oai_response(&VkdgError::Unauthenticated);
    };
    if let Err(e) = identity.check_model(&model) {
        return vkdg_error_to_oai_response(&e);
    }
    let mut envelope = RequestEnvelope {
        request_id: RequestId::new(),
        client_id: ClientId(identity.key_id.clone()),
        tenant_id: TenantId(identity.tenant_id.clone()),
        session_key: None,
        api_type: ApiType::OpenAiChatCompletions,
        model_requested: model,
        deadline: None,
        mode_pack_override: None,
        compression_override: None,
        cache_bypass: false,
        include_think_tags: false,
        client_ip: identity.client_ip,
    };
    // Extract per-request override headers (all are optional).
    extract_vkdg_overrides(headers, &mut envelope);

    // 4. Dispatch to pipeline or return 501 Not Implemented.
    match state.pipeline {
        Some(pipeline) => {
            let ctx = PipelineCtx::new(envelope);
            run_conversation_pipeline(pipeline, ctx, operation).await
        }
        None => (
            StatusCode::NOT_IMPLEMENTED,
            [(http::header::CONTENT_TYPE, "application/json")],
            r#"{"error":{"type":"not_implemented","message":"pipeline not configured"}}"#,
        )
            .into_response(),
    }
}

/// Axum handler for `POST /v1/images/generations`.
///
/// Decodes the `OpenAI` Images API request, builds a pipeline context, and
/// dispatches to `run_conversation_pipeline`. Returns 501 when the pipeline is
/// not configured on `AppState`. The pipeline performs passthrough of the
/// upstream JSON response verbatim.
pub async fn handle_image_generations(State(state): State<AppState>, req: Request) -> Response {
    // 1. Split request to access headers and body separately.
    let (parts, body) = req.into_parts();
    let limit = state.front_door.server_config.max_body_bytes;
    let bytes: Bytes = match vkdg_http::read_body(body, limit).await {
        Ok(b) => b,
        Err(e) => return vkdg_error_to_oai_response(&e),
    };

    // 2. Decode OpenAI Images JSON → (model, Operation::ImageGenerate).
    let (model, operation) = match decode_image_generate(&bytes) {
        Ok(v) => v,
        Err(e) => return vkdg_error_to_oai_response(&e),
    };

    // 3. Build request envelope with per-request override headers.
    let headers = &parts.headers;
    // Set by `vkdg_http::require_api_key`; a route mounted without it is a wiring
    // bug, so refuse rather than serve an unidentified caller.
    let Some(identity) = parts.extensions.get::<vkdg_http::ClientIdentity>() else {
        tracing::error!("data-plane route mounted without require_api_key");
        return vkdg_error_to_oai_response(&VkdgError::Unauthenticated);
    };
    if let Err(e) = identity.check_model(&model) {
        return vkdg_error_to_oai_response(&e);
    }
    let mut envelope = RequestEnvelope {
        request_id: RequestId::new(),
        client_id: ClientId(identity.key_id.clone()),
        tenant_id: TenantId(identity.tenant_id.clone()),
        session_key: None,
        api_type: ApiType::OpenAiImages,
        model_requested: model,
        deadline: None,
        mode_pack_override: None,
        compression_override: None,
        cache_bypass: false,
        include_think_tags: false,
        client_ip: identity.client_ip,
    };
    // Extract per-request override headers (all are optional).
    extract_vkdg_overrides(headers, &mut envelope);

    // 4. Dispatch to pipeline or return 501 Not Implemented.
    match state.pipeline {
        Some(pipeline) => {
            let ctx = PipelineCtx::new(envelope);
            run_conversation_pipeline(pipeline, ctx, operation).await
        }
        None => (
            StatusCode::NOT_IMPLEMENTED,
            [(http::header::CONTENT_TYPE, "application/json")],
            r#"{"error":{"type":"not_implemented","message":"pipeline not configured"}}"#,
        )
            .into_response(),
    }
}

/// Axum handler for `POST /v1/responses`.
///
/// Decodes the `OpenAI` Responses API request, builds a pipeline context, and
/// dispatches to `run_conversation_pipeline`. Non-streaming returns 501 (rare
/// in practice; can be added later). Streaming uses `text/event-stream` with
/// Responses API SSE event format.
pub async fn handle_responses(State(state): State<AppState>, req: Request) -> Response {
    // 1. Split request to access headers and body separately.
    let (parts, body) = req.into_parts();
    let limit = state.front_door.server_config.max_body_bytes;
    let bytes: Bytes = match vkdg_http::read_body(body, limit).await {
        Ok(b) => b,
        Err(e) => return vkdg_error_to_oai_response(&e),
    };

    // 2. Decode Responses API JSON → (model, Operation).
    let (model, operation) = match decode_responses_request(&bytes) {
        Ok(v) => v,
        Err(e) => return vkdg_error_to_oai_response(&e),
    };

    // Non-streaming Responses API is rarely used; defer to avoid scope creep.
    let is_streaming = if let vkdg_operations::Operation::Conversation(conv_req) = &operation {
        conv_req.stream
    } else {
        false
    };
    if !is_streaming {
        return (
            StatusCode::NOT_IMPLEMENTED,
            [(http::header::CONTENT_TYPE, "application/json")],
            r#"{"error":{"type":"not_supported","message":"non-streaming Responses API not supported"}}"#,
        )
            .into_response();
    }

    // 3. Build request envelope with per-request override headers.
    let headers = &parts.headers;
    let Some(identity) = parts.extensions.get::<vkdg_http::ClientIdentity>() else {
        tracing::error!("data-plane route mounted without require_api_key");
        return vkdg_error_to_oai_response(&VkdgError::Unauthenticated);
    };
    if let Err(e) = identity.check_model(&model) {
        return vkdg_error_to_oai_response(&e);
    }
    let mut envelope = RequestEnvelope {
        request_id: RequestId::new(),
        client_id: ClientId(identity.key_id.clone()),
        tenant_id: TenantId(identity.tenant_id.clone()),
        session_key: None,
        api_type: ApiType::OpenAiResponses,
        model_requested: model,
        deadline: None,
        mode_pack_override: None,
        compression_override: None,
        cache_bypass: false,
        include_think_tags: false,
        client_ip: identity.client_ip,
    };
    extract_vkdg_overrides(headers, &mut envelope);

    // 4. Dispatch to pipeline or return 501 Not Implemented.
    let Some(pipeline) = state.pipeline else {
        return (
            StatusCode::NOT_IMPLEMENTED,
            [(http::header::CONTENT_TYPE, "application/json")],
            r#"{"error":{"type":"not_implemented","message":"pipeline not configured"}}"#,
        )
            .into_response();
    };

    let ctx = PipelineCtx::new(envelope);
    responses_sse_stream(pipeline, ctx, operation).await
}

/// Drive the pipeline and encode events into Responses API SSE format.
///
/// Currently uses the passthrough pipeline path; the upstream is expected to
/// return Responses API SSE directly. `encode_event_to_responses_chunk` is the
/// re-encoding path for when providers need translation.
async fn responses_sse_stream(
    pipeline: std::sync::Arc<vkdg_http::PipelineState>,
    ctx: PipelineCtx,
    operation: vkdg_operations::Operation,
) -> Response {
    run_conversation_pipeline(pipeline, ctx, operation).await
}
