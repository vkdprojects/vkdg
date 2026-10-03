//! VKDG provider plugin: codex
//! `OpenAI` Responses API format over OAuth 2.0 (Authorization Code + PKCE).

use std::collections::HashMap;

use bytes::Bytes;
use futures::future::BoxFuture;
use http::HeaderMap;
use serde_json::{json, Map, Value};
use vkdg_connections::{ConnectionConfig, ProviderKind};
use vkdg_operations::{ContentBlock, ConversationRequest, MessageContent, Operation, Role};
use vkdg_provider_sdk::{
    Credential, OAuthConfig, OAuthFlow, OAuthProvider, PreparedRequest, ProviderAdapter,
    ProviderError, TokenPair,
};

pub struct CodexAdapter;

impl ProviderAdapter for CodexAdapter {
    fn oauth(&self) -> Option<&dyn OAuthProvider> {
        Some(self)
    }

    fn id(&self) -> &'static str {
        "codex"
    }

    fn display_name(&self) -> &'static str {
        "OpenAI Codex"
    }

    fn default_models(&self) -> Vec<String> {
        ["gpt-*", "o1-*", "o3-*", "o4-*", "codex-*"]
            .map(String::from)
            .into()
    }

    fn meta(&self) -> vkdg_provider_sdk::ProviderMeta {
        vkdg_provider_sdk::ProviderMeta {
            icon_char: 'C',
            icon_color: "#10a37f",
            category: vkdg_provider_sdk::ProviderCategory::OauthIde,
            site_url: Some("https://chatgpt.com"),
            description: Some("OpenAI Codex — coding agent via OAuth."),
        }
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

        let body = build_responses_body(req);
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

/// Converts a `ConversationRequest` into the Codex Responses API body shape.
/// Key differences from Chat Completions (which Codex /v1/responses rejects):
/// - `input` not `messages`; system messages stay in `input` as `developer` role
///   for prompt-cache eligibility (`instructions` is not cached on GPT-5 models)
/// - `instructions` holds the system prompt passed from `req.system`
/// - `store: false` (OAuth accounts: backend rejects `true`)
/// - `max_tokens` / `max_output_tokens` stripped (Codex rejects both)
/// - always `stream: true`; the Responses endpoint is SSE-first
fn build_responses_body(req: &ConversationRequest) -> Bytes {
    let model = vkdg_provider_sdk::upstream_model(req, "gpt-4o").to_owned();

    let mut input: Vec<Value> = Vec::new();
    for m in &req.messages {
        // system → developer keeps the message in `input` for prompt caching;
        // `instructions` is not part of GPT-5's cache key.
        let role = match m.role {
            Role::System => "developer",
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
        };
        let item = match &m.content {
            MessageContent::Text(text) => json!({
                "type": "message", "role": role,
                "content": [{ "type": "input_text", "text": text }]
            }),
            MessageContent::Blocks(blocks) => {
                let tool_calls: Vec<Value> = blocks
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::ToolUse { id, name, input } => {
                            let args = serde_json::to_string(input).unwrap_or_else(|_| "{}".into());
                            Some(json!({ "type": "function_call", "call_id": id,
                                     "name": name, "arguments": args }))
                        }
                        _ => None,
                    })
                    .collect();
                let tool_results: Vec<Value> = blocks
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::ToolResult {
                            tool_use_id,
                            content,
                            ..
                        } => Some(json!({
                            "type": "function_call_output",
                            "call_id": tool_use_id, "output": content
                        })),
                        _ => None,
                    })
                    .collect();
                if !tool_calls.is_empty() {
                    // Each function_call is a top-level item, not nested in a message.
                    for tc in &tool_calls {
                        input.push(tc.clone());
                    }
                    continue;
                } else if !tool_results.is_empty() {
                    for tr in &tool_results {
                        input.push(tr.clone());
                    }
                    continue;
                }
                let parts: Vec<Value> = blocks
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::Text { text } => {
                            Some(json!({ "type": "input_text", "text": text }))
                        }
                        _ => None,
                    })
                    .collect();
                json!({ "type": "message", "role": role, "content": parts })
            }
        };
        input.push(item);
    }

    let mut body = Map::new();
    body.insert("model".into(), Value::String(model));
    body.insert("input".into(), Value::Array(input));
    // Always set instructions; Codex Responses rejects requests without it.
    body.insert(
        "instructions".into(),
        Value::String(
            req.system
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or("Follow the developer instructions in the conversation.")
                .to_owned(),
        ),
    );
    body.insert("store".into(), Value::Bool(false));
    // Always stream; the /responses endpoint is SSE-first.
    body.insert("stream".into(), Value::Bool(true));
    // max_tokens and max_output_tokens are stripped: Codex rejects both.
    if let Some(temp) = req.temperature {
        body.insert("temperature".into(), json!(temp));
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use vkdg_connections::AuthKind;
    use vkdg_operations::{CapabilitySet, Message};

    fn simple(requested: &str, patterns: &[&str]) -> Value {
        let config = ConnectionConfig {
            id: vkdg_core::ConnectionId("codex-1".into()),
            provider: ProviderKind::Plugin { id: "codex".into() },
            auth: AuthKind::Account {
                account_id: "a".into(),
            },
            models: patterns.iter().map(|p| (*p).to_owned()).collect(),
            max_concurrent: 1,
            weight: 1,
            tags: vec![],
            endpoint: None,
            capabilities: CapabilitySet::default(),
        };
        let op = Operation::Conversation(ConversationRequest {
            model: requested.into(),
            messages: vec![Message {
                role: Role::User,
                content: MessageContent::Text("hi".into()),
            }],
            tools: vec![],
            max_tokens: None,
            temperature: None,
            stream: true,
            system: None,
            required_capabilities: CapabilitySet::default(),
            thinking: None,
            ..Default::default()
        });
        let cred = Credential {
            token: "t".into(),
            extra: Arc::new(HashMap::new()),
        };
        let req = CodexAdapter.prepare(&op, &config, &cred).unwrap();
        serde_json::from_slice(&req.body).unwrap()
    }

    #[test]
    fn upstream_model_is_the_requested_one_not_the_route_pattern() {
        assert_eq!(simple("gpt-5-codex", &["gpt-*"])["model"], "gpt-5-codex");
        assert_eq!(simple("o3", &["gpt-5", "o3"])["model"], "o3");
        assert_eq!(simple("", &["gpt-*"])["model"], "gpt-4o");
    }

    // Previously sent Chat Completions body; Responses API rejects messages/max_tokens.
    #[test]
    fn body_uses_responses_api_shape() {
        let b = simple("gpt-5-codex", &["gpt-*"]);
        assert!(b.get("input").is_some(), "must have input: {b}");
        assert!(b.get("messages").is_none(), "messages must not appear: {b}");
        assert!(
            b.get("instructions").is_some(),
            "instructions required: {b}"
        );
        assert_eq!(b["store"], false, "OAuth accounts must send store:false");
        assert!(
            b.get("max_tokens").is_none(),
            "Codex rejects max_tokens: {b}"
        );
        assert_eq!(b["stream"], true, "/responses always streams");
    }
}
