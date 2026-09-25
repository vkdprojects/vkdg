//! Decode OpenAI Chat Completions and Images API wire requests into internal `Operation` types.

use bytes::Bytes;
use serde::Deserialize;

use vkdg_core::{Capability, CapabilitySet, VkdgError};
use vkdg_operations::{
    ContentBlock, ConversationRequest, ImageGenerateRequest, Message, MessageContent, Operation,
    Role, Tool,
};

// ── OpenAI wire types (deserialization only) ──────────────────────────────────

#[derive(Debug, Deserialize)]
struct OaiRequest {
    model: String,
    messages: Vec<OaiMessage>,
    stream: Option<bool>,
    tools: Option<Vec<OaiTool>>,
    max_tokens: Option<u32>,
    temperature: Option<f32>,
    n: Option<u32>,
    response_format: Option<OaiResponseFormat>,
}

#[derive(Debug, Deserialize)]
struct OaiMessage {
    role: String,
    #[serde(default)]
    content: OaiContent,
    tool_call_id: Option<String>,
    tool_calls: Option<Vec<OaiToolCall>>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(untagged)]
enum OaiContent {
    Text(String),
    Blocks(Vec<OaiContentBlock>),
    #[default]
    Null,
}

#[derive(Debug, Deserialize)]
struct OaiContentBlock {
    #[serde(rename = "type")]
    type_: String,
    text: Option<String>,
    image_url: Option<OaiImageUrl>,
}

#[derive(Debug, Deserialize)]
struct OaiImageUrl {
    url: String,
}

#[derive(Debug, Deserialize)]
struct OaiToolCall {
    id: String,
    #[serde(rename = "type")]
    #[allow(dead_code)]
    type_: String,
    function: OaiToolCallFunction,
}

#[derive(Debug, Deserialize)]
struct OaiToolCallFunction {
    name: String,
    arguments: String,
}

#[derive(Debug, Deserialize)]
struct OaiTool {
    #[serde(rename = "type")]
    #[allow(dead_code)]
    type_: String,
    function: OaiToolFunction,
}

#[derive(Debug, Deserialize)]
struct OaiToolFunction {
    name: String,
    description: Option<String>,
    parameters: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct OaiResponseFormat {
    #[serde(rename = "type")]
    type_: String,
}

// ── Decode ────────────────────────────────────────────────────────────────────

/// Parse raw bytes from an OpenAI Chat Completions request into `(model_name, Operation)`.
pub fn decode_request(body: Bytes) -> Result<(String, Operation), VkdgError> {
    let req: OaiRequest = serde_json::from_slice(&body).map_err(|e| VkdgError::ConfigInvalid {
        field: "body".to_string(),
        message: e.to_string(),
    })?;

    // n > 1 is not supported; reject early with a clear error.
    if req.n.unwrap_or(1) > 1 {
        return Err(VkdgError::ConfigInvalid {
            field: "n".to_string(),
            message: "n>1 not supported".to_string(),
        });
    }

    let mut system: Option<String> = None;
    let mut messages: Vec<Message> = Vec::with_capacity(req.messages.len());

    for m in req.messages {
        match m.role.as_str() {
            "system" => {
                // First system message wins; subsequent ones are discarded.
                if system.is_none() {
                    system = Some(oai_content_to_string(m.content));
                }
            }
            "tool" => {
                // role:tool carries a tool result; map to ToolResult content block.
                let tool_use_id = m.tool_call_id.unwrap_or_default();
                let content_text = oai_content_to_string(m.content);
                let block = ContentBlock::ToolResult {
                    tool_use_id,
                    content: content_text,
                };
                messages.push(Message {
                    role: Role::Tool,
                    content: MessageContent::Blocks(vec![block]),
                });
            }
            role_str => {
                let role = match role_str {
                    "assistant" => Role::Assistant,
                    _ => Role::User,
                };

                // assistant messages may carry tool_calls instead of (or in addition to) content.
                if let Some(tool_calls) = m.tool_calls {
                    let blocks: Vec<ContentBlock> = tool_calls
                        .into_iter()
                        .map(|tc| {
                            let input: serde_json::Value =
                                serde_json::from_str(&tc.function.arguments)
                                    .unwrap_or(serde_json::Value::Null);
                            ContentBlock::ToolUse {
                                id: tc.id,
                                name: tc.function.name,
                                input,
                            }
                        })
                        .collect();
                    messages.push(Message {
                        role,
                        content: MessageContent::Blocks(blocks),
                    });
                } else {
                    messages.push(Message {
                        role,
                        content: MessageContent::Text(oai_content_to_string(m.content)),
                    });
                }
            }
        }
    }

    let tools: Vec<Tool> = req
        .tools
        .unwrap_or_default()
        .into_iter()
        .map(|t| Tool {
            name: t.function.name,
            description: t.function.description,
            input_schema: t.function.parameters,
        })
        .collect();

    let mut required_capabilities = CapabilitySet::default();
    if let Some(rf) = req.response_format {
        if rf.type_ == "json_object" {
            required_capabilities.0.insert(Capability::JsonSchema);
        }
    }

    let operation = Operation::Conversation(ConversationRequest {
        messages,
        tools,
        max_tokens: req.max_tokens,
        temperature: req.temperature,
        stream: req.stream.unwrap_or(false),
        system,
        required_capabilities,
    });

    Ok((req.model, operation))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn oai_content_to_string(content: OaiContent) -> String {
    match content {
        OaiContent::Text(s) => s,
        OaiContent::Blocks(blocks) => blocks
            .into_iter()
            .filter_map(|b| match b.type_.as_str() {
                "text" => b.text,
                "image_url" => b.image_url.map(|u| u.url),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        OaiContent::Null => String::new(),
    }
}

// ── OpenAI Images API wire type ───────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct OaiImageGenerateRequest {
    model: String,
    prompt: Option<String>,
    n: Option<u32>,
    size: Option<String>,
    quality: Option<String>,
    style: Option<String>,
    response_format: Option<String>,
    user: Option<String>,
}

// ── Images decode ─────────────────────────────────────────────────────────────

/// Decode POST /v1/images/generations body into `(model_name, Operation::ImageGenerate)`.
///
/// Returns `Err(ConfigInvalid)` when `prompt` is missing.
pub fn decode_image_generate(body: Bytes) -> Result<(String, Operation), VkdgError> {
    let req: OaiImageGenerateRequest =
        serde_json::from_slice(&body).map_err(|e| VkdgError::ConfigInvalid {
            field: "body".to_string(),
            message: e.to_string(),
        })?;

    let prompt = req.prompt.ok_or_else(|| VkdgError::ConfigInvalid {
        field: "prompt".to_string(),
        message: "prompt is required".to_string(),
    })?;

    let model = req.model.clone();
    let operation = Operation::ImageGenerate(ImageGenerateRequest {
        prompt,
        model: Some(req.model),
        n: req.n,
        size: req.size,
        quality: req.quality,
        style: req.style,
        response_format: req.response_format,
        user: req.user,
    });

    Ok((model, operation))
}


// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn as_bytes(s: &str) -> Bytes {
        Bytes::from(s.to_string())
    }

    // Defeat: mapping role:user content to wrong type or losing the text.
    #[test]
    fn decode_text_message() {
        let body = as_bytes(
            r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hello"}]}"#,
        );
        let (_model, op) = decode_request(body).unwrap();
        let Operation::Conversation(req) = op else { panic!("expected Conversation") };
        assert_eq!(req.messages.len(), 1);
        assert_eq!(req.messages[0].role, Role::User);
        assert!(matches!(&req.messages[0].content, MessageContent::Text(t) if t == "hello"));
    }

    // Defeat: treating role:system as a regular message instead of extracting to system field.
    #[test]
    fn decode_system_as_field() {
        let body = as_bytes(
            r#"{"model":"gpt-4o","messages":[{"role":"system","content":"be nice"},{"role":"user","content":"hi"}]}"#,
        );
        let (_model, op) = decode_request(body).unwrap();
        let Operation::Conversation(req) = op else { panic!("expected Conversation") };
        assert_eq!(req.system, Some("be nice".to_string()));
        // system message must NOT appear in messages[]
        assert_eq!(req.messages.len(), 1);
        assert_eq!(req.messages[0].role, Role::User);
    }

    // Defeat: losing tool_call_id or mapping to wrong ContentBlock variant.
    #[test]
    fn decode_tool_message() {
        let body = as_bytes(
            r#"{"model":"gpt-4o","messages":[{"role":"tool","tool_call_id":"call_123","content":"42"}]}"#,
        );
        let (_model, op) = decode_request(body).unwrap();
        let Operation::Conversation(req) = op else { panic!("expected Conversation") };
        assert_eq!(req.messages.len(), 1);
        assert_eq!(req.messages[0].role, Role::Tool);
        let MessageContent::Blocks(blocks) = &req.messages[0].content else {
            panic!("expected Blocks")
        };
        assert!(matches!(
            &blocks[0],
            ContentBlock::ToolResult { tool_use_id, content }
                if tool_use_id == "call_123" && content == "42"
        ));
    }

    // Defeat: ignoring tool_calls field or mapping to wrong ContentBlock variant.
    #[test]
    fn decode_assistant_with_tool_calls() {
        let body = as_bytes(
            r#"{"model":"gpt-4o","messages":[{"role":"assistant","tool_calls":[{"id":"call_abc","type":"function","function":{"name":"get_weather","arguments":"{\"city\":\"Paris\"}"}}]}]}"#,
        );
        let (_model, op) = decode_request(body).unwrap();
        let Operation::Conversation(req) = op else { panic!("expected Conversation") };
        assert_eq!(req.messages.len(), 1);
        let MessageContent::Blocks(blocks) = &req.messages[0].content else {
            panic!("expected Blocks")
        };
        assert!(matches!(
            &blocks[0],
            ContentBlock::ToolUse { id, name, .. }
                if id == "call_abc" && name == "get_weather"
        ));
    }

    // Defeat: silently accepting n>1 and producing invalid downstream behaviour.
    #[test]
    fn decode_reject_n_greater_than_1() {
        let body = as_bytes(
            r#"{"model":"gpt-4o","n":2,"messages":[{"role":"user","content":"hi"}]}"#,
        );
        let err = decode_request(body).unwrap_err();
        assert!(matches!(&err, VkdgError::ConfigInvalid { field, .. } if field == "n"));
    }

    // Defeat: dropping the stream flag so callers always get buffered responses.
    #[test]
    fn decode_stream_flag() {
        let body = as_bytes(
            r#"{"model":"gpt-4o","stream":true,"messages":[{"role":"user","content":"hi"}]}"#,
        );
        let (_model, op) = decode_request(body).unwrap();
        let Operation::Conversation(req) = op else { panic!("expected Conversation") };
        assert!(req.stream);
    }

    // Defeat: ignoring response_format so json_object requests get no JsonSchema capability.
    #[test]
    fn decode_response_format_json_schema_capability() {
        let body = as_bytes(
            r#"{"model":"gpt-4o","response_format":{"type":"json_object"},"messages":[{"role":"user","content":"hi"}]}"#,
        );
        let (_model, op) = decode_request(body).unwrap();
        let Operation::Conversation(req) = op else { panic!("expected Conversation") };
        assert!(
            req.required_capabilities.0.contains(&Capability::JsonSchema),
            "json_object response_format must insert Capability::JsonSchema"
        );
    }

    // Defeat: decode_image_generate returns Ok for a body with missing prompt,
    // causing downstream providers to get an empty prompt.
    #[test]
    fn decode_image_generate_missing_prompt() {
        let body = as_bytes(r#"{"model":"dall-e-3"}"#);
        let err = decode_image_generate(body).unwrap_err();
        assert!(
            matches!(&err, VkdgError::ConfigInvalid { field, .. } if field == "prompt"),
            "missing prompt must produce ConfigInvalid {{ field: \"prompt\" }}, got: {err:?}"
        );
    }

    // Defeat: decode_image_generate drops the model name or returns the wrong
    // operation variant, causing the pipeline to route to the wrong provider.
    #[test]
    fn decode_image_generate_basic() {
        let body = as_bytes(r#"{"model":"dall-e-3","prompt":"a cat","n":1,"size":"1024x1024"}"#);
        let (model, op) = decode_image_generate(body).unwrap();
        assert_eq!(model, "dall-e-3");
        let Operation::ImageGenerate(req) = op else {
            panic!("expected Operation::ImageGenerate")
        };
        assert_eq!(req.prompt, "a cat");
        assert_eq!(req.n, Some(1));
        assert_eq!(req.size.as_deref(), Some("1024x1024"));
    }
}
