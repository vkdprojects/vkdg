//! Anthropic wire types — deserialization only; not part of the public API.

use serde::{Deserialize, Serialize};

// ── Request wire types ────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(crate) struct AnthropicRequest {
    pub(crate) model: String,
    pub(crate) messages: Vec<AnthropicMessage>,
    pub(crate) system: Option<String>,
    pub(crate) max_tokens: Option<u32>,
    pub(crate) temperature: Option<f32>,
    pub(crate) stream: Option<bool>,
    pub(crate) tools: Option<Vec<AnthropicTool>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AnthropicMessage {
    pub(crate) role: String,
    pub(crate) content: AnthropicContent,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum AnthropicContent {
    Text(String),
    Blocks(Vec<AnthropicBlock>),
}

#[derive(Debug, Deserialize)]
pub(crate) struct AnthropicBlock {
    #[serde(rename = "type")]
    pub(crate) type_: String,
    // text block
    pub(crate) text: Option<String>,
    // tool_use block
    pub(crate) id: Option<String>,
    pub(crate) name: Option<String>,
    pub(crate) input: Option<serde_json::Value>,
    // tool_result block
    pub(crate) tool_use_id: Option<String>,
    pub(crate) content: Option<AnthropicToolResultContent>,
    // image block
    pub(crate) source: Option<AnthropicImageSource>,
}

/// The `content` field of a `tool_result` block may be a plain string or
/// an array of sub-blocks (images, text, etc.).
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum AnthropicToolResultContent {
    Text(String),
    Blocks(Vec<AnthropicBlock>),
}

#[derive(Debug, Deserialize)]
pub(crate) struct AnthropicImageSource {
    #[serde(rename = "type")]
    pub(crate) type_: String,          // "base64" | "url"
    pub(crate) media_type: Option<String>,
    pub(crate) data: Option<String>,   // base64 payload
    pub(crate) url: Option<String>,    // URL payload
}

#[derive(Debug, Deserialize)]
pub(crate) struct AnthropicTool {
    pub(crate) name: String,
    pub(crate) description: Option<String>,
    pub(crate) input_schema: serde_json::Value,
}

// ── Error wire types ──────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub(crate) struct AnthropicErrorBody {
    #[serde(rename = "type")]
    pub(crate) type_: String,
    pub(crate) error: AnthropicErrorDetail,
}

#[derive(Debug, Serialize)]
pub(crate) struct AnthropicErrorDetail {
    #[serde(rename = "type")]
    pub(crate) type_: String,
    pub(crate) message: String,
}

impl AnthropicErrorBody {
    pub(crate) fn new(error_type: &str, message: impl Into<String>) -> Self {
        Self {
            type_: "error".to_string(),
            error: AnthropicErrorDetail {
                type_: error_type.to_string(),
                message: message.into(),
            },
        }
    }
}
