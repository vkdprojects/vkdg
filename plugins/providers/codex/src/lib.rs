//! VKDG provider plugin: codex
//! OpenAI Responses API format over OAuth 2.0 (Authorization Code + PKCE).

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

pub struct CodexAdapter;

impl ProviderAdapter for CodexAdapter {
    fn id(&self) -> &str {
        "codex"
    }

    fn display_name(&self) -> &str {
        "OpenAI Codex"
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

        let body = build_chat_completions_body(req, config, "gpt-4o");
        // Codex uses the OpenAI Responses API endpoint
        let url = format!("{}/v1/responses", base_url(config));
        let headers = build_auth_headers(token);

        Ok(PreparedRequest {
            url,
            headers,
            body,
            is_streaming: req.stream,
        })
    }
}

impl OAuthProvider for CodexAdapter {
    fn oauth_config(&self) -> OAuthConfig {
        let mut extra_auth_params = HashMap::new();
        extra_auth_params.insert("prompt".into(), "login".into());
        extra_auth_params.insert("codex_cli_simplified_flow".into(), "true".into());
        extra_auth_params.insert("originator".into(), "codex_cli_rs".into());

        OAuthConfig {
            flow: OAuthFlow::AuthorizationCodePkce,
            authorize_url: Some("https://auth.openai.com/oauth/authorize".into()),
            token_url: "https://auth.openai.com/oauth/token".into(),
            client_id: std::env::var("CODEX_OAUTH_CLIENT_ID").unwrap_or_default(),
            scopes: vec![
                "openid".into(),
                "profile".into(),
                "email".into(),
                "offline_access".into(),
            ],
            redirect_uri: Some("http://127.0.0.1:1455/auth/callback".into()),
            extra_auth_params,
        }
    }

    fn refresh_token<'a>(
        &'a self,
        refresh_token: &'a str,
        _extra: &'a HashMap<String, String>,
    ) -> BoxFuture<'a, Result<TokenPair, ProviderError>> {
        Box::pin(async move {
            let client_id = std::env::var("CODEX_OAUTH_CLIENT_ID").unwrap_or_default();
            let client = reqwest::Client::new();
            let resp = client
                .post("https://auth.openai.com/oauth/token")
                .json(&json!({
                    "grant_type": "refresh_token",
                    "refresh_token": refresh_token,
                    "client_id": client_id,
                }))
                .send()
                .await
                .map_err(|e| ProviderError::Http(e.to_string()))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(ProviderError::TokenRefresh(format!(
                    "HTTP {status}: {body}"
                )));
            }

            let json: Value = resp
                .json()
                .await
                .map_err(|e| ProviderError::Serialization(e.to_string()))?;

            Ok(TokenPair {
                access_token: json["access_token"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                refresh_token: json["refresh_token"].as_str().map(str::to_string),
                expires_in_secs: json["expires_in"].as_u64(),
                extra: HashMap::new(),
            })
        })
    }
}

fn base_url(config: &ConnectionConfig) -> String {
    match &config.provider {
        ProviderKind::Custom { base_url } => base_url.clone(),
        _ => "https://api.openai.com".into(),
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
