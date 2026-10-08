//! Decode Anthropic Messages API wire requests into internal `Operation` types.

use vkdg_core::VkdgError;
use vkdg_operations::{
    validated_stop_sequences, validated_top_p, CacheControl, CapabilitySet, ContentBlock,
    ConversationRequest, ImageData, Message, MessageContent, Operation, Role, ServerTool,
    SystemBlock, Tool, ToolChoice, ToolResultImage,
};

use crate::wire::{
    AnthropicBlock, AnthropicCacheControl, AnthropicContent, AnthropicImageSource, AnthropicTool,
    AnthropicToolResultContent,
};

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

    let (tools, server_tools) = split_tools(req.tools.unwrap_or_default())?;

    let (tool_choice, disable_parallel_tool_use) = match req.tool_choice {
        Some(raw) => {
            let (choice, disable_parallel) = decode_tool_choice(raw)?;
            (Some(choice), disable_parallel)
        }
        None => (None, false),
    };
    let tool_choice = ToolChoice::settle(tool_choice, &tools, &server_tools)?;
    // Without tools there is no tool call to serialise, and providers reject
    // parallel-call settings that come without tools.
    let disable_parallel_tool_use =
        disable_parallel_tool_use && !(tools.is_empty() && server_tools.is_empty());
    let stop_sequences =
        validated_stop_sequences("stop_sequences", req.stop_sequences.unwrap_or_default())?;
    let top_p = validated_top_p(req.top_p)?;

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

    let (system, system_blocks) = match req.system.map(decode_system) {
        Some((text, blocks)) => (Some(text), blocks),
        None => (None, Vec::new()),
    };
    let operation = Operation::Conversation(ConversationRequest {
        model: base_model.clone(),
        messages,
        tools,
        server_tools,
        max_tokens: req.max_tokens,
        temperature: req.temperature,
        stream: req.stream.unwrap_or(false),
        system,
        system_blocks,
        required_capabilities: CapabilitySet::default(),
        thinking,
        tool_choice,
        stop_sequences,
        top_p,
        disable_parallel_tool_use,
        ..Default::default()
    });

    Ok((base_model, operation))
}

/// Splits the wire `tools` into custom tools (they have an `input_schema`) and
/// provider-run server tools (a `type` such as `web_search_20250305`, no schema),
/// which are kept exactly as written.
fn split_tools(raw: Vec<serde_json::Value>) -> Result<(Vec<Tool>, Vec<ServerTool>), VkdgError> {
    let invalid = |index: usize, message: String| VkdgError::ConfigInvalid {
        field: format!("tools[{index}]"),
        message,
    };
    let mut tools = Vec::with_capacity(raw.len());
    let mut server_tools = Vec::new();
    for (index, value) in raw.into_iter().enumerate() {
        if value.get("input_schema").is_some() {
            let tool: AnthropicTool =
                serde_json::from_value(value).map_err(|e| invalid(index, e.to_string()))?;
            tools.push(Tool {
                name: tool.name,
                description: tool.description,
                input_schema: tool.input_schema,
                cache_control: cache_control(tool.cache_control),
            });
            continue;
        }
        let kind = value.get("type").and_then(serde_json::Value::as_str);
        let name = value.get("name").and_then(serde_json::Value::as_str);
        match (kind, name) {
            (Some(kind), Some(name)) if kind != "custom" => server_tools.push(ServerTool {
                name: name.to_owned(),
                declaration: value,
            }),
            _ => {
                return Err(invalid(
                    index,
                    "a tool needs an `input_schema`, or a server tool `type` and `name`".into(),
                ))
            }
        }
    }
    Ok((tools, server_tools))
}
/// Decode the wire `tool_choice` object into the choice and the client's
/// `disable_parallel_tool_use` flag.
///
/// Anthropic defines the flag for `auto`, `any` and `tool`; for `none` no tool
/// is called, so a well-formed flag there carries no meaning and is dropped.
/// An absent or `null` flag is `false`.
fn decode_tool_choice(raw: serde_json::Value) -> Result<(ToolChoice, bool), VkdgError> {
    let invalid = |message: &str| VkdgError::ConfigInvalid {
        field: "tool_choice".to_owned(),
        message: message.to_owned(),
    };
    let serde_json::Value::Object(object) = raw else {
        return Err(invalid("must be an object with a `type`"));
    };
    let disable_parallel = match object.get("disable_parallel_tool_use") {
        None | Some(serde_json::Value::Null) => false,
        Some(serde_json::Value::Bool(flag)) => *flag,
        Some(_) => return Err(invalid("`disable_parallel_tool_use` must be a boolean")),
    };
    let kind = object
        .get("type")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| invalid("`type` must be a string"))?;
    match kind {
        "auto" => Ok((ToolChoice::Auto, disable_parallel)),
        "any" => Ok((ToolChoice::Required, disable_parallel)),
        "none" => Ok((ToolChoice::Disabled, false)),
        "tool" => match object.get("name").and_then(serde_json::Value::as_str) {
            Some(name) if !name.is_empty() => {
                Ok((ToolChoice::Named(name.to_owned()), disable_parallel))
            }
            _ => Err(invalid("`type: tool` needs a non-empty string `name`")),
        },
        other => Err(invalid(&format!(
            "unknown type `{other}`; expected auto, any, none or tool"
        ))),
    }
}

/// Flatten `system` to text and, for a block array carrying prompt-cache
/// markers, keep the text blocks with their markers. Block arrays keep their
/// order, joined by a blank line; non-text blocks carry no system text and are
/// skipped. Without a marker the blocks are not kept: the joined text says it all.
fn decode_system(system: AnthropicContent) -> (String, Vec<SystemBlock>) {
    match system {
        AnthropicContent::Text(s) => (s, Vec::new()),
        AnthropicContent::Blocks(blocks) => {
            let blocks: Vec<SystemBlock> = blocks
                .into_iter()
                .filter(|b| b.type_ == "text")
                .filter_map(|b| {
                    let cache_control = cache_control(b.cache_control);
                    b.text.map(|text| SystemBlock {
                        text,
                        cache_control,
                    })
                })
                .collect();
            let text = blocks
                .iter()
                .map(|b| b.text.as_str())
                .collect::<Vec<_>>()
                .join("\n\n");
            let marked = blocks.iter().any(|b| b.cache_control.is_some());
            (text, if marked { blocks } else { Vec::new() })
        }
    }
}

/// The neutral marker for a wire `cache_control`. `ephemeral` is the only type
/// Anthropic defines; anything else is not a breakpoint and is ignored.
fn cache_control(raw: Option<AnthropicCacheControl>) -> Option<CacheControl> {
    let raw = raw?;
    match raw.type_.as_deref() {
        None | Some("ephemeral") => Some(CacheControl { ttl: raw.ttl }),
        Some(_) => None,
    }
}

/// The media type and data of an image `source` (`base64` or `url`).
fn image_source(source: AnthropicImageSource) -> (String, ImageData) {
    let data = match source.type_.as_str() {
        "base64" => ImageData::Base64 {
            data: source.data.unwrap_or_default(),
        },
        _ => ImageData::Url {
            url: source.url.unwrap_or_default(),
        },
    };
    let media_type = source.media_type.unwrap_or_else(|| "image/jpeg".into());
    (media_type, data)
}

/// Convert an Anthropic wire block into a [`ContentBlock`].
/// Returns `None` for unrecognised block types.
fn anthropic_block_to_content(b: AnthropicBlock) -> Option<ContentBlock> {
    match b.type_.as_str() {
        "text" => {
            let cache_control = cache_control(b.cache_control);
            b.text.map(|text| ContentBlock::Text {
                text,
                cache_control,
            })
        }
        "image" => {
            let (media_type, data) = image_source(b.source?);
            Some(ContentBlock::Image {
                media_type,
                data,
                cache_control: cache_control(b.cache_control),
            })
        }
        "tool_use" => {
            let id = b.id.unwrap_or_default();
            let name = b.name.unwrap_or_default();
            let input = b
                .input
                .unwrap_or_else(|| serde_json::Value::Object(serde_json::Map::new()));
            Some(ContentBlock::ToolUse {
                id,
                name,
                input,
                cache_control: cache_control(b.cache_control),
            })
        }
        "tool_result" => {
            let tool_use_id = b.tool_use_id.unwrap_or_default();
            let (content, images) = match b.content {
                Some(AnthropicToolResultContent::Text(s)) => (s, Vec::new()),
                Some(AnthropicToolResultContent::Blocks(sub_blocks)) => {
                    // Text blocks join into the result text; image blocks stay images,
                    // in order. Other block types (documents, search results) have no
                    // neutral form yet and are skipped.
                    let mut texts: Vec<String> = Vec::new();
                    let mut images: Vec<ToolResultImage> = Vec::new();
                    for sub in sub_blocks {
                        match sub.type_.as_str() {
                            "text" => texts.extend(sub.text),
                            "image" => images.extend(sub.source.map(|s| {
                                let (media_type, data) = image_source(s);
                                ToolResultImage { media_type, data }
                            })),
                            _ => {}
                        }
                    }
                    (texts.join("\n"), images)
                }
                None => (String::new(), Vec::new()),
            };
            Some(ContentBlock::ToolResult {
                tool_use_id,
                content,
                images,
                is_error: b.is_error.unwrap_or(false),
                cache_control: cache_control(b.cache_control),
            })
        }
        "thinking" => Some(ContentBlock::Thinking {
            thinking: b.thinking.or(b.text).unwrap_or_default(),
            signature: b.signature,
        }),
        "redacted_thinking" => Some(ContentBlock::RedactedThinking {
            data: b
                .data
                .or(b.text)
                .unwrap_or_else(|| b.input.as_ref().map(|v| v.to_string()).unwrap_or_default()),
        }),
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

    // Plausible wrong impl: `_ => None` arm in anthropic_block_to_content silently
    // drops thinking blocks, producing an empty assistant message instead of
    // preserving the reasoning trace.
    #[test]
    fn thinking_block_in_history_is_preserved() {
        let body = br#"{
            "model": "m",
            "max_tokens": 8,
            "messages": [
                {"role": "user", "content": "hi"},
                {"role": "assistant", "content": [
                    {"type": "thinking", "thinking": "I need to...", "signature": "abc123"}
                ]}
            ]
        }"#;
        let (_, op) = decode_request(body).expect("must decode thinking block");
        let Operation::Conversation(req) = op else {
            panic!("expected conversation")
        };
        let msg = &req.messages[1];
        let MessageContent::Blocks(blocks) = &msg.content else {
            panic!("expected blocks, got {:?}", msg.content)
        };
        assert_eq!(blocks.len(), 1, "thinking block must not be dropped");
        match &blocks[0] {
            ContentBlock::Thinking {
                thinking,
                signature,
            } => {
                assert_eq!(thinking, "I need to...");
                assert_eq!(signature.as_deref(), Some("abc123"));
            }
            other => panic!("expected Thinking block, got {other:?}"),
        }
    }

    // Plausible wrong impl: `_ => None` silently drops redacted_thinking blocks,
    // losing the sealed reasoning trace from multi-turn conversation history.
    #[test]
    fn redacted_thinking_block_is_preserved() {
        let body = br#"{
            "model": "m",
            "max_tokens": 8,
            "messages": [
                {"role": "user", "content": "hi"},
                {"role": "assistant", "content": [
                    {"type": "redacted_thinking", "data": "<redacted>"}
                ]}
            ]
        }"#;
        let (_, op) = decode_request(body).expect("must decode redacted_thinking block");
        let Operation::Conversation(req) = op else {
            panic!("expected conversation")
        };
        let msg = &req.messages[1];
        let MessageContent::Blocks(blocks) = &msg.content else {
            panic!("expected blocks, got {:?}", msg.content)
        };
        assert_eq!(
            blocks.len(),
            1,
            "redacted_thinking block must not be dropped"
        );
        match &blocks[0] {
            ContentBlock::RedactedThinking { data } => {
                assert_eq!(data, "<redacted>");
            }
            other => panic!("expected RedactedThinking block, got {other:?}"),
        }
    }

    // Plausible wrong impl: tool_result is_error flag dropped during decode,
    // causing downstream adapters to treat errors as successful tool outputs.
    #[test]
    fn tool_result_is_error_preserved() {
        let body = br#"{
            "model": "m",
            "max_tokens": 8,
            "messages": [
                {"role": "user", "content": [
                    {"type": "tool_use", "id": "t1", "name": "fn", "input": {}}
                ]},
                {"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "t1", "content": "error text", "is_error": true}
                ]}
            ]
        }"#;
        let (_, op) = decode_request(body).expect("must decode is_error tool_result");
        let Operation::Conversation(req) = op else {
            panic!("expected conversation")
        };
        let msg = &req.messages[1];
        let MessageContent::Blocks(blocks) = &msg.content else {
            panic!("expected blocks")
        };
        match &blocks[0] {
            ContentBlock::ToolResult {
                tool_use_id,
                is_error,
                ..
            } => {
                assert_eq!(tool_use_id, "t1");
                assert!(*is_error, "is_error must be true");
            }
            other => panic!("expected ToolResult, got {other:?}"),
        }
    }

    // Plausible wrong impl: `{type:"adaptive"}` without budget_tokens left with
    // effort=None, making the provider skip reasoning entirely instead of using
    // the highest available mode.
    #[test]
    fn adaptive_thinking_maps_to_max_effort() {
        let body = br#"{
            "model": "m",
            "max_tokens": 8,
            "thinking": {"type": "adaptive"},
            "messages": [{"role": "user", "content": "hi"}]
        }"#;
        let (_, op) = decode_request(body).expect("must decode adaptive thinking");
        let Operation::Conversation(req) = op else {
            panic!("expected conversation")
        };
        let thinking = req.thinking.expect("thinking must be Some for adaptive");
        assert_eq!(
            thinking.effort.as_deref(),
            Some("max"),
            "adaptive without budget_tokens must map to effort=max"
        );
        assert!(
            thinking.budget_tokens.is_none(),
            "adaptive must not set budget_tokens"
        );
    }
}
