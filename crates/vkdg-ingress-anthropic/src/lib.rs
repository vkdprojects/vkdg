use axum::{
    extract::{Request, State},
    response::{IntoResponse, Response, Sse},
    response::sse::Event,
};
use bytes::Bytes;
use futures::Stream;
use http::{HeaderMap, HeaderValue, StatusCode};
use serde::{Deserialize, Serialize};
use std::convert::Infallible;

use vkdg_core::{ApiType, ClientId, RequestEnvelope, RequestId, TenantId, VkdgError};
use vkdg_core::pipeline::PipelineCtx;
use vkdg_http::AppState;
use vkdg_http::{extract_client_ip, pipeline::run_conversation_pipeline};
use vkdg_operations::{
    CapabilitySet, ContentBlock, ConversationEvent, ConversationRequest, ImageData, Message,
    MessageContent, Operation, Role, Tool,
};

// ── Anthropic wire types (deserialization only) ───────────────────────────────

#[derive(Debug, Deserialize)]
struct AnthropicRequest {
    model: String,
    messages: Vec<AnthropicMessage>,
    system: Option<String>,
    max_tokens: Option<u32>,
    temperature: Option<f32>,
    stream: Option<bool>,
    tools: Option<Vec<AnthropicTool>>,
}

#[derive(Debug, Deserialize)]
struct AnthropicMessage {
    role: String,
    content: AnthropicContent,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum AnthropicContent {
    Text(String),
    Blocks(Vec<AnthropicBlock>),
}

#[derive(Debug, Deserialize)]
struct AnthropicBlock {
    #[serde(rename = "type")]
    type_: String,
    // text block
    text: Option<String>,
    // tool_use block
    id: Option<String>,
    name: Option<String>,
    input: Option<serde_json::Value>,
    // tool_result block
    tool_use_id: Option<String>,
    content: Option<AnthropicToolResultContent>,
    // image block
    source: Option<AnthropicImageSource>,
}

/// The `content` field of a `tool_result` block may be a plain string or
/// an array of sub-blocks (images, text, etc.).
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum AnthropicToolResultContent {
    Text(String),
    Blocks(Vec<AnthropicBlock>),
}

#[derive(Debug, Deserialize)]
struct AnthropicImageSource {
    #[serde(rename = "type")]
    type_: String,          // "base64" | "url"
    media_type: Option<String>,
    data: Option<String>,   // base64 payload
    url: Option<String>,    // URL payload
}

#[derive(Debug, Deserialize)]
struct AnthropicTool {
    name: String,
    description: Option<String>,
    input_schema: serde_json::Value,
}

// ── Anthropic error wire types ────────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct AnthropicErrorBody {
    #[serde(rename = "type")]
    type_: String,
    error: AnthropicErrorDetail,
}

#[derive(Debug, Serialize)]
struct AnthropicErrorDetail {
    #[serde(rename = "type")]
    type_: String,
    message: String,
}

impl AnthropicErrorBody {
    fn new(error_type: &str, message: impl Into<String>) -> Self {
        Self {
            type_: "error".to_string(),
            error: AnthropicErrorDetail {
                type_: error_type.to_string(),
                message: message.into(),
            },
        }
    }
}

// ── Decode ────────────────────────────────────────────────────────────────────

/// Parse raw bytes from an Anthropic Messages request into `(model_name, Operation)`.
pub fn decode_request(body: Bytes) -> Result<(String, Operation), VkdgError> {
    let req: AnthropicRequest =
        serde_json::from_slice(&body).map_err(|e| VkdgError::ConfigInvalid {
            field: "body".to_string(),
            message: e.to_string(),
        })?;

    let messages: Vec<Message> = req
        .messages
        .into_iter()
        .map(|m| {
            let role = match m.role.as_str() {
                "user" => Role::User,
                "assistant" => Role::Assistant,
                "system" => Role::System,
                _ => Role::User,
            };
            let content = match m.content {
                AnthropicContent::Text(s) => MessageContent::Text(s),
                AnthropicContent::Blocks(blocks) => {
                    let content_blocks: Vec<ContentBlock> = blocks
                        .into_iter()
                        .filter_map(anthropic_block_to_content)
                        .collect();
                    if content_blocks.is_empty() {
                        MessageContent::Text(String::new())
                    } else {
                        MessageContent::Blocks(content_blocks)
                    }
                }
            };
            Message { role, content }
        })
        .collect();

    let tools: Vec<Tool> = req
        .tools
        .unwrap_or_default()
        .into_iter()
        .map(|t| Tool {
            name: t.name,
            description: t.description,
            input_schema: t.input_schema,
        })
        .collect();

    let operation = Operation::Conversation(ConversationRequest {
        messages,
        tools,
        max_tokens: req.max_tokens,
        temperature: req.temperature,
        stream: req.stream.unwrap_or(false),
        system: req.system,
        required_capabilities: CapabilitySet::default(),
    });

    Ok((req.model, operation))
}

// ── Block conversion ──────────────────────────────────────────────────────────

/// Convert an Anthropic wire block into a [`ContentBlock`].
/// Returns `None` for unrecognised block types.
fn anthropic_block_to_content(b: AnthropicBlock) -> Option<ContentBlock> {
    match b.type_.as_str() {
        "text" => b.text.map(|t| ContentBlock::Text { text: t }),
        "image" => {
            let source = b.source?;
            let data = match source.type_.as_str() {
                "base64" => ImageData::Base64 { data: source.data.unwrap_or_default() },
                _ => ImageData::Url { url: source.url.unwrap_or_default() },
            };
            let media_type = source.media_type.unwrap_or_else(|| "image/jpeg".into());
            Some(ContentBlock::Image { media_type, data })
        }
        "tool_use" => {
            let id = b.id.unwrap_or_default();
            let name = b.name.unwrap_or_default();
            let input = b.input.unwrap_or(serde_json::Value::Null);
            Some(ContentBlock::ToolUse { id, name, input })
        }
        "tool_result" => {
            let tool_use_id = b.tool_use_id.unwrap_or_default();
            // Content can be a plain string or an array of sub-blocks (including images).
            // When the content is structured, serialize it as a JSON string so it can be
            // passed through to the downstream provider.
            let content = match b.content {
                Some(AnthropicToolResultContent::Text(s)) => s,
                Some(AnthropicToolResultContent::Blocks(sub_blocks)) => {
                    let sub: Vec<ContentBlock> = sub_blocks
                        .into_iter()
                        .filter_map(anthropic_block_to_content)
                        .collect();
                    serde_json::to_string(&sub).unwrap_or_default()
                }
                None => String::new(),
            };
            Some(ContentBlock::ToolResult { tool_use_id, content })
        }
        _ => None,
    }
}


// ── SSE encoding ──────────────────────────────────────────────────────────────

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

// ── Error mapping ─────────────────────────────────────────────────────────────

/// Map a `VkdgError` to the appropriate HTTP status code + Anthropic error JSON body.
pub fn vkdg_error_to_anthropic_response(err: VkdgError) -> Response {
    let (status, error_type) = match &err {
        VkdgError::Unauthenticated => (StatusCode::UNAUTHORIZED, "authentication_error"),
        VkdgError::Unauthorized => (StatusCode::FORBIDDEN, "permission_error"),
        VkdgError::AdmissionRejected { .. } => (StatusCode::TOO_MANY_REQUESTS, "overloaded_error"),
        VkdgError::CapabilityUnsupported { .. } => (StatusCode::BAD_REQUEST, "invalid_request_error"),
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

// ── Handler ───────────────────────────────────────────────────────────────────

/// Axum handler for `POST /v1/messages`.
///
/// Decodes the Anthropic wire request, builds a pipeline context, and
/// dispatches to `run_conversation_pipeline`.  Returns 501 when the pipeline
/// is not configured on `AppState`.
pub async fn handle_messages(
    State(state): State<AppState>,
    req: Request,
) -> Response {
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
    envelope.mode_pack_override = headers
        .get("x-vkdg-mode")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    envelope.compression_override = headers
        .get("x-vkdg-compression")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    envelope.cache_bypass = headers
        .get("x-vkdg-cache")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.eq_ignore_ascii_case("none"))
        .unwrap_or(false);
    envelope.include_think_tags = headers
        .get("x-vkdg-think-tags")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.eq_ignore_ascii_case("include"))
        .unwrap_or(false);
    envelope.client_ip = extract_client_ip(headers);

    // 4. Dispatch to pipeline or return 501 Not Implemented.
    // VkdgError::Internal would map to 500 — wrong semantics.
    // Pipeline absent means the server is not yet configured, which is 501.
    match state.pipeline {
        Some(pipeline) => {
            let ctx = PipelineCtx::new(envelope);
            run_conversation_pipeline(pipeline, ctx, operation).await
        }
        None => {
            use axum::response::IntoResponse;
            use http::StatusCode;
            (
                StatusCode::NOT_IMPLEMENTED,
                [(http::header::CONTENT_TYPE, "application/json")],
                r#"{"type":"error","error":{"type":"not_implemented","message":"pipeline not configured"}}"#,
            )
                .into_response()
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use bytes::Bytes;
    use http::{Method, Request, StatusCode};
    use vkdg_http::{AppState, ServerConfig};

    fn valid_body() -> &'static str {
        r#"{"model":"claude-3-5-sonnet-20241022","max_tokens":100,"messages":[{"role":"user","content":"hello"}]}"#
    }

    async fn call_handler(state: AppState, body: &'static str) -> Response {
        let req = Request::builder()
            .method(Method::POST)
            .uri("/v1/messages")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body))
            .unwrap();
        handle_messages(State(state), req).await
    }

    // Plausible wrong impl: panics on None pipeline, or maps None →
    // VkdgError::Internal → 500. Correct: 501 Not Implemented.
    #[tokio::test]
    async fn pipeline_none_returns_501_not_panic() {
        let state = AppState::new(ServerConfig::default()); // pipeline=None
        let resp = call_handler(state, valid_body()).await;
        assert_eq!(
            resp.status(),
            StatusCode::NOT_IMPLEMENTED,
            "pipeline=None must return 501 Not Implemented, not 500 or panic"
        );
        let body = to_bytes(resp.into_body(), 4096).await.unwrap();
        let text = String::from_utf8_lossy(&body);
        assert!(
            text.contains("pipeline not configured"),
            "body must mention 'pipeline not configured', got: {text}"
        );
    }

    // Plausible wrong impl: returns 200 or silently ignores bad JSON
    // instead of returning a 400 with ConfigInvalid.
    #[tokio::test]
    async fn invalid_json_body_returns_400() {
        let state = AppState::new(ServerConfig::default());
        let resp = call_handler(state, "not json at all").await;
        assert_eq!(
            resp.status(),
            StatusCode::BAD_REQUEST,
            "invalid JSON must produce 400 ConfigInvalid, not 200 or 500"
        );
    }

    // Plausible wrong impl: decode_request returns Ok for empty messages,
    // losing required fields and producing a bad Operation.
    #[tokio::test]
    async fn decode_round_trips_model_name() {
        let body = Bytes::from(valid_body());
        let (model, _op) = decode_request(body).unwrap();
        assert_eq!(model, "claude-3-5-sonnet-20241022");
    }
}
