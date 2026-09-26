//! Shared helpers for OpenAI Chat Completions wire format.
//!
//! All OpenAI-compatible providers (Groq, Together, Fireworks, DeepSeek,
//! Mistral, Gemini) delegate here instead of duplicating conversion logic.

use bytes::Bytes;
use http::HeaderMap;
use serde_json::{json, Map, Value};
use vkdg_connections::{ConnectionConfig, ProviderKind};
use vkdg_operations::{ContentBlock, ConversationRequest, MessageContent, Operation, Role};

use crate::{PreparedRequest, ProviderAdapter, ProviderError};

// ── Public helpers ────────────────────────────────────────────────────────────

/// Build a `Bearer <token>` + `Content-Type: application/json` header map.
pub fn bearer_headers(token: &str) -> HeaderMap {
    let mut h = HeaderMap::new();
    h.insert(
        http::header::AUTHORIZATION,
        format!("Bearer {token}")
            .parse()
            .unwrap_or_else(|_| http::HeaderValue::from_static("invalid")),
    );
    h.insert(
        http::header::CONTENT_TYPE,
        http::HeaderValue::from_static("application/json"),
    );
    h
}

/// Serialize a `ConversationRequest` to OpenAI Chat Completions JSON.
pub fn chat_completions_body(req: &ConversationRequest, model: &str) -> Bytes {
    let mut messages: Vec<Value> = Vec::new();

    if let Some(sys) = &req.system {
        messages.push(json!({ "role": "system", "content": sys }));
    }

    for m in &req.messages {
        messages.push(match m.role {
            Role::System => json!({ "role": "system", "content": content_to_value(&m.content) }),
            Role::User => json!({ "role": "user", "content": content_to_value(&m.content) }),
            Role::Assistant => assistant_message(&m.content),
            Role::Tool => {
                let (id, text) = tool_result(&m.content);
                json!({ "role": "tool", "tool_call_id": id, "content": text })
            }
        });
    }

    let mut body = Map::new();
    body.insert("model".into(), Value::String(model.into()));
    body.insert("messages".into(), Value::Array(messages));
    if let Some(v) = req.max_tokens {
        body.insert("max_tokens".into(), json!(v));
    }
    if let Some(v) = req.temperature {
        body.insert("temperature".into(), json!(v));
    }
    if req.stream {
        body.insert("stream".into(), Value::Bool(true));
        body.insert("stream_options".into(), json!({ "include_usage": true }));
    }
    if !req.tools.is_empty() {
        body.insert(
            "tools".into(),
            Value::Array(
                req.tools
                    .iter()
                    .map(|t| {
                        let mut f = Map::new();
                        f.insert("name".into(), Value::String(t.name.clone()));
                        if let Some(d) = &t.description {
                            f.insert("description".into(), Value::String(d.clone()));
                        }
                        f.insert("parameters".into(), t.input_schema.clone());
                        json!({ "type": "function", "function": f })
                    })
                    .collect(),
            ),
        );
    }

    Bytes::from(serde_json::to_vec(&Value::Object(body)).unwrap_or_default())
}

// ── Generic adapter ───────────────────────────────────────────────────────────

/// A zero-config `ProviderAdapter` for any OpenAI-compatible endpoint.
///
/// ```rust
/// use vkdg_provider_sdk::openai_compat::OpenAiCompatAdapter;
///
/// pub static PROVIDER: OpenAiCompatAdapter = OpenAiCompatAdapter::new(
///     "groq", "Groq", "https://api.groq.com/openai", "llama-3.3-70b-versatile",
/// );
/// ```
pub struct OpenAiCompatAdapter {
    id: &'static str,
    display_name: &'static str,
    default_base_url: &'static str,
    default_model: &'static str,
}

impl OpenAiCompatAdapter {
    pub const fn new(
        id: &'static str,
        display_name: &'static str,
        default_base_url: &'static str,
        default_model: &'static str,
    ) -> Self {
        Self {
            id,
            display_name,
            default_base_url,
            default_model,
        }
    }
}

impl ProviderAdapter for OpenAiCompatAdapter {
    fn id(&self) -> &str {
        self.id
    }
    fn display_name(&self) -> &str {
        self.display_name
    }

    fn prepare(
        &self,
        operation: &Operation,
        config: &ConnectionConfig,
        token: &str,
    ) -> Result<PreparedRequest, ProviderError> {
        let req = match operation {
            Operation::Conversation(r) => r,
            _ => return Err(ProviderError::UnsupportedOperation),
        };
        let base = match &config.provider {
            ProviderKind::Custom { base_url } => base_url.as_str(),
            _ => self.default_base_url,
        };
        let model = config
            .models
            .first()
            .map(|s| s.as_str())
            .unwrap_or(self.default_model);
        Ok(PreparedRequest {
            url: format!("{base}/v1/chat/completions"),
            headers: bearer_headers(token),
            body: chat_completions_body(req, model),
            is_streaming: req.stream,
        })
    }
}

// ── Private helpers ───────────────────────────────────────────────────────────

fn content_to_value(content: &MessageContent) -> Value {
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

fn assistant_message(content: &MessageContent) -> Value {
    if let MessageContent::Blocks(blocks) = content {
        let calls: Vec<Value> = blocks.iter().filter_map(|b| match b {
            ContentBlock::ToolUse { id, name, input } => Some(json!({
                "id": id, "type": "function",
                "function": { "name": name, "arguments": serde_json::to_string(input).unwrap_or_default() }
            })),
            _ => None,
        }).collect();
        if !calls.is_empty() {
            return json!({ "role": "assistant", "content": Value::Null, "tool_calls": calls });
        }
    }
    json!({ "role": "assistant", "content": content_to_value(content) })
}

fn tool_result(content: &MessageContent) -> (String, String) {
    if let MessageContent::Blocks(blocks) = content {
        for b in blocks {
            if let ContentBlock::ToolResult {
                tool_use_id,
                content: text,
            } = b
            {
                return (tool_use_id.clone(), text.clone());
            }
        }
    }
    (
        String::new(),
        match content {
            MessageContent::Text(t) => t.clone(),
            _ => String::new(),
        },
    )
}
