//! VKDG provider plugin: claude-code
//! Anthropic Messages format over OAuth 2.0 (Authorization Code + PKCE).

use std::collections::HashMap;

use bytes::Bytes;
use futures::future::BoxFuture;
use http::HeaderMap;
use serde_json::{json, Map, Value};
use vkdg_connections::{ConnectionConfig, ProviderKind};
use vkdg_operations::{ConversationRequest, MessageContent, Operation, Role};
use vkdg_provider_sdk::{
    OAuthConfig, OAuthFlow, OAuthProvider, PreparedRequest, ProviderAdapter, ProviderError,
    TokenPair,
};

pub struct ClaudeCodeAdapter;

impl ProviderAdapter for ClaudeCodeAdapter {
    fn id(&self) -> &str {
        "claude-code"
    }

    fn display_name(&self) -> &str {
        "Claude Code"
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

        let body = build_body(req, config);
        let url = format!("{}/v1/messages", base_url(config));

        let mut headers = HeaderMap::new();
        headers.insert(
            http::header::AUTHORIZATION,
            format!("Bearer {token}")
                .parse()
                .unwrap_or_else(|_| http::HeaderValue::from_static("invalid")),
        );
        headers.insert(
            "anthropic-version",
            http::HeaderValue::from_static("2023-06-01"),
        );
        headers.insert(
            "anthropic-beta",
            http::HeaderValue::from_static("oauth-2025-04-20"),
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

impl OAuthProvider for ClaudeCodeAdapter {
    fn oauth_config(&self) -> OAuthConfig {
        let mut extra_auth_params = HashMap::new();
        extra_auth_params.insert("prompt".into(), "login".into());

        OAuthConfig {
            flow: OAuthFlow::AuthorizationCodePkce,
            authorize_url: Some("https://claude.ai/oauth/authorize".into()),
            token_url: "https://api.anthropic.com/v1/oauth/token".into(),
            client_id: std::env::var("CLAUDE_OAUTH_CLIENT_ID").unwrap_or_default(),
            scopes: vec![
                "org:create_api_key".into(),
                "user:profile".into(),
                "user:inference".into(),
                "user:sessions:claude_code".into(),
            ],
            redirect_uri: Some("https://platform.claude.com/oauth/code/callback".into()),
            extra_auth_params,
        }
    }

    fn refresh_token<'a>(
        &'a self,
        refresh_token: &'a str,
        _extra: &'a HashMap<String, String>,
    ) -> BoxFuture<'a, Result<TokenPair, ProviderError>> {
        Box::pin(async move {
            let client_id = std::env::var("CLAUDE_OAUTH_CLIENT_ID").unwrap_or_default();
            let client = reqwest::Client::new();
            let resp = client
                .post("https://api.anthropic.com/v1/oauth/token")
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
        _ => "https://api.anthropic.com".into(),
    }
}

fn build_body(req: &ConversationRequest, config: &ConnectionConfig) -> Bytes {
    let model = config
        .models
        .first()
        .cloned()
        .unwrap_or_else(|| "claude-opus-4-5".into());

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
