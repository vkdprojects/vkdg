//! Decode Anthropic Messages API wire requests into internal `Operation` types.

use bytes::Bytes;

use vkdg_core::VkdgError;
use vkdg_operations::{
    CapabilitySet, ContentBlock, ConversationRequest, ImageData, Message, MessageContent,
    Operation, Role, Tool,
};

use crate::wire::{
    AnthropicBlock, AnthropicContent, AnthropicToolResultContent,
};

/// Parse raw bytes from an Anthropic Messages request into `(model_name, Operation)`.
pub fn decode_request(body: Bytes) -> Result<(String, Operation), VkdgError> {
    let req: crate::wire::AnthropicRequest =
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use bytes::Bytes;
    use http::{Method, Request, StatusCode};
    use vkdg_http::{AppState, ServerConfig};

    use crate::handle_messages;
    use axum::extract::State;
    use axum::response::Response;

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
