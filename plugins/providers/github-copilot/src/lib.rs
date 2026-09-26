//! VKDG provider plugin: github-copilot
//! GitHub Copilot — Device Code OAuth with OpenAI Chat Completions wire format.

use std::collections::HashMap;

use bytes::Bytes;
use futures::future::BoxFuture;
use http::HeaderMap;
use serde_json::{json, Map, Value};
use vkdg_connections::{ConnectionConfig, ProviderKind};
use vkdg_operations::{ContentBlock, ConversationRequest, MessageContent, Operation, Role};
use vkdg_provider_sdk::{
    OAuthConfig, OAuthFlow, OAuthProvider, PreparedRequest, ProviderAdapter, ProviderError,
    TokenPair,
};

pub struct GitHubCopilotAdapter;

impl ProviderAdapter for GitHubCopilotAdapter {
    fn id(&self) -> &str {
        "github-copilot"
    }

    fn display_name(&self) -> &str {
        "GitHub Copilot"
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

        // Note: production use requires fetching a short-lived copilot_token from
        //   /copilot_internal/v2/token first; Phase E will add that layer.
        let body = build_chat_completions_body(req, config, "gpt-4o");
        let url = format!("{}/v1/chat/completions", base_url(config));
        let headers = build_auth_headers(token);

        Ok(PreparedRequest {
            url,
            headers,
            body,
            is_streaming: req.stream,
        })
    }
}

impl OAuthProvider for GitHubCopilotAdapter {
    fn oauth_config(&self) -> OAuthConfig {
        OAuthConfig {
            flow: OAuthFlow::DeviceCode,
            authorize_url: None,
            token_url: "https://github.com/login/oauth/access_token".into(),
            client_id: std::env::var("GITHUB_COPILOT_CLIENT_ID").unwrap_or_default(),
            scopes: vec!["read:user".into()],
            redirect_uri: None,
            extra_auth_params: HashMap::new(),
        }
    }

    fn refresh_token<'a>(
        &'a self,
        _refresh_token: &'a str,
        _extra: &'a HashMap<String, String>,
    ) -> BoxFuture<'a, Result<TokenPair, ProviderError>> {
        Box::pin(async move {
            // GitHub device tokens are long-lived PATs; refresh is not supported.
            Err(ProviderError::TokenRefresh(
                "GitHub OAuth tokens do not support refresh; re-authenticate".into(),
            ))
        })
    }
}

fn base_url(config: &ConnectionConfig) -> String {
    match &config.provider {
        ProviderKind::Custom { base_url } => base_url.clone(),
        _ => "https://api.githubcopilot.com".into(),
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

fn build_chat_completions_body(
    req: &ConversationRequest,
    config: &ConnectionConfig,
    default_model: &str,
) -> Bytes {
    let model = config
        .models
        .first()
        .cloned()
        .unwrap_or_else(|| default_model.to_string());

    let mut messages: Vec<Value> = Vec::new();
    if let Some(sys) = &req.system {
        messages.push(json!({ "role": "system", "content": sys }));
    }

    for m in &req.messages {
        let msg = match m.role {
            Role::System => {
                let c = msg_content(&m.content);
                json!({ "role": "system", "content": c })
            }
            Role::User => {
                let c = msg_content(&m.content);
                json!({ "role": "user", "content": c })
            }
            Role::Assistant => {
                if let MessageContent::Blocks(blocks) = &m.content {
                    let tool_calls: Vec<Value> = blocks
                        .iter()
                        .filter_map(|b| match b {
                            ContentBlock::ToolUse { id, name, input } => {
                                let args =
                                    serde_json::to_string(input).unwrap_or_else(|_| "{}".into());
                                Some(json!({
                                    "id": id,
                                    "type": "function",
                                    "function": { "name": name, "arguments": args }
                                }))
                            }
                            _ => None,
                        })
                        .collect();
                    if !tool_calls.is_empty() {
                        json!({ "role": "assistant", "content": Value::Null, "tool_calls": tool_calls })
                    } else {
                        let c = msg_content(&m.content);
                        json!({ "role": "assistant", "content": c })
                    }
                } else {
                    let c = msg_content(&m.content);
                    json!({ "role": "assistant", "content": c })
                }
            }
            Role::Tool => {
                let (id, text) = extract_tool_result(&m.content);
                json!({ "role": "tool", "tool_call_id": id, "content": text })
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

fn msg_content(content: &MessageContent) -> Value {
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
                content: c,
            } = b
            {
                return (tool_use_id.clone(), c.clone());
            }
        }
    }
    let text = match content {
        MessageContent::Text(t) => t.clone(),
        _ => String::new(),
    };
    (String::new(), text)
}
