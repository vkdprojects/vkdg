//! Anthropic provider adapter — implements [`ProviderAdapter`] for the Anthropic
//! Messages wire format.  The pipeline core has no knowledge of Anthropic
//! specifics; it only calls `prepare()` and uses the returned [`PreparedRequest`].

use bytes::Bytes;
use http::HeaderMap;
use serde_json::{json, Map, Value};
use vkdg_connections::{ConnectionConfig, ProviderKind};
use vkdg_core::VkdgError;
use vkdg_http::provider::{PreparedRequest, ProviderAdapter};
use vkdg_operations::{ConversationRequest, MessageContent, Operation, Role};

pub struct AnthropicAdapter;

impl ProviderAdapter for AnthropicAdapter {
    fn name(&self) -> &str {
        "anthropic"
    }

    fn prepare(
        &self,
        operation: &Operation,
        config: &ConnectionConfig,
        token: &str,
    ) -> Result<PreparedRequest, VkdgError> {
        let req = match operation {
            Operation::Conversation(r) => r,
            _ => {
                return Err(VkdgError::Internal(
                    "non-conversation ops not yet implemented".into(),
                ))
            }
        };

        let body = build_body(req, config);
        let url = format!("{}/v1/messages", base_url(config));

        let mut headers = HeaderMap::new();
        headers.insert(
            "x-api-key",
            token
                .parse()
                .unwrap_or_else(|_| http::HeaderValue::from_static("invalid")),
        );
        headers.insert(
            "anthropic-version",
            http::HeaderValue::from_static("2023-06-01"),
        );
        headers.insert(
            http::header::CONTENT_TYPE,
            http::HeaderValue::from_static("application/json"),
        );

        Ok(PreparedRequest {
            url,
            headers,
            body,
            is_streaming: req.stream,
        })
    }
}

fn base_url(config: &ConnectionConfig) -> String {
    match &config.provider {
        ProviderKind::Anthropic => "https://api.anthropic.com".into(),
        ProviderKind::OpenAI => "https://api.openai.com".into(),
        ProviderKind::Google => "https://generativelanguage.googleapis.com".into(),
        ProviderKind::Custom { base_url } => base_url.clone(),
    }
}

fn build_body(req: &ConversationRequest, config: &ConnectionConfig) -> Bytes {
    let model = config
        .models
        .first()
        .cloned()
        .unwrap_or_else(|| "claude-3-5-sonnet-20241022".into());

    let messages: Vec<Value> = req
        .messages
        .iter()
        .map(|m| {
            let role = match m.role {
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::System => "system",
                Role::Tool => "user",
            };
            let content = match &m.content {
                MessageContent::Text(t) => Value::String(t.clone()),
                MessageContent::Blocks(blocks) => Value::Array(
                    blocks
                        .iter()
                        .map(|b| serde_json::to_value(b).unwrap_or(Value::Null))
                        .collect(),
                ),
            };
            json!({ "role": role, "content": content })
        })
        .collect();

    let mut body = Map::new();
    body.insert("model".into(), Value::String(model));
    body.insert("messages".into(), Value::Array(messages));
    if let Some(sys) = &req.system {
        body.insert("system".into(), Value::String(sys.clone()));
    }
    if let Some(max) = req.max_tokens {
        body.insert("max_tokens".into(), json!(max));
    }
    if let Some(temp) = req.temperature {
        body.insert("temperature".into(), json!(temp));
    }
    if req.stream {
        body.insert("stream".into(), Value::Bool(true));
    }
    if !req.tools.is_empty() {
        let tools: Vec<Value> = req
            .tools
            .iter()
            .map(|t| {
                let mut tool = Map::new();
                tool.insert("name".into(), Value::String(t.name.clone()));
                if let Some(desc) = &t.description {
                    tool.insert("description".into(), Value::String(desc.clone()));
                }
                tool.insert("input_schema".into(), t.input_schema.clone());
                Value::Object(tool)
            })
            .collect();
        body.insert("tools".into(), Value::Array(tools));
    }

    Bytes::from(serde_json::to_vec(&Value::Object(body)).unwrap_or_default())
}
