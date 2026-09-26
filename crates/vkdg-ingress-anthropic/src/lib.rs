mod decode;
mod encode;
mod wire;

pub use decode::decode_request;
pub use encode::{encode_event, events_to_sse_stream, vkdg_error_to_anthropic_response};

use axum::{
    extract::{Request, State},
    response::{IntoResponse, Response},
};
use http::StatusCode;

use vkdg_core::pipeline::PipelineCtx;
use vkdg_core::{ApiType, ClientId, RequestEnvelope, RequestId, TenantId, VkdgError};
use vkdg_http::AppState;
use vkdg_http::{extract_vkdg_overrides, pipeline::run_conversation_pipeline};

// ── Handler ───────────────────────────────────────────────────────────────────

/// Axum handler for `POST /v1/messages`.
///
/// Decodes the Anthropic wire request, builds a pipeline context, and
/// dispatches to `run_conversation_pipeline`.  Returns 501 when the pipeline
/// is not configured on `AppState`.
pub async fn handle_messages(State(state): State<AppState>, req: Request) -> Response {
    // Split request into parts so we can read headers before consuming the body.
    let (parts, body) = req.into_parts();

    // 1. Read body (4 MB hard limit — same as ServerConfig::default).
    let bytes = match axum::body::to_bytes(body, 4 * 1024 * 1024).await {
        Ok(b) => b,
        Err(_) => {
            return vkdg_error_to_anthropic_response(VkdgError::ConfigInvalid {
                field: "body".into(),
                message: "body too large or unreadable".into(),
            });
        }
    };

    // 2. Decode Anthropic JSON → (model, Operation).
    let (model, operation) = match decode_request(bytes) {
        Ok(v) => v,
        Err(e) => return vkdg_error_to_anthropic_response(e),
    };

    // 3. Build request envelope with per-request override headers.
    let headers = &parts.headers;
    let mut envelope = RequestEnvelope {
        request_id: RequestId::new(),
        client_id: ClientId("anonymous".into()),
        tenant_id: TenantId("default".into()),
        session_key: None,
        api_type: ApiType::AnthropicMessages,
        model_requested: model,
        deadline: None,
        mode_pack_override: None,
        compression_override: None,
        cache_bypass: false,
        include_think_tags: false,
        client_ip: None,
    };
    // Extract per-request override headers (all are optional).
    extract_vkdg_overrides(headers, &mut envelope);

    // 4. Dispatch to pipeline or return 501 Not Implemented.
    // VkdgError::Internal would map to 500 — wrong semantics.
    // Pipeline absent means the server is not yet configured, which is 501.
    match state.pipeline {
        Some(pipeline) => {
            let ctx = PipelineCtx::new(envelope);
            run_conversation_pipeline(pipeline, ctx, operation).await
        }
        None => {
            (
                StatusCode::NOT_IMPLEMENTED,
                [(http::header::CONTENT_TYPE, "application/json")],
                r#"{"type":"error","error":{"type":"not_implemented","message":"pipeline not configured"}}"#,
            )
                .into_response()
        }
    }
}
