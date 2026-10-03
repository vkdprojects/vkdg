//! Shared helpers for `OpenAI` Chat Completions wire format.
//!
//! All OpenAI-compatible providers (Groq, Together, Fireworks, `DeepSeek`,
//! Mistral, Gemini) delegate here instead of duplicating conversion logic.

mod messages;

use bytes::Bytes;
use http::HeaderMap;
use serde_json::{json, Map, Value};
use vkdg_connections::{ConnectionConfig, Credential, ProviderKind};
use vkdg_operations::{ConversationRequest, Operation, ToolChoice};

use self::messages::chat_messages;
use crate::sampling_json::insert_f32;
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

/// Which field carries the output limit. `OpenAI` itself deprecated `max_tokens`
/// and its reasoning models reject it; compatible servers still expect it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenLimitField {
    /// `max_tokens`, what compatible servers expect.
    MaxTokens,
    /// `max_completion_tokens`, what api.openai.com takes for every current model.
    MaxCompletionTokens,
}

/// Serialize a `ConversationRequest` to `OpenAI` Chat Completions JSON for a
/// compatible server (`max_tokens`).
pub fn chat_completions_body(req: &ConversationRequest, model: &str) -> Bytes {
    chat_completions_body_with(req, model, TokenLimitField::MaxTokens)
}

/// Like [`chat_completions_body`], with the output limit under `limit`. Exactly
/// one limit field is ever sent.
///
/// Tool history is repaired to what `OpenAI` validates (see
/// [`crate::turns::arrange`]). Tool policy fields are sent only with tools:
/// `tool_choice`, and `parallel_tool_calls: false` for a client that forbade
/// parallel calls. `stream` adds `stream_options.include_usage` so the final
/// chunk carries the usage the gateway meters.
pub fn chat_completions_body_with(
    req: &ConversationRequest,
    model: &str,
    limit: TokenLimitField,
) -> Bytes {
    let mut body = Map::new();
    body.insert("model".into(), Value::String(model.into()));
    body.insert("messages".into(), Value::Array(chat_messages(req)));
    if let Some(v) = req.max_tokens {
        let key = match limit {
            TokenLimitField::MaxTokens => "max_tokens",
            TokenLimitField::MaxCompletionTokens => "max_completion_tokens",
        };
        body.insert(key.into(), json!(v));
    }
    if let Some(v) = req.temperature {
        insert_f32(&mut body, "temperature", v);
    }
    if let Some(v) = req.top_p {
        insert_f32(&mut body, "top_p", v);
    }
    if !req.stop_sequences.is_empty() {
        body.insert("stop".into(), json!(req.stop_sequences));
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
        if let Some(choice) = &req.tool_choice {
            body.insert("tool_choice".into(), tool_choice_wire(choice));
        }
        if req.disable_parallel_tool_use {
            body.insert("parallel_tool_calls".into(), Value::Bool(false));
        }
    }

    // A `Value` tree always serializes; the empty fallback only satisfies the type.
    Bytes::from(serde_json::to_vec(&Value::Object(body)).unwrap_or_default())
}

fn tool_choice_wire(choice: &ToolChoice) -> Value {
    match choice {
        ToolChoice::Auto => json!("auto"),
        ToolChoice::Required => json!("required"),
        ToolChoice::Disabled => json!("none"),
        ToolChoice::Named(name) => json!({ "type": "function", "function": { "name": name } }),
    }
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
    icon_char: char,
    icon_color: &'static str,
    site_url: Option<&'static str>,
    description: Option<&'static str>,
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
            icon_char: ' ',
            icon_color: "#6b7280",
            site_url: None,
            description: None,
        }
    }

    /// Builder method: set visual metadata for the admin console.
    #[must_use]
    pub const fn with_meta(
        mut self,
        icon_char: char,
        icon_color: &'static str,
        site_url: Option<&'static str>,
        description: Option<&'static str>,
    ) -> Self {
        self.icon_char = icon_char;
        self.icon_color = icon_color;
        self.site_url = site_url;
        self.description = description;
        self
    }
}

impl ProviderAdapter for OpenAiCompatAdapter {
    fn id(&self) -> &str {
        self.id
    }
    fn display_name(&self) -> &str {
        self.display_name
    }

    fn wire_format(&self, _config: &ConnectionConfig) -> Option<vkdg_operations::WireFormat> {
        Some(vkdg_operations::WireFormat::OpenAiChat)
    }

    fn prepare(
        &self,
        operation: &Operation,
        config: &ConnectionConfig,
        credential: &Credential,
    ) -> Result<PreparedRequest, ProviderError> {
        let token = credential.token.as_str();
        let Operation::Conversation(req) = operation else {
            return Err(ProviderError::UnsupportedOperation);
        };
        let base = match &config.provider {
            ProviderKind::Custom { base_url } => base_url.as_str(),
            _ => self.default_base_url,
        };
        let model = crate::upstream_model(req, self.default_model);
        Ok(PreparedRequest {
            url: format!("{base}/v1/chat/completions"),
            headers: bearer_headers(token),
            body: chat_completions_body(req, model),
            is_streaming: req.stream,
        })
    }

    fn meta(&self) -> crate::ProviderMeta {
        crate::ProviderMeta {
            icon_char: if self.icon_char == ' ' {
                self.id.chars().next().unwrap_or('?').to_ascii_uppercase()
            } else {
                self.icon_char
            },
            icon_color: self.icon_color,
            category: crate::ProviderCategory::LlmApi,
            site_url: self.site_url,
            description: self.description,
        }
    }
}

#[cfg(test)]
mod tests;
