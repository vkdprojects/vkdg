//! VKDG provider plugin: Fireworks AI (OpenAI-compatible endpoint).

use bytes::Bytes;
use http::HeaderMap;
use serde_json::{json, Map, Value};
use vkdg_connections::{ConnectionConfig, ProviderKind};
use vkdg_operations::{ContentBlock, ConversationRequest, MessageContent, Operation, Role};
use vkdg_provider_sdk::{PreparedRequest, ProviderAdapter, ProviderError};

pub struct FireworksProvider;

impl ProviderAdapter for FireworksProvider {
    fn id(&self) -> &str {
        "fireworks"
    }

    fn display_name(&self) -> &str {
        "Fireworks AI"
    }

    fn prepare(
        &self,
        operation: &Operation,
        config: &ConnectionConfig,
        token: &str,
    ) -> Result<PreparedRequest, ProviderError> {
        match operation {
            Operation::Conversation(req) => {
                let body = build_body(req, config);
                let url = format!("{}/v1/chat/completions", base_url(config));
                let headers = build_auth_headers(token);
                Ok(PreparedRequest {
                    url,
                    headers,
                    body,
                    is_streaming: req.stream,
                })
            }
            _ => Err(ProviderError::UnsupportedOperation),
        }
    }
}

fn base_url(config: &ConnectionConfig) -> String {
    match &config.provider {
        ProviderKind::Custom { base_url } => base_url.clone(),
        _ => "https://api.fireworks.ai/inference".into(),
    }
}

fn build_auth_headers(token: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        http::header::AUTHORIZATION,
        format!("Bearer {token}")
            .parse()
            .unwrap_or_else(|_| http::HeaderValue::from_static("invalid")),
    );
    headers.insert(
        http::header::CONTENT_TYPE,
        http::HeaderValue::from_static("application/json"),
    );
    headers
}

fn build_body(req: &ConversationRequest, config: &ConnectionConfig) -> Bytes {
    let model = config
        .models
        .first()
        .cloned()
        .unwrap_or_else(|| "accounts/fireworks/models/llama-v3p3-70b-instruct".into());

    let mut messages: Vec<Value> = Vec::new();
    if let Some(sys) = &req.system {
        messages.push(json!({ "role": "system", "content": sys }));
    }

    for m in &req.messages {
        let msg = match m.role {
            Role::System => {
                let content = message_content_to_value(&m.content);
                json!({ "role": "system", "content": content })
            }
            Role::User => {
                let content = message_content_to_value(&m.content);
                json!({ "role": "user", "content": content })
            }
            Role::Assistant => {
                if let MessageContent::Blocks(blocks) = &m.content {
                    let tool_calls: Vec<Value> = blocks
                        .iter()
                        .filter_map(|b| match b {
                            ContentBlock::ToolUse { id, name, input } => {
                                let arguments =
                                    serde_json::to_string(input).unwrap_or_else(|_| "{}".into());
                                Some(json!({
                                    "id": id,
                                    "type": "function",
                                    "function": { "name": name, "arguments": arguments }
                                }))
                            }
                            _ => None,
                        })
                        .collect();
                    if !tool_calls.is_empty() {
                        json!({ "role": "assistant", "content": Value::Null, "tool_calls": tool_calls })
                    } else {
                        let content = message_content_to_value(&m.content);
                        json!({ "role": "assistant", "content": content })
                    }
                } else {
                    let content = message_content_to_value(&m.content);
                    json!({ "role": "assistant", "content": content })
                }
            }
            Role::Tool => {
                let (tool_call_id, content_str) = extract_tool_result(&m.content);
                json!({ "role": "tool", "tool_call_id": tool_call_id, "content": content_str })
            }
        };
        messages.push(msg);
    }

    let mut body = Map::new();
    body.insert("model".into(), Value::String(model));
    body.insert("messages".into(), Value::Array(messages));
    if let Some(max) = req.max_tokens {
        body.insert("max_tokens".into(), json!(max));
    }
    if let Some(temp) = req.temperature {
        body.insert("temperature".into(), json!(temp));
    }
    if req.stream {
        body.insert("stream".into(), Value::Bool(true));
        body.insert("stream_options".into(), json!({ "include_usage": true }));
    }
    if !req.tools.is_empty() {
        let tools: Vec<Value> = req
            .tools
            .iter()
            .map(|t| {
                let mut func = Map::new();
                func.insert("name".into(), Value::String(t.name.clone()));
                if let Some(desc) = &t.description {
                    func.insert("description".into(), Value::String(desc.clone()));
                }
                func.insert("parameters".into(), t.input_schema.clone());
                json!({ "type": "function", "function": func })
            })
            .collect();
        body.insert("tools".into(), Value::Array(tools));
    }

    Bytes::from(serde_json::to_vec(&Value::Object(body)).unwrap_or_default())
}

fn message_content_to_value(content: &MessageContent) -> Value {
    match content {
        MessageContent::Text(t) => Value::String(t.clone()),
        MessageContent::Blocks(blocks) => {
            let parts: Vec<Value> = blocks
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::Text { text } => Some(json!({ "type": "text", "text": text })),
                    _ => None,
                })
                .collect();
            if parts.is_empty() {
                Value::Null
            } else {
                Value::Array(parts)
            }
        }
    }
}

fn extract_tool_result(content: &MessageContent) -> (String, String) {
    if let MessageContent::Blocks(blocks) = content {
        for b in blocks {
            if let ContentBlock::ToolResult {
                tool_use_id,
                content: result_content,
            } = b
            {
                return (tool_use_id.clone(), result_content.clone());
            }
        }
    }
    let text = match content {
        MessageContent::Text(t) => t.clone(),
        _ => String::new(),
    };
    (String::new(), text)
}
