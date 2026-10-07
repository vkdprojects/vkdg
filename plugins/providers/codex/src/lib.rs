//! VKDG provider plugin: codex
//! `OpenAI` Responses API format over OAuth 2.0 (Authorization Code + PKCE).

use std::collections::HashMap;
use std::sync::Arc;

use bytes::Bytes;
use futures::future::BoxFuture;
use http::HeaderMap;
use serde_json::{json, Map, Value};
use vkdg_connections::{ConnectionConfig, ProviderKind};
use vkdg_operations::{ContentBlock, ConversationRequest, MessageContent, Operation, Role};
use vkdg_provider_sdk::{
    Credential, DynamicModelCatalog, ModelCatalog, OAuthConfig, OAuthFlow, OAuthProvider,
    PreparedRequest, ProviderAdapter, ProviderError, TokenPair,
};
/// Public OAuth `client_id` for the Codex CLI (`openai/codex`, `codex-rs`).
/// Source: `codex-rs/login/src/auth/manager.rs` — `pub const CLIENT_ID`.
/// Can be overridden via `CODEX_OAUTH_CLIENT_ID` env var.
const CODEX_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";

pub struct CodexAdapter {
    catalog: Option<Arc<DynamicModelCatalog>>,
}

impl CodexAdapter {
    pub fn new() -> Self {
        Self { catalog: None }
    }

    pub fn with_store(store: Arc<vkdg_config::GatewayStore>) -> Self {
        Self {
            catalog: Some(Arc::new(DynamicModelCatalog {
                store,
                provider_id: "codex",
            })),
        }
    }
}

impl Default for CodexAdapter {
    fn default() -> Self {
        Self::new()
    }
}

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
        let is_oauth = !matches!(config.auth, vkdg_connections::AuthKind::ApiKey { .. });
        let url = endpoint_url(config, is_oauth);
        let headers = build_auth_headers(token, credential);
        Ok(PreparedRequest {
            url,
            headers,
            body,
            is_streaming: req.stream,
        })
    }

    fn model_catalog(&self) -> Option<&dyn ModelCatalog> {
        self.catalog.as_deref().map(|c| c as &dyn ModelCatalog)
    }
}

impl OAuthProvider for CodexAdapter {
    fn login_methods(&self) -> Vec<vkdg_provider_sdk::LoginMethod> {
        use vkdg_provider_sdk::LoginMethod;
        vec![LoginMethod {
            id: "pkce".into(),
            label: "Sign in with OpenAI".into(),
            flow: vkdg_provider_sdk::OAuthFlow::AuthorizationCodePkce,
            hint: Some("Opens a popup to auth.openai.com — completes automatically.".into()),
            icon_char: Some('O'),
            fields: vec![],
        }]
    }

    fn oauth_config(&self) -> OAuthConfig {
        let mut extra_auth_params = HashMap::new();
        extra_auth_params.insert("prompt".into(), "login".into());
        extra_auth_params.insert("codex_cli_simplified_flow".into(), "true".into());
        extra_auth_params.insert("originator".into(), "codex_cli_rs".into());
        extra_auth_params.insert("id_token_add_organizations".into(), "true".into());

        OAuthConfig {
            flow: OAuthFlow::AuthorizationCodePkce,
            authorize_url: Some("https://auth.openai.com/oauth/authorize".into()),
            token_url: "https://auth.openai.com/oauth/token".into(),
            client_id: std::env::var("CODEX_OAUTH_CLIENT_ID")
                .unwrap_or_else(|_| CODEX_CLIENT_ID.into()),
            scopes: vec![
                "openid".into(),
                "profile".into(),
                "email".into(),
                "offline_access".into(),
                "model.request".into(),
                "model.read".into(),
            ],
            redirect_uri: Some("http://127.0.0.1:1455/auth/callback".into()),
            extra_auth_params,
        }
    }

    fn start_pkce_login<'a>(
        &'a self,
        _method: &'a str,
        params: &'a vkdg_provider_sdk::LoginParams,
    ) -> BoxFuture<'a, Result<vkdg_provider_sdk::PkceAuthorization, ProviderError>> {
        Box::pin(async move {
            let redirect_uri = match params.get("redirect_uri").filter(|u| !u.is_empty()) {
                Some(uri) => vkdg_provider_sdk::validated_loopback_redirect(uri)?,
                None => codex_redirect_uri(),
            };
            vkdg_provider_sdk::build_pkce_authorization(&self.oauth_config(), &redirect_uri)
        })
    }

    fn finish_pkce_login<'a>(
        &'a self,
        _method: &'a str,
        state: &'a vkdg_provider_sdk::LoginState,
        code: &'a str,
    ) -> BoxFuture<'a, Result<vkdg_provider_sdk::LoginResult, ProviderError>> {
        Box::pin(async move {
            let verifier = state.get("code_verifier").cloned().unwrap_or_default();
            let redirect_uri = state
                .get("redirect_uri")
                .cloned()
                .unwrap_or_else(codex_redirect_uri);
            // Accept either raw code or pasted callback URL.
            let (auth_code, _state) = vkdg_provider_sdk::parse_pkce_callback(code)?;
            finish_pkce_exchange(verifier, redirect_uri, auth_code).await
        })
    }

    fn refresh_token<'a>(
        &'a self,
        refresh_token: &'a str,
        _extra: &'a HashMap<String, String>,
    ) -> BoxFuture<'a, Result<TokenPair, ProviderError>> {
        Box::pin(async move {
            let client_id =
                std::env::var("CODEX_OAUTH_CLIENT_ID").unwrap_or_else(|_| CODEX_CLIENT_ID.into());
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

            let access_token = json["access_token"]
                .as_str()
                .unwrap_or_default()
                .to_string();

            let mut extra_map = HashMap::new();
            if let Some(account_id) = extract_chatgpt_account_id(&access_token) {
                extra_map.insert("chatgpt_account_id".into(), account_id);
            }

            Ok(TokenPair {
                access_token,
                refresh_token: json["refresh_token"].as_str().map(str::to_string),
                expires_in_secs: json["expires_in"].as_u64(),
                extra: extra_map,
            })
        })
    }
}
fn codex_redirect_uri() -> String {
    std::env::var("CODEX_REDIRECT_URI")
        .unwrap_or_else(|_| "http://127.0.0.1:1455/auth/callback".into())
}

async fn finish_pkce_exchange(
    verifier: String,
    redirect_uri: String,
    auth_code: String,
) -> Result<vkdg_provider_sdk::LoginResult, ProviderError> {
    let client_id =
        std::env::var("CODEX_OAUTH_CLIENT_ID").unwrap_or_else(|_| CODEX_CLIENT_ID.into());
    let client = reqwest::Client::new();
    let resp = client
        .post("https://auth.openai.com/oauth/token")
        .json(&json!({
            "code": auth_code,
            "grant_type": "authorization_code",
            "client_id": client_id,
            "redirect_uri": redirect_uri,
            "code_verifier": verifier,
        }))
        .send()
        .await
        .map_err(|e| ProviderError::Http(e.to_string()))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(ProviderError::TokenRefresh(format!(
            "token exchange HTTP {status}: {body}"
        )));
    }
    let tokens: Value = resp
        .json()
        .await
        .map_err(|e| ProviderError::Serialization(e.to_string()))?;

    let access_token = tokens["access_token"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let mut extra: HashMap<String, String> = HashMap::new();
    if let Some(account_id) = extract_chatgpt_account_id(&access_token) {
        extra.insert("chatgpt_account_id".into(), account_id);
    }

    // Best-effort: fetch userinfo from OpenAI.
    if let Ok(ui_resp) = client
        .get("https://auth.openai.com/oauth/userinfo")
        .header("Authorization", format!("Bearer {access_token}"))
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
    {
        if let Ok(ui) = ui_resp.json::<Value>().await {
            if let Some(email) = ui["email"].as_str() {
                extra.insert("email".into(), email.into());
            }
            if let Some(sub) = ui["sub"].as_str() {
                extra.insert("sub".into(), sub.into());
            }
            if let Some(account_id) = ui["chatgpt_account_id"].as_str() {
                extra.insert("chatgpt_account_id".into(), account_id.to_string());
            }
        }
    }

    let label = extra.get("email").cloned().unwrap_or_default();

    Ok(vkdg_provider_sdk::LoginResult {
        tokens: TokenPair {
            access_token,
            refresh_token: tokens["refresh_token"].as_str().map(str::to_string),
            expires_in_secs: tokens["expires_in"].as_u64(),
            extra,
        },
        label,
    })
}

pub const CHATGPT_CODEX_RESPONSES_URL: &str = "https://chatgpt.com/backend-api/codex/responses";
pub const OPENAI_RESPONSES_URL: &str = "https://api.openai.com/v1/responses";

fn endpoint_url(config: &ConnectionConfig, is_oauth: bool) -> String {
    match &config.provider {
        ProviderKind::Custom { base_url } => {
            if base_url.ends_with("/responses") {
                base_url.clone()
            } else {
                format!("{}/v1/responses", base_url.trim_end_matches('/'))
            }
        }
        _ => {
            if is_oauth {
                CHATGPT_CODEX_RESPONSES_URL.into()
            } else {
                OPENAI_RESPONSES_URL.into()
            }
        }
    }
}

pub fn extract_chatgpt_account_id(token: &str) -> Option<String> {
    let payload_b64 = token.split('.').nth(1)?;
    let mut padded = payload_b64.replace('-', "+").replace('_', "/");
    while padded.len() % 4 != 0 {
        padded.push('=');
    }
    let decoded = base64_decode(&padded)?;
    let claims: Value = serde_json::from_slice(&decoded).ok()?;

    if let Some(acc) = claims["chatgpt_account_id"].as_str() {
        return Some(acc.to_string());
    }
    if let Some(auth) = claims.get("https://api.openai.com/auth") {
        if let Some(acc) = auth["chatgpt_account_id"].as_str() {
            return Some(acc.to_string());
        }
    }
    None
}

fn base64_decode(input: &str) -> Option<Vec<u8>> {
    fn val(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            b'=' => Some(0),
            _ => None,
        }
    }
    let bytes = input.as_bytes();
    if bytes.len() % 4 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity((bytes.len() / 4) * 3);
    for chunk in bytes.chunks_exact(4) {
        let b0 = val(chunk[0])?;
        let b1 = val(chunk[1])?;
        let b2 = val(chunk[2])?;
        let b3 = val(chunk[3])?;
        let triple =
            (u32::from(b0) << 18) | (u32::from(b1) << 12) | (u32::from(b2) << 6) | u32::from(b3);
        #[allow(clippy::cast_possible_truncation)]
        out.push((triple >> 16) as u8);
        if chunk[2] != b'=' {
            #[allow(clippy::cast_possible_truncation)]
            out.push((triple >> 8) as u8);
        }
        if chunk[3] != b'=' {
            #[allow(clippy::cast_possible_truncation)]
            out.push(triple as u8);
        }
    }
    Some(out)
}

fn build_auth_headers(token: &str, credential: &Credential) -> HeaderMap {
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

    // Determine ChatGPT-Account-Id from credential extra or JWT claims
    let account_id = credential
        .extra
        .get("chatgpt_account_id")
        .cloned()
        .or_else(|| extract_chatgpt_account_id(token));

    if let Some(acc_id) = account_id {
        if let Ok(val) = acc_id.parse() {
            headers.insert(http::HeaderName::from_static("chatgpt-account-id"), val);
        }
    }

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
    // Not forwarded, on purpose: `tool_choice`, `stop_sequences`, `top_p` and
    // `disable_parallel_tool_use` (the Responses API has no `stop`; the others
    // are untested against this OAuth backend, and this adapter's wire behavior is
    // unchanged). `max_tokens` and `max_output_tokens` are stripped: Codex rejects both.
    if let Some(temp) = req.temperature {
        body.insert("temperature".into(), json!(temp));
    }
    if !req.tools.is_empty() {
        let tools: Vec<Value> = req
            .tools
            .iter()
            .map(|t| {
                let mut tool = Map::new();
                tool.insert("type".into(), Value::String("function".into()));
                tool.insert("name".into(), Value::String(t.name.clone()));
                if let Some(desc) = &t.description {
                    tool.insert("description".into(), Value::String(desc.clone()));
                }
                tool.insert("parameters".into(), t.input_schema.clone());
                Value::Object(tool)
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
        let req = CodexAdapter::new().prepare(&op, &config, &cred).unwrap();
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

    #[test]
    fn oauth_account_uses_backend_api_and_account_header() {
        let config = ConnectionConfig {
            id: vkdg_core::ConnectionId("codex-oauth".into()),
            provider: ProviderKind::Plugin { id: "codex".into() },
            auth: AuthKind::Account {
                account_id: "oauth-user".into(),
            },
            models: vec!["gpt-*".into()],
            max_concurrent: 1,
            weight: 1,
            tags: vec![],
            endpoint: None,
            capabilities: CapabilitySet::default(),
        };
        let op = Operation::Conversation(ConversationRequest {
            model: "gpt-5-codex".into(),
            messages: vec![Message {
                role: Role::User,
                content: MessageContent::Text("hello".into()),
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
        let mut extra = HashMap::new();
        extra.insert("chatgpt_account_id".into(), "acc_test123".into());
        let cred = Credential {
            token: "valid-token".into(),
            extra: Arc::new(extra),
        };
        let req = CodexAdapter::new().prepare(&op, &config, &cred).unwrap();
        assert_eq!(req.url, CHATGPT_CODEX_RESPONSES_URL);
        assert_eq!(
            req.headers
                .get("chatgpt-account-id")
                .map(|v| v.to_str().unwrap()),
            Some("acc_test123")
        );
    }

    #[test]
    fn api_key_account_uses_openai_responses_url() {
        let config = ConnectionConfig {
            id: vkdg_core::ConnectionId("codex-apikey".into()),
            provider: ProviderKind::Plugin { id: "codex".into() },
            auth: AuthKind::ApiKey {
                env_var: "OPENAI_API_KEY".into(),
            },
            models: vec!["gpt-*".into()],
            max_concurrent: 1,
            weight: 1,
            tags: vec![],
            endpoint: None,
            capabilities: CapabilitySet::default(),
        };
        let op = Operation::Conversation(ConversationRequest {
            model: "gpt-5-codex".into(),
            messages: vec![Message {
                role: Role::User,
                content: MessageContent::Text("hello".into()),
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
            token: "sk-mock".into(),
            extra: Arc::new(HashMap::new()),
        };
        let req = CodexAdapter::new().prepare(&op, &config, &cred).unwrap();
        assert_eq!(req.url, OPENAI_RESPONSES_URL);
        assert!(req.headers.get("chatgpt-account-id").is_none());
    }

    #[test]
    fn tools_serialized_with_flat_function_schema() {
        let config = ConnectionConfig {
            id: vkdg_core::ConnectionId("codex-tools".into()),
            provider: ProviderKind::Plugin { id: "codex".into() },
            auth: AuthKind::ApiKey {
                env_var: "OPENAI_API_KEY".into(),
            },
            models: vec!["gpt-*".into()],
            max_concurrent: 1,
            weight: 1,
            tags: vec![],
            endpoint: None,
            capabilities: CapabilitySet::default(),
        };
        let op = Operation::Conversation(ConversationRequest {
            model: "gpt-5-codex".into(),
            messages: vec![Message {
                role: Role::User,
                content: MessageContent::Text("what's the weather?".into()),
            }],
            tools: vec![vkdg_operations::Tool {
                name: "get_weather".into(),
                description: Some("Get current weather".into()),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "location": { "type": "string" }
                    },
                    "required": ["location"]
                }),
            }],
            max_tokens: None,
            temperature: None,
            stream: true,
            system: None,
            required_capabilities: CapabilitySet::default(),
            thinking: None,
            ..Default::default()
        });
        let cred = Credential {
            token: "sk-mock".into(),
            extra: Arc::new(HashMap::new()),
        };
        let req = CodexAdapter::new().prepare(&op, &config, &cred).unwrap();
        let body: Value = serde_json::from_slice(&req.body).unwrap();
        let tools = body.get("tools").and_then(Value::as_array).unwrap();
        assert_eq!(tools.len(), 1);
        let tool = &tools[0];
        assert_eq!(tool["type"], "function");
        assert_eq!(tool["name"], "get_weather");
        assert_eq!(tool["description"], "Get current weather");
        assert_eq!(tool["parameters"]["type"], "object");
        assert!(tool.get("function").is_none(), "Codex Responses API tools must not nest under 'function'");
    }

    #[test]
    fn tool_calls_and_results_serialized_in_input() {
        let config = ConnectionConfig {
            id: vkdg_core::ConnectionId("codex-tools".into()),
            provider: ProviderKind::Plugin { id: "codex".into() },
            auth: AuthKind::ApiKey {
                env_var: "OPENAI_API_KEY".into(),
            },
            models: vec!["gpt-*".into()],
            max_concurrent: 1,
            weight: 1,
            tags: vec![],
            endpoint: None,
            capabilities: CapabilitySet::default(),
        };
        let op = Operation::Conversation(ConversationRequest {
            model: "gpt-5-codex".into(),
            messages: vec![
                Message {
                    role: Role::User,
                    content: MessageContent::Text("what's the weather?".into()),
                },
                Message {
                    role: Role::Assistant,
                    content: MessageContent::Blocks(vec![ContentBlock::ToolUse {
                        id: "call_123".into(),
                        name: "get_weather".into(),
                        input: json!({ "location": "San Francisco" }),
                    }]),
                },
                Message {
                    role: Role::Tool,
                    content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                        tool_use_id: "call_123".into(),
                        content: "{\"temp\": 65}".into(),
                        is_error: false,
                        images: vec![],
                    }]),
                },
            ],
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
            token: "sk-mock".into(),
            extra: Arc::new(HashMap::new()),
        };
        let req = CodexAdapter::new().prepare(&op, &config, &cred).unwrap();
        let body: Value = serde_json::from_slice(&req.body).unwrap();
        let input = body.get("input").and_then(Value::as_array).unwrap();
        assert_eq!(input.len(), 3);
        assert_eq!(input[0]["type"], "message");
        assert_eq!(input[0]["role"], "user");
        assert_eq!(input[1]["type"], "function_call");
        assert_eq!(input[1]["call_id"], "call_123");
        assert_eq!(input[1]["name"], "get_weather");
        assert_eq!(input[1]["arguments"], "{\"location\":\"San Francisco\"}");
        assert_eq!(input[2]["type"], "function_call_output");
        assert_eq!(input[2]["call_id"], "call_123");
        assert_eq!(input[2]["output"], "{\"temp\": 65}");
    }
}
