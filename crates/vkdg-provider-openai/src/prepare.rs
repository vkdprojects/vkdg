//! Build [`PreparedRequest`] for the OpenAI Chat Completions and Images APIs.

use bytes::Bytes;
use http::HeaderMap;
use serde_json::{json, Map, Value};
use vkdg_connections::{ConnectionConfig, ProviderKind};
use vkdg_core::VkdgError;
use vkdg_http::provider::{PreparedRequest, ProviderAdapter};
use vkdg_operations::{
    ContentBlock, ConversationRequest, ImageGenerateRequest, MessageContent, Operation, Role,
    VideoGenerateRequest,
};

/// OpenAI provider adapter; converts internal operations to Chat Completions requests.
pub struct OpenAIAdapter;

impl ProviderAdapter for OpenAIAdapter {
    fn name(&self) -> &str {
        "openai"
    }

    fn prepare(
        &self,
        operation: &Operation,
        config: &ConnectionConfig,
        token: &str,
    ) -> Result<PreparedRequest, VkdgError> {
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
            Operation::ImageGenerate(req) => {
                let body = build_image_generate_body(req, config);
                let url = format!("{}/v1/images/generations", base_url(config));
                let headers = build_auth_headers(token);
                Ok(PreparedRequest {
                    url,
                    headers,
                    body,
                    is_streaming: false,
                })
            }
            Operation::ImageEdit(_) => Err(VkdgError::CapabilityUnsupported {
                capability: "image_edit_multipart".into(),
            }),
            Operation::VideoGenerate(req) => {
                let body = build_video_generate_body(req);
                let url = format!("{}/v1/videos/generations", base_url(config));
                let headers = build_auth_headers(token);
                Ok(PreparedRequest {
                    url,
                    headers,
                    body,
                    is_streaming: false,
                })
            }
            _ => Err(VkdgError::CapabilityUnsupported {
                capability: "operation_not_implemented".into(),
            }),
        }
    }
}

/// Resolve base URL from connection config; falls back to api.openai.com.
fn base_url(config: &ConnectionConfig) -> String {
    match &config.provider {
        ProviderKind::OpenAI => "https://api.openai.com".into(),
        ProviderKind::Custom { base_url } => base_url.clone(),
        _ => "https://api.openai.com".into(),
    }
}

/// Build common Authorization + Content-Type headers for OpenAI API requests.
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

/// Serialise an [`ImageGenerateRequest`] to an OpenAI Images generations JSON body.
/// Only non-`None` optional fields are included.
pub(crate) fn build_image_generate_body(
    req: &ImageGenerateRequest,
    config: &ConnectionConfig,
) -> Bytes {
    let model = req.model.clone().or_else(|| config.models.first().cloned()).unwrap_or_else(|| "dall-e-3".into());

    let mut body = Map::new();
    body.insert("prompt".into(), Value::String(req.prompt.clone()));
    body.insert("model".into(), Value::String(model));
    if let Some(n) = req.n {
        body.insert("n".into(), json!(n));
    }
    if let Some(size) = &req.size {
        body.insert("size".into(), Value::String(size.clone()));
    }
    if let Some(quality) = &req.quality {
        body.insert("quality".into(), Value::String(quality.clone()));
    }
    if let Some(style) = &req.style {
        body.insert("style".into(), Value::String(style.clone()));
    }
    if let Some(fmt) = &req.response_format {
        body.insert("response_format".into(), Value::String(fmt.clone()));
    }
    if let Some(user) = &req.user {
        body.insert("user".into(), Value::String(user.clone()));
    }
    Bytes::from(serde_json::to_vec(&Value::Object(body)).unwrap_or_default())
}

/// Serialise a [`VideoGenerateRequest`] to a video generations JSON body.
/// Only non-`None` optional fields are included.
pub(crate) fn build_video_generate_body(req: &VideoGenerateRequest) -> Bytes {
    let mut body = Map::new();
    body.insert("prompt".into(), Value::String(req.prompt.clone()));
    if let Some(model) = &req.model {
        body.insert("model".into(), Value::String(model.clone()));
    }
    if let Some(dur) = req.duration_secs {
        body.insert("duration_secs".into(), json!(dur));
    }
    if let Some(res) = &req.resolution {
        body.insert("resolution".into(), Value::String(res.clone()));
    }
    if let Some(style) = &req.style {
        body.insert("style".into(), Value::String(style.clone()));
    }
    if let Some(audio) = req.audio {
        body.insert("audio".into(), json!(audio));
    }
    if let Some(url) = &req.webhook_url {
        body.insert("webhook_url".into(), Value::String(url.clone()));
    }
    Bytes::from(serde_json::to_vec(&Value::Object(body)).unwrap_or_default())
}


/// Serialise a [`ConversationRequest`] to an OpenAI Chat Completions JSON body.
fn build_body(req: &ConversationRequest, config: &ConnectionConfig) -> Bytes {
    let model = config
        .models
        .first()
        .cloned()
        .unwrap_or_else(|| "gpt-4o".into());

    // Prepend system prompt as a role:system message when present.
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
                // If the message contains ToolUse blocks, emit tool_calls format.
                if let MessageContent::Blocks(blocks) = &m.content {
                    let tool_calls: Vec<Value> = blocks
                        .iter()
                        .filter_map(|b| match b {
                            ContentBlock::ToolUse { id, name, input } => {
                                let arguments = serde_json::to_string(input)
                                    .unwrap_or_else(|_| "{}".into());
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
                // Extract tool_call_id and content string from ToolResult block.
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
        // Request usage in the final chunk so the pipeline can report it.
        body.insert(
            "stream_options".into(),
            json!({ "include_usage": true }),
        );
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

/// Convert [`MessageContent`] to a JSON value suitable for OpenAI's `content` field.
fn message_content_to_value(content: &MessageContent) -> Value {
    match content {
        MessageContent::Text(t) => Value::String(t.clone()),
        MessageContent::Blocks(blocks) => {
            // Convert text blocks to OpenAI content part objects; skip non-text.
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

/// Extract `(tool_call_id, content_string)` from a Tool role message content.
/// Looks for the first [`ContentBlock::ToolResult`]; falls back to empty strings.
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
    // Plain text in a Tool message — use empty id, propagate text as-is.
    let text = match content {
        MessageContent::Text(t) => t.clone(),
        _ => String::new(),
    };
    (String::new(), text)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use vkdg_connections::{AuthKind, ConnectionConfig, ProviderKind};
    use vkdg_core::{CapabilitySet, ConnectionId};
    use vkdg_operations::{ContentBlock, ConversationRequest, ImageGenerateRequest, Message, MessageContent, Operation, Role, Tool};

    fn openai_config() -> ConnectionConfig {
        ConnectionConfig {
            id: ConnectionId("test".into()),
            provider: ProviderKind::OpenAI,
            auth: AuthKind::ApiKey {
                env_var: "OPENAI_API_KEY".into(),
            },
            models: vec!["gpt-4o".into()],
            max_concurrent: 4,
            weight: 1,
            tags: vec![],
            capabilities: CapabilitySet::default(),
        }
    }

    fn custom_config(url: &str) -> ConnectionConfig {
        ConnectionConfig {
            id: ConnectionId("custom".into()),
            provider: ProviderKind::Custom {
                base_url: url.into(),
            },
            auth: AuthKind::ApiKey {
                env_var: "KEY".into(),
            },
            models: vec!["gpt-4o".into()],
            max_concurrent: 4,
            weight: 1,
            tags: vec![],
            capabilities: CapabilitySet::default(),
        }
    }

    fn simple_request() -> ConversationRequest {
        ConversationRequest {
            messages: vec![Message {
                role: Role::User,
                content: MessageContent::Text("hello".into()),
            }],
            tools: vec![],
            max_tokens: None,
            temperature: None,
            stream: false,
            system: None,
            required_capabilities: CapabilitySet::default(),
        }
    }

    /// A basic request should produce model and messages fields in the JSON body.
    #[test]
    fn prepare_basic_body() {
        let req = simple_request();
        let body = build_body(&req, &openai_config());
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["model"], "gpt-4o");
        assert_eq!(v["messages"][0]["role"], "user");
        assert_eq!(v["messages"][0]["content"], "hello");
    }

    /// system Some(s) must appear as first message with role:system.
    #[test]
    fn prepare_system_injected() {
        let mut req = simple_request();
        req.system = Some("be concise".into());
        let body = build_body(&req, &openai_config());
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["messages"][0]["role"], "system");
        assert_eq!(v["messages"][0]["content"], "be concise");
        assert_eq!(v["messages"][1]["role"], "user");
    }

    /// Non-empty tools array must produce an OpenAI tools array with type:function entries.
    #[test]
    fn prepare_tools() {
        let mut req = simple_request();
        req.tools = vec![Tool {
            name: "get_weather".into(),
            description: Some("Return weather".into()),
            input_schema: json!({ "type": "object", "properties": {} }),
        }];
        let body = build_body(&req, &openai_config());
        let v: Value = serde_json::from_slice(&body).unwrap();
        let tool = &v["tools"][0];
        assert_eq!(tool["type"], "function");
        assert_eq!(tool["function"]["name"], "get_weather");
        assert_eq!(tool["function"]["description"], "Return weather");
    }

    /// stream:true must include both stream:true and stream_options:{include_usage:true}.
    #[test]
    fn prepare_stream_options() {
        let mut req = simple_request();
        req.stream = true;
        let body = build_body(&req, &openai_config());
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["stream"], true);
        assert_eq!(v["stream_options"]["include_usage"], true);
    }

    /// An assistant message with ToolUse blocks must produce role:assistant with tool_calls.
    #[test]
    fn prepare_tool_use_in_assistant() {
        let mut req = simple_request();
        req.messages.push(Message {
            role: Role::Assistant,
            content: MessageContent::Blocks(vec![ContentBlock::ToolUse {
                id: "call_1".into(),
                name: "get_weather".into(),
                input: json!({ "location": "NYC" }),
            }]),
        });
        let body = build_body(&req, &openai_config());
        let v: Value = serde_json::from_slice(&body).unwrap();
        let asst = &v["messages"][1];
        assert_eq!(asst["role"], "assistant");
        assert!(asst["tool_calls"].is_array());
        let tc = &asst["tool_calls"][0];
        assert_eq!(tc["id"], "call_1");
        assert_eq!(tc["type"], "function");
        assert_eq!(tc["function"]["name"], "get_weather");
        // arguments must be a JSON string
        let args: Value =
            serde_json::from_str(tc["function"]["arguments"].as_str().unwrap()).unwrap();
        assert_eq!(args["location"], "NYC");
    }

    /// A Tool role message must produce role:tool with tool_call_id and content string.
    #[test]
    fn prepare_tool_result_message() {
        let mut req = simple_request();
        req.messages.push(Message {
            role: Role::Tool,
            content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                tool_use_id: "call_1".into(),
                content: "sunny, 22°C".into(),
            }]),
        });
        let body = build_body(&req, &openai_config());
        let v: Value = serde_json::from_slice(&body).unwrap();
        let tool_msg = &v["messages"][1];
        assert_eq!(tool_msg["role"], "tool");
        assert_eq!(tool_msg["tool_call_id"], "call_1");
        assert_eq!(tool_msg["content"], "sunny, 22°C");
    }

    /// Authorization header must be Bearer <token>.
    #[test]
    fn prepare_auth_is_bearer() {
        let req = Operation::Conversation(simple_request());
        let adapter = OpenAIAdapter;
        let prepared = adapter.prepare(&req, &openai_config(), "sk-test123").unwrap();
        let auth = prepared
            .headers
            .get(http::header::AUTHORIZATION)
            .unwrap()
            .to_str()
            .unwrap();
        assert_eq!(auth, "Bearer sk-test123");
    }

    /// ProviderKind::OpenAI must produce a URL pointing to api.openai.com.
    #[test]
    fn prepare_url_openai() {
        let url = base_url(&openai_config());
        assert!(url.contains("api.openai.com"), "url={url}");
    }

    /// ProviderKind::Custom must use the supplied base_url verbatim.
    #[test]
    fn prepare_url_custom() {
        let cfg = custom_config("http://localhost:8080");
        let url = base_url(&cfg);
        assert!(url.starts_with("http://localhost:8080"), "url={url}");
    }

    // Defeat: build_image_generate_body includes None optional fields as JSON nulls,
    // causing OpenAI to reject the request or use unexpected defaults.
    #[test]
    fn prepare_image_generate_omits_none_fields() {
        let req = ImageGenerateRequest {
            prompt: "a cat".into(),
            model: None,
            n: None,
            size: None,
            quality: None,
            style: None,
            response_format: None,
            user: None,
        };
        let cfg = openai_config();
        let body = build_image_generate_body(&req, &cfg);
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert!(v.get("n").is_none(), "None n must not appear in body");
        assert!(v.get("size").is_none(), "None size must not appear in body");
        assert!(v.get("quality").is_none(), "None quality must not appear in body");
        assert!(v.get("style").is_none(), "None style must not appear in body");
        assert!(v.get("response_format").is_none(), "None response_format must not appear in body");
        assert!(v.get("user").is_none(), "None user must not appear in body");
        assert_eq!(v["prompt"], "a cat");
    }

    // Defeat: prompt or size are serialized with wrong keys or types, causing
    // OpenAI to return 400 unrecognized_field.
    #[test]
    fn prepare_image_generate_body() {
        let req = ImageGenerateRequest {
            prompt: "a red barn".into(),
            model: Some("dall-e-3".into()),
            n: Some(1),
            size: Some("1024x1024".into()),
            quality: Some("hd".into()),
            style: Some("vivid".into()),
            response_format: Some("url".into()),
            user: None,
        };
        let cfg = openai_config();
        let body = build_image_generate_body(&req, &cfg);
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["prompt"], "a red barn");
        assert_eq!(v["model"], "dall-e-3");
        assert_eq!(v["n"], 1);
        assert_eq!(v["size"], "1024x1024");
        assert_eq!(v["quality"], "hd");
        assert_eq!(v["style"], "vivid");
        assert_eq!(v["response_format"], "url");
    }

    // Defeat: ImageGenerate prepare uses /v1/chat/completions URL instead of
    // the Images API endpoint, silently routing image requests to the wrong API.
    #[test]
    fn prepare_image_generate_url() {
        let op = Operation::ImageGenerate(ImageGenerateRequest {
            prompt: "a cat".into(),
            model: None,
            n: None,
            size: None,
            quality: None,
            style: None,
            response_format: None,
            user: None,
        });
        let adapter = OpenAIAdapter;
        let prepared = adapter.prepare(&op, &openai_config(), "sk-test").unwrap();
        assert!(
            prepared.url.contains("/v1/images/generations"),
            "url must contain /v1/images/generations, got: {}",
            prepared.url
        );
    }

    // Defeat: VideoGenerate prepare uses wrong URL (/v1/chat/completions or /v1/images/generations)
    // instead of the video generations endpoint, silently routing video requests to the wrong API.
    #[test]
    fn prepare_video_generate_url() {
        let op = Operation::VideoGenerate(VideoGenerateRequest {
            prompt: "a sunset timelapse".into(),
            model: None,
            duration_secs: None,
            resolution: None,
            style: None,
            reference_image: None,
            audio: None,
            webhook_url: None,
        });
        let adapter = OpenAIAdapter;
        let prepared = adapter.prepare(&op, &openai_config(), "sk-test").unwrap();
        assert!(
            prepared.url.contains("/v1/videos/generations"),
            "url must contain /v1/videos/generations, got: {}",
            prepared.url
        );
    }

    // Defeat: prompt is dropped or serialised under a different key, causing the
    // upstream to reject the request with a missing-required-field error.
    #[test]
    fn prepare_video_generate_body_prompt() {
        let req = VideoGenerateRequest {
            prompt: "dancing robots".into(),
            model: Some("sora".into()),
            duration_secs: Some(10),
            resolution: Some("1920x1080".into()),
            style: None,
            reference_image: None,
            audio: Some(true),
            webhook_url: None,
        };
        let body = build_video_generate_body(&req);
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["prompt"], "dancing robots", "prompt must be present in body");
        assert_eq!(v["model"], "sora");
        assert_eq!(v["duration_secs"], 10);
        assert_eq!(v["resolution"], "1920x1080");
        assert_eq!(v["audio"], true);
        assert!(v.get("style").is_none(), "None style must not appear in body");
        assert!(v.get("webhook_url").is_none(), "None webhook_url must not appear in body");
    }
}
