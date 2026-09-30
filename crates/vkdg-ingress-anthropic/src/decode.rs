//! Decode Anthropic Messages API wire requests into internal `Operation` types.

use vkdg_core::VkdgError;
use vkdg_operations::{
    CapabilitySet, ContentBlock, ConversationRequest, ImageData, Message, MessageContent,
    Operation, Role, Tool,
};

use crate::wire::{AnthropicBlock, AnthropicContent, AnthropicToolResultContent};

/// Parse raw bytes from an Anthropic Messages request into `(model_name, Operation)`.
pub fn decode_request(body: &[u8]) -> Result<(String, Operation), VkdgError> {
    let req: crate::wire::AnthropicRequest =
        serde_json::from_slice(body).map_err(|e| VkdgError::ConfigInvalid {
            field: "body".to_string(),
            message: e.to_string(),
        })?;

    // omp and other clients encode reasoning effort as a suffix on the model
    // name: `claude-sonnet-4.6:max`, `claude-opus-5:high`, `...:off`.
    // Strip it and carry it as ThinkingRequest.effort so the provider adapter
    // can activate the right reasoning mode without changing the model id.
    const KNOWN_EFFORTS: &[&str] = &["off", "min", "low", "medium", "high", "xhigh", "max"];
    let (base_model, effort_suffix) = {
        let m = &req.model;
        if let Some(pos) = m.rfind(':') {
            let suffix = &m[pos + 1..];
            if KNOWN_EFFORTS.contains(&suffix) {
                (m[..pos].to_owned(), Some(suffix.to_owned()))
            } else {
                (m.clone(), None)
            }
        } else {
            (m.clone(), None)
        }
    };

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

    // Merge thinking from the wire field and from the model suffix.
    // Suffix wins for effort; wire field wins for budget_tokens.
    let thinking = match (
        req.thinking.filter(|t| t.kind != "disabled"),
        effort_suffix.as_deref(),
    ) {
        (_, Some("off")) => None,
        (Some(t), Some(e)) => Some(vkdg_operations::ThinkingRequest {
            budget_tokens: t.budget_tokens,
            effort: Some(e.to_owned()),
        }),
        (Some(t), None) => Some(vkdg_operations::ThinkingRequest {
            budget_tokens: t.budget_tokens,
            // `{type:"adaptive"}` without explicit effort → use "max" so the
            // provider can activate the highest available reasoning mode.
            effort: if t.budget_tokens.is_none() && t.kind == "adaptive" {
                Some("max".to_owned())
            } else {
                None
            },
        }),
        (None, Some(e)) => Some(vkdg_operations::ThinkingRequest {
            budget_tokens: None,
            effort: Some(e.to_owned()),
        }),
        (None, None) => None,
    };

    let operation = Operation::Conversation(ConversationRequest {
        model: base_model.clone(),
        messages,
        tools,
        max_tokens: req.max_tokens,
        temperature: req.temperature,
        stream: req.stream.unwrap_or(false),
        system: req.system.map(system_text),
        required_capabilities: CapabilitySet::default(),
        thinking,
    });

    Ok((base_model, operation))
}
/// Flatten `system` to text. Block arrays keep their order, joined by a blank
/// line; non-text blocks carry no system text and are skipped.
fn system_text(system: AnthropicContent) -> String {
    match system {
        AnthropicContent::Text(s) => s,
        AnthropicContent::Blocks(blocks) => blocks
            .into_iter()
            .filter(|b| b.type_ == "text")
            .filter_map(|b| b.text)
            .collect::<Vec<_>>()
            .join("\n\n"),
    }
}

/// Convert an Anthropic wire block into a [`ContentBlock`].
/// Returns `None` for unrecognised block types.
fn anthropic_block_to_content(b: AnthropicBlock) -> Option<ContentBlock> {
    match b.type_.as_str() {
        "text" => b.text.map(|t| ContentBlock::Text { text: t }),
        "image" => {
            let source = b.source?;
            let data = match source.type_.as_str() {
                "base64" => ImageData::Base64 {
                    data: source.data.unwrap_or_default(),
                },
                _ => ImageData::Url {
                    url: source.url.unwrap_or_default(),
                },
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
            Some(ContentBlock::ToolResult {
                tool_use_id,
                content,
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use http::{Method, Request, StatusCode};
    use vkdg_http::{AppState, ServerConfig};

    use crate::handle_messages;
    use axum::extract::State;
    use axum::response::Response;

    fn valid_body() -> &'static str {
        r#"{"model":"claude-3-5-sonnet-20241022","max_tokens":100,"messages":[{"role":"user","content":"hello"}]}"#
    }

    async fn call_handler(state: AppState, body: impl Into<axum::body::Body>) -> Response {
        let mut req = Request::builder()
            .method(Method::POST)
            .uri("/v1/messages")
            .header("content-type", "application/json")
            .body(body.into())
            .unwrap();
        // What `vkdg_http::require_api_key` attaches for an authenticated caller.
        req.extensions_mut().insert(vkdg_http::ClientIdentity {
            key_id: "key-test".into(),
            tenant_id: "default".into(),
            client_ip: None,
            allowed_models: std::sync::Arc::from([]),
        });
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
        let body = valid_body().as_bytes();
        let (model, _op) = decode_request(body).unwrap();
        assert_eq!(model, "claude-3-5-sonnet-20241022");
    }

    // Plausible wrong impl: `system` typed as a string rejects the block-array
    // form Claude Code sends (400), or joins blocks out of order / drops one.
    #[test]
    fn system_block_array_is_accepted_in_order() {
        let body = br#"{"model":"m","max_tokens":8,
            "system":[{"type":"text","text":"You are Claude Code."},
                      {"type":"text","text":"Project rules.","cache_control":{"type":"ephemeral"}}],
            "messages":[{"role":"user","content":"hi"}]}"#;
        let (_, op) = decode_request(body).expect("block-array system must decode");
        let Operation::Conversation(req) = op else {
            panic!("expected conversation")
        };
        assert_eq!(
            req.system.as_deref(),
            Some("You are Claude Code.\n\nProject rules.")
        );
    }

    /// `valid_body()` padded with whitespace to exactly `len` bytes.
    fn padded_body(len: usize) -> Vec<u8> {
        let mut b = valid_body().as_bytes().to_vec();
        b.resize(len, b' ');
        b
    }

    // Plausible wrong impl: oversize body mapped to a generic 400
    // invalid_request_error, so clients can't tell "too big" from "malformed".
    #[tokio::test]
    async fn body_over_configured_limit_returns_413() {
        let state = AppState::new(ServerConfig {
            max_body_bytes: 256,
            ..ServerConfig::default()
        });
        let resp = call_handler(state, padded_body(257)).await;
        assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let body = to_bytes(resp.into_body(), 4096).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"]["type"], "request_too_large");
    }

    // Plausible wrong impl: limit still hardcoded at 4 MiB, ignoring
    // ServerConfig. A 5 MiB body under an 8 MiB limit must reach the pipeline
    // (501 here, since pipeline=None), not be rejected as too large.
    #[tokio::test]
    async fn body_under_raised_limit_is_accepted() {
        let state = AppState::new(ServerConfig {
            max_body_bytes: 8 * 1024 * 1024,
            ..ServerConfig::default()
        });
        let resp = call_handler(state, padded_body(5 * 1024 * 1024)).await;
        assert_eq!(resp.status(), StatusCode::NOT_IMPLEMENTED);
    }
}
