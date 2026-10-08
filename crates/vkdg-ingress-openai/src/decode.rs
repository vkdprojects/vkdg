//! Decode `OpenAI` Chat Completions and Images API wire requests into internal `Operation` types.

use serde::Deserialize;
use serde_json::Value;

use vkdg_core::{Capability, CapabilitySet, VkdgError};
use vkdg_operations::{
    validated_stop_sequences, validated_top_p, ContentBlock, ConversationRequest, ImageData,
    ImageGenerateRequest, Message, MessageContent, Operation, Role, Tool, ToolChoice,
    ToolResultImage,
};

// ── OpenAI wire types (deserialization only) ──────────────────────────────────

#[derive(Debug, Deserialize)]
struct OaiRequest {
    model: String,
    messages: Vec<OaiMessage>,
    stream: Option<bool>,
    tools: Option<Vec<OaiTool>>,
    /// Deprecated in favour of `max_completion_tokens`, which wins when both are sent.
    max_tokens: Option<u32>,
    max_completion_tokens: Option<u32>,
    temperature: Option<f32>,
    top_p: Option<f32>,
    n: Option<u32>,
    response_format: Option<OaiResponseFormat>,
    /// `minimal` | `low` | `medium` | `high`; reasoning models only.
    reasoning_effort: Option<String>,
    /// String or object; parsed by `decode_tool_choice` so errors name the field.
    tool_choice: Option<Value>,
    parallel_tool_calls: Option<bool>,
    /// String or array of strings; parsed by `decode_stop` so errors name the field.
    stop: Option<Value>,
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
    refusal: Option<String>,
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
    #[serde(default)]
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

/// Parse raw bytes from an `OpenAI` Chat Completions request into `(model_name, Operation)`.
///
/// # Errors
/// `ConfigInvalid` naming the offending wire field (`tool_choice`, `stop`, `top_p`,
/// `messages[i].role`, `messages[i].content`, ...) for anything the gateway cannot
/// carry faithfully.
pub fn decode_request(body: &[u8]) -> Result<(String, Operation), VkdgError> {
    let req: OaiRequest = serde_json::from_slice(body).map_err(|e| VkdgError::ConfigInvalid {
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

    let mut instructions: Vec<String> = Vec::new();
    let mut messages: Vec<Message> = Vec::with_capacity(req.messages.len());

    for (index, m) in req.messages.into_iter().enumerate() {
        if !matches!(
            m.role.as_str(),
            "system" | "developer" | "user" | "assistant" | "tool"
        ) {
            return Err(VkdgError::ConfigInvalid {
                field: format!("messages[{index}].role"),
                message: format!("unsupported role `{}`", m.role),
            });
        }
        if m.role != "assistant" && m.tool_calls.as_ref().is_some_and(|c| !c.is_empty()) {
            return Err(VkdgError::ConfigInvalid {
                field: format!("messages[{index}].tool_calls"),
                message: format!("only assistant messages carry tool_calls, not `{}`", m.role),
            });
        }

        match m.role.as_str() {
            "system" | "developer" => {
                let text = joined_text(m.content, index, false)?;
                if !text.is_empty() {
                    instructions.push(text);
                }
            }
            "tool" => {
                // role:tool carries one tool result; adapters group consecutive ones.
                let (content, images) = decode_tool_content(m.content, index)?;
                messages.push(Message {
                    role: Role::Tool,
                    content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                        tool_use_id: m.tool_call_id.unwrap_or_default(),
                        content,
                        images,
                        is_error: false,
                        cache_control: None,
                    }]),
                });
            }
            "assistant" => messages.push(decode_assistant(m, index)?),
            _ => messages.push(Message {
                role: Role::User,
                content: decode_user_content(m.content, index)?,
            }),
        }
    }
    let system = (!instructions.is_empty()).then(|| instructions.join("\n\n"));

    let tools: Vec<Tool> = req
        .tools
        .unwrap_or_default()
        .into_iter()
        .map(|t| Tool {
            name: t.function.name,
            description: t.function.description,
            input_schema: t.function.parameters,
            cache_control: None,
        })
        .collect();
    let tool_choice = ToolChoice::settle(decode_tool_choice(req.tool_choice)?, &tools, &[])?;

    let mut required_capabilities = CapabilitySet::default();
    if let Some(rf) = req.response_format {
        if rf.type_ == "json_object" {
            required_capabilities.0.insert(Capability::JsonSchema);
        }
    }

    let operation = Operation::Conversation(ConversationRequest {
        model: req.model.clone(),
        messages,
        tools,
        max_tokens: req.max_completion_tokens.or(req.max_tokens),
        temperature: req.temperature,
        stream: req.stream.unwrap_or(false),
        system,
        required_capabilities,
        thinking: req
            .reasoning_effort
            .map(|e| e.trim().to_ascii_lowercase())
            .filter(|e| !e.is_empty())
            .map(|effort| vkdg_operations::ThinkingRequest {
                budget_tokens: None,
                effort: Some(effort),
            }),
        tool_choice,
        stop_sequences: decode_stop(req.stop)?,
        top_p: validated_top_p(req.top_p)?,
        disable_parallel_tool_use: req.parallel_tool_calls == Some(false),
        ..Default::default()
    });

    Ok((req.model, operation))
}

// ── Chat helpers ──────────────────────────────────────────────────────────────

fn content_error(index: usize, message: String) -> VkdgError {
    VkdgError::ConfigInvalid {
        field: format!("messages[{index}].content"),
        message,
    }
}

fn unsupported_part(index: usize, kind: &str) -> VkdgError {
    content_error(index, format!("unsupported content part type `{kind}`"))
}

/// Text of one `text` part, or of a `refusal` part when `accept_refusal`.
fn part_text(
    part: OaiContentBlock,
    index: usize,
    accept_refusal: bool,
) -> Result<String, VkdgError> {
    match part.type_.as_str() {
        "text" => part
            .text
            .ok_or_else(|| content_error(index, "`text` part has no `text`".to_string())),
        "refusal" if accept_refusal => part
            .refusal
            .ok_or_else(|| content_error(index, "`refusal` part has no `refusal`".to_string())),
        other => Err(unsupported_part(index, other)),
    }
}

/// Content that may only hold text (system, developer, tool, assistant); parts join with `\n`.
fn joined_text(
    content: OaiContent,
    index: usize,
    accept_refusal: bool,
) -> Result<String, VkdgError> {
    match content {
        OaiContent::Text(s) => Ok(s),
        OaiContent::Null => Ok(String::new()),
        OaiContent::Blocks(parts) => Ok(parts
            .into_iter()
            .map(|p| part_text(p, index, accept_refusal))
            .collect::<Result<Vec<_>, _>>()?
            .join("\n")),
    }
}

/// A tool message: its text parts join into the result text and `image_url` parts
/// become the result's images (Chat Completions says text only, but clients that
/// return screenshots send images here).
fn decode_tool_content(
    content: OaiContent,
    index: usize,
) -> Result<(String, Vec<ToolResultImage>), VkdgError> {
    let OaiContent::Blocks(parts) = content else {
        return Ok((joined_text(content, index, false)?, Vec::new()));
    };
    let mut texts: Vec<String> = Vec::new();
    let mut images: Vec<ToolResultImage> = Vec::new();
    for part in parts {
        if part.type_ == "image_url" {
            let image = part
                .image_url
                .filter(|i| !i.url.is_empty())
                .ok_or_else(|| content_error(index, "`image_url` part has no `url`".to_string()))?;
            let (media_type, data) = image_parts(image.url);
            images.push(ToolResultImage { media_type, data });
        } else {
            texts.push(part_text(part, index, false)?);
        }
    }
    Ok((texts.join("\n"), images))
}

/// A user turn stays plain text unless it carries an image, so text-only clients
/// see no change; with an image every part keeps its position as a block.
fn decode_user_content(content: OaiContent, index: usize) -> Result<MessageContent, VkdgError> {
    let parts = match content {
        OaiContent::Text(s) => return Ok(MessageContent::Text(s)),
        OaiContent::Null => return Ok(MessageContent::Text(String::new())),
        OaiContent::Blocks(parts) => parts,
    };
    let blocks = parts
        .into_iter()
        .map(|part| match part.type_.as_str() {
            "text" => part_text(part, index, false).map(|text| ContentBlock::Text {
                text,
                cache_control: None,
            }),
            "image_url" => {
                let image = part
                    .image_url
                    .filter(|i| !i.url.is_empty())
                    .ok_or_else(|| {
                        content_error(index, "`image_url` part has no `url`".to_string())
                    })?;
                Ok(image_block(image.url))
            }
            other => Err(unsupported_part(index, other)),
        })
        .collect::<Result<Vec<_>, VkdgError>>()?;
    if blocks
        .iter()
        .any(|b| matches!(b, ContentBlock::Image { .. }))
    {
        return Ok(MessageContent::Blocks(blocks));
    }
    let texts: Vec<String> = blocks
        .into_iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text, .. } => Some(text),
            _ => None,
        })
        .collect();
    Ok(MessageContent::Text(texts.join("\n")))
}

/// `data:<media-type>;base64,<payload>` becomes inline base64; any other URL is
/// passed through with no media type (the provider fetches or sniffs it).
fn image_parts(url: String) -> (String, ImageData) {
    match base64_data_url(&url) {
        Some((media_type, data)) => (
            media_type.to_owned(),
            ImageData::Base64 {
                data: data.to_owned(),
            },
        ),
        None => (String::new(), ImageData::Url { url }),
    }
}

fn image_block(url: String) -> ContentBlock {
    let (media_type, data) = image_parts(url);
    ContentBlock::Image {
        media_type,
        data,
        cache_control: None,
    }
}

fn base64_data_url(url: &str) -> Option<(&str, &str)> {
    let (header, payload) = url.strip_prefix("data:")?.split_once(',')?;
    let mut params = header.split(';');
    let media_type = params.next().filter(|m| !m.is_empty())?;
    params
        .any(|p| p.eq_ignore_ascii_case("base64"))
        .then_some((media_type, payload))
}

fn decode_assistant(m: OaiMessage, index: usize) -> Result<Message, VkdgError> {
    let text = joined_text(m.content, index, true)?;
    let tool_calls = m.tool_calls.unwrap_or_default();
    if tool_calls.is_empty() {
        return Ok(Message {
            role: Role::Assistant,
            content: MessageContent::Text(text),
        });
    }
    let mut blocks = Vec::with_capacity(tool_calls.len() + 1);
    if !text.is_empty() {
        blocks.push(ContentBlock::Text {
            text,
            cache_control: None,
        });
    }
    blocks.extend(tool_calls.into_iter().map(|tc| ContentBlock::ToolUse {
        id: tc.id,
        input: tool_call_input(&tc.function.arguments),
        name: tc.function.name,
        cache_control: None,
    }));
    Ok(Message {
        role: Role::Assistant,
        content: MessageContent::Blocks(blocks),
    })
}

/// Tool call arguments as the JSON object the call replays as.
///
/// Deliberately lenient: clients replay model output verbatim, and models do emit
/// empty, truncated, or non-object `arguments`. Rejecting that would wedge the
/// session on every later turn, so anything that is not a JSON object becomes `{}`.
/// The result is never `Value::Null`, which providers reject as a tool input.
fn tool_call_input(arguments: &str) -> Value {
    match serde_json::from_str::<Value>(arguments) {
        Ok(object @ Value::Object(_)) => object,
        _ => Value::Object(serde_json::Map::new()),
    }
}

fn decode_tool_choice(raw: Option<Value>) -> Result<Option<ToolChoice>, VkdgError> {
    let invalid = |message: String| VkdgError::ConfigInvalid {
        field: "tool_choice".to_string(),
        message,
    };
    match raw {
        None => Ok(None),
        Some(Value::String(mode)) => match mode.as_str() {
            "auto" => Ok(Some(ToolChoice::Auto)),
            "none" => Ok(Some(ToolChoice::Disabled)),
            "required" => Ok(Some(ToolChoice::Required)),
            other => Err(invalid(format!(
                "unsupported value `{other}`; expected auto, none, required, or a function object"
            ))),
        },
        Some(Value::Object(obj)) => match obj.get("type").and_then(Value::as_str) {
            Some("function") => obj
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(Value::as_str)
                .filter(|name| !name.is_empty())
                .map(|name| Some(ToolChoice::Named(name.to_owned())))
                .ok_or_else(|| invalid("`function.name` must be a non-empty string".to_string())),
            Some(other) => Err(invalid(format!("unsupported type `{other}`"))),
            None => Err(invalid("object needs a string `type`".to_string())),
        },
        Some(_) => Err(invalid("must be a string or a function object".to_string())),
    }
}

fn decode_stop(raw: Option<Value>) -> Result<Vec<String>, VkdgError> {
    let invalid = |message: &str| VkdgError::ConfigInvalid {
        field: "stop".to_string(),
        message: message.to_string(),
    };
    let sequences = match raw {
        None => Vec::new(),
        Some(Value::String(s)) => vec![s],
        Some(Value::Array(items)) => items
            .into_iter()
            .map(|item| match item {
                Value::String(s) => Ok(s),
                _ => Err(invalid("every entry must be a string")),
            })
            .collect::<Result<Vec<_>, _>>()?,
        Some(_) => return Err(invalid("must be a string or an array of strings")),
    };
    validated_stop_sequences("stop", sequences)
}

// ── OpenAI Responses API wire types ──────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct OaiResponsesRequest {
    model: String,
    input: OaiResponsesInput,
    stream: Option<bool>,
    max_output_tokens: Option<u32>,
    instructions: Option<String>,
    reasoning: Option<OaiReasoning>,
    tools: Option<Vec<OaiResponsesTool>>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum OaiResponsesTool {
    Flat {
        #[serde(rename = "type")]
        #[allow(dead_code)]
        type_: String,
        name: String,
        description: Option<String>,
        #[serde(default)]
        parameters: serde_json::Value,
    },
    Nested(OaiTool),
}
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum OaiResponsesInput {
    Text(String),
    Items(Vec<OaiResponsesItem>),
}

#[derive(Debug, Deserialize)]
struct OaiResponsesItem {
    role: String,
    content: OaiResponsesContent,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum OaiResponsesContent {
    Text(String),
    Blocks(Vec<OaiResponsesBlock>),
}

#[derive(Debug, Deserialize)]
struct OaiResponsesBlock {
    #[serde(rename = "type")]
    type_: String,
    text: Option<String>,
    image_url: Option<OaiImageUrl>,
}

#[derive(Debug, Deserialize)]
struct OaiReasoning {
    effort: Option<String>,
}

// ── Responses decode ──────────────────────────────────────────────────────────

/// Parse raw bytes from an `OpenAI` Responses API request into `(model_name, Operation)`.
pub fn decode_responses_request(body: &[u8]) -> Result<(String, Operation), VkdgError> {
    let req: OaiResponsesRequest =
        serde_json::from_slice(body).map_err(|e| VkdgError::ConfigInvalid {
            field: "body".to_string(),
            message: e.to_string(),
        })?;

    let mut system: Option<String> = req.instructions.clone();
    let mut messages: Vec<Message> = Vec::new();

    match req.input {
        OaiResponsesInput::Text(s) => {
            messages.push(Message {
                role: Role::User,
                content: MessageContent::Text(s),
            });
        }
        OaiResponsesInput::Items(items) => {
            for item in items {
                let text = responses_content_to_string(item.content);
                match item.role.as_str() {
                    "system" => {
                        // instructions wins; only fall back to system item when absent.
                        if system.is_none() {
                            system = Some(text);
                        }
                    }
                    "assistant" => {
                        messages.push(Message {
                            role: Role::Assistant,
                            content: MessageContent::Text(text),
                        });
                    }
                    _ => {
                        messages.push(Message {
                            role: Role::User,
                            content: MessageContent::Text(text),
                        });
                    }
                }
            }
        }
    }

    let tools: Vec<Tool> = req
        .tools
        .unwrap_or_default()
        .into_iter()
        .map(|t| match t {
            OaiResponsesTool::Flat {
                name,
                description,
                parameters,
                ..
            } => Tool {
                name,
                description,
                input_schema: parameters,
                cache_control: None,
            },
            OaiResponsesTool::Nested(nested) => Tool {
                name: nested.function.name,
                description: nested.function.description,
                input_schema: nested.function.parameters,
                cache_control: None,
            },
        })
        .collect();

    let thinking =
        req.reasoning
            .and_then(|r| r.effort)
            .map(|effort| vkdg_operations::ThinkingRequest {
                budget_tokens: None,
                effort: Some(effort),
            });

    let operation = Operation::Conversation(ConversationRequest {
        model: req.model.clone(),
        messages,
        tools,
        max_tokens: req.max_output_tokens,
        temperature: None,
        stream: req.stream.unwrap_or(false),
        system,
        required_capabilities: CapabilitySet::default(),
        thinking,
        ..Default::default()
    });

    Ok((req.model, operation))
}

fn responses_content_to_string(content: OaiResponsesContent) -> String {
    match content {
        OaiResponsesContent::Text(s) => s,
        OaiResponsesContent::Blocks(blocks) => blocks
            .into_iter()
            .filter_map(|b| match b.type_.as_str() {
                "text" | "output_text" | "input_text" => b.text,
                "image_url" => b.image_url.map(|u| u.url),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
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
pub fn decode_image_generate(body: &[u8]) -> Result<(String, Operation), VkdgError> {
    let req: OaiImageGenerateRequest =
        serde_json::from_slice(body).map_err(|e| VkdgError::ConfigInvalid {
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

    fn as_bytes(s: &str) -> &[u8] {
        s.as_bytes()
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

    // ── Responses API tests ───────────────────────────────────────────────────

    // Defeat: text shorthand not mapped to single user message.
    #[test]
    fn decode_responses_text_shorthand() {
        let body = as_bytes(r#"{"model":"m","input":"hello","max_output_tokens":8}"#);
        let (model, op) = decode_responses_request(body).unwrap();
        assert_eq!(model, "m");
        let Operation::Conversation(req) = op else {
            panic!("expected Conversation")
        };
        assert_eq!(req.messages.len(), 1);
        assert_eq!(req.messages[0].role, Role::User);
        assert!(matches!(&req.messages[0].content, MessageContent::Text(t) if t == "hello"));
        assert!(!req.stream);
        assert!(req.system.is_none());
        assert_eq!(req.max_tokens, Some(8));
    }

    // Defeat: system item in array input not extracted to system field.
    #[test]
    fn decode_responses_items_with_system() {
        let body = as_bytes(
            r#"{"model":"m","input":[{"role":"system","content":"be nice"},{"role":"user","content":"hi"}]}"#,
        );
        let (_model, op) = decode_responses_request(body).unwrap();
        let Operation::Conversation(req) = op else {
            panic!("expected Conversation")
        };
        assert_eq!(req.system, Some("be nice".to_string()));
        assert_eq!(req.messages.len(), 1);
        assert_eq!(req.messages[0].role, Role::User);
    }

    // Defeat: instructions not winning over a system item in input.
    #[test]
    fn decode_responses_instructions_wins() {
        let body = as_bytes(
            r#"{"model":"m","instructions":"override","input":[{"role":"system","content":"be nice"},{"role":"user","content":"hi"}]}"#,
        );
        let (_model, op) = decode_responses_request(body).unwrap();
        let Operation::Conversation(req) = op else {
            panic!("expected Conversation")
        };
        assert_eq!(req.system, Some("override".to_string()));
    }

    #[test]
    fn decode_responses_tools_flat_and_nested() {
        let flat_body = as_bytes(
            r#"{"model":"m","input":"hi","tools":[{"type":"function","name":"flat_tool","description":"flat desc","parameters":{"type":"object"}}]}"#,
        );
        let (_model, op) = decode_responses_request(flat_body).unwrap();
        let Operation::Conversation(req) = op else {
            panic!("expected Conversation");
        };
        assert_eq!(req.tools.len(), 1);
        assert_eq!(req.tools[0].name, "flat_tool");
        assert_eq!(req.tools[0].description.as_deref(), Some("flat desc"));

        let nested_body = as_bytes(
            r#"{"model":"m","input":"hi","tools":[{"type":"function","function":{"name":"nested_tool","description":"nested desc","parameters":{"type":"object"}}}]}"#,
        );
        let (_model, op) = decode_responses_request(nested_body).unwrap();
        let Operation::Conversation(req) = op else {
            panic!("expected Conversation");
        };
        assert_eq!(req.tools.len(), 1);
        assert_eq!(req.tools[0].name, "nested_tool");
        assert_eq!(req.tools[0].description.as_deref(), Some("nested desc"));
    }
}
