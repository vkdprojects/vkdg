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
    Credential, OAuthConfig, OAuthFlow, OAuthProvider, PreparedRequest, ProviderAdapter,
    ProviderError, TokenPair,
};

pub struct ClaudeCodeAdapter;

impl ProviderAdapter for ClaudeCodeAdapter {
    fn oauth(&self) -> Option<&dyn OAuthProvider> {
        Some(self)
    }

    fn id(&self) -> &'static str {
        "claude-code"
    }

    fn display_name(&self) -> &'static str {
        "Claude Code"
    }

    fn meta(&self) -> vkdg_provider_sdk::ProviderMeta {
        vkdg_provider_sdk::ProviderMeta {
            icon_char: 'C',
            icon_color: "#CC785C",
            category: vkdg_provider_sdk::ProviderCategory::OauthIde,
            site_url: Some("https://claude.ai"),
            description: Some("Anthropic Claude Code — OAuth coding agent."),
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

        let body = build_body(req);
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

/// Public OAuth `client_id` for the Claude Code CLI.
/// This is a public value — the same one the official `Claude Code CLI` ships in
/// its binary and that the Anthropic `OAuth` flow is designed to accept from any
/// `PKCE` public client. Source: `OmniRoute` `open-sse/utils/publicCreds.ts`.
const CLAUDE_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const CLAUDE_REDIRECT_URI_DEFAULT: &str = "https://platform.claude.com/oauth/code/callback";

fn claude_redirect_uri() -> String {
    std::env::var("CLAUDE_CODE_REDIRECT_URI").unwrap_or_else(|_| CLAUDE_REDIRECT_URI_DEFAULT.into())
}
const CLAUDE_TOKEN_URL: &str = "https://api.anthropic.com/v1/oauth/token";
const CLAUDE_AUTHORIZE_URL: &str = "https://claude.ai/oauth/authorize";
const CLAUDE_BOOTSTRAP_URL: &str = "https://api.anthropic.com/api/claude_cli/bootstrap";

impl OAuthProvider for ClaudeCodeAdapter {
    fn login_methods(&self) -> Vec<vkdg_provider_sdk::LoginMethod> {
        use vkdg_provider_sdk::LoginMethod;
        vec![LoginMethod {
            id: "pkce".into(),
            label: "Sign in with Claude".into(),
            flow: vkdg_provider_sdk::OAuthFlow::AuthorizationCodePkce,
            hint: Some("Opens a popup to claude.ai — completes automatically.".into()),
            icon_char: Some('C'),
            fields: vec![],
        }]
    }

    fn oauth_config(&self) -> OAuthConfig {
        let mut extra_auth_params = HashMap::new();
        // prompt=login forces re-authentication on every login so that
        // multi-account setups never silently reuse an existing session.
        extra_auth_params.insert("prompt".into(), "login".into());
        OAuthConfig {
            flow: OAuthFlow::AuthorizationCodePkce,
            authorize_url: Some(CLAUDE_AUTHORIZE_URL.into()),
            token_url: CLAUDE_TOKEN_URL.into(),
            client_id: std::env::var("CLAUDE_OAUTH_CLIENT_ID")
                .unwrap_or_else(|_| CLAUDE_CLIENT_ID.into()),
            scopes: vec![
                "org:create_api_key".into(),
                "user:profile".into(),
                "user:inference".into(),
                "user:sessions:claude_code".into(),
                "user:mcp_servers".into(),
            ],
            redirect_uri: Some(claude_redirect_uri()),
            extra_auth_params,
        }
    }

    fn start_pkce_login<'a>(
        &'a self,
        _method: &'a str,
        _params: &'a vkdg_provider_sdk::LoginParams,
    ) -> BoxFuture<'a, Result<vkdg_provider_sdk::PkceAuthorization, ProviderError>> {
        Box::pin(async {
            use base64::Engine as _;
            use rand::RngCore;
            use sha2::{Digest, Sha256};

            // Generate a cryptographically random code verifier (RFC 7636 §4.1).
            let mut verifier_bytes = [0u8; 32];
            rand::thread_rng().fill_bytes(&mut verifier_bytes);
            let verifier = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(verifier_bytes);

            // S256 challenge = BASE64URL(SHA256(verifier)).
            let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(Sha256::digest(verifier.as_bytes()));

            // OAuth `state` — random anti-CSRF nonce.
            let mut state_bytes = [0u8; 16];
            rand::thread_rng().fill_bytes(&mut state_bytes);
            let state = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(state_bytes);

            let client_id =
                std::env::var("CLAUDE_OAUTH_CLIENT_ID").unwrap_or_else(|_| CLAUDE_CLIENT_ID.into());

            let scopes = "org:create_api_key user:profile user:inference user:sessions:claude_code user:mcp_servers";

            let mut url = reqwest::Url::parse(CLAUDE_AUTHORIZE_URL)
                .map_err(|e| ProviderError::Http(e.to_string()))?;
            url.query_pairs_mut()
                .append_pair("code", "true")
                .append_pair("client_id", &client_id)
                .append_pair("response_type", "code")
                .append_pair("redirect_uri", &claude_redirect_uri())
                .append_pair("scope", scopes)
                .append_pair("code_challenge", &challenge)
                .append_pair("code_challenge_method", "S256")
                .append_pair("state", &state)
                .append_pair("prompt", "login");

            let mut login_state = HashMap::new();
            login_state.insert("code_verifier".into(), verifier);
            login_state.insert("state".into(), state);

            Ok(vkdg_provider_sdk::PkceAuthorization {
                authorize_url: url.to_string(),
                state: login_state,
            })
        })
    }

    fn finish_pkce_login<'a>(
        &'a self,
        _method: &'a str,
        state: &'a vkdg_provider_sdk::LoginState,
        code: &'a str,
    ) -> BoxFuture<'a, Result<vkdg_provider_sdk::LoginResult, ProviderError>> {
        Box::pin(async move {
            let verifier = state
                .get("code_verifier")
                .map(String::as_str)
                .unwrap_or_default();
            let original_state = state.get("state").map(String::as_str).unwrap_or_default();

            // OmniRoute: the user may paste the full callback URL; strip to code+state.
            let (auth_code, code_state) = if code.contains('#') {
                let parts: Vec<&str> = code.splitn(2, '#').collect();
                (parts[0], parts.get(1).copied().unwrap_or(""))
            } else if code.starts_with("https://") {
                // Full callback URL: parse code from query string.
                let parsed =
                    reqwest::Url::parse(code).map_err(|e| ProviderError::Http(e.to_string()))?;
                let pairs: HashMap<_, _> = parsed.query_pairs().into_owned().collect();
                let c = pairs
                    .get("code")
                    .cloned()
                    .unwrap_or_else(|| code.to_owned());
                let s = pairs.get("state").cloned().unwrap_or_default();
                return finish_exchange(verifier.to_owned(), original_state.to_owned(), c, s).await;
            } else {
                (code, "")
            };

            finish_exchange(
                verifier.to_owned(),
                original_state.to_owned(),
                auth_code.to_owned(),
                code_state.to_owned(),
            )
            .await
        })
    }

    fn refresh_token<'a>(
        &'a self,
        refresh_token: &'a str,
        _extra: &'a HashMap<String, String>,
    ) -> BoxFuture<'a, Result<TokenPair, ProviderError>> {
        Box::pin(async move {
            let client_id =
                std::env::var("CLAUDE_OAUTH_CLIENT_ID").unwrap_or_else(|_| CLAUDE_CLIENT_ID.into());
            let client = reqwest::Client::new();
            let resp = client
                .post(CLAUDE_TOKEN_URL)
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

/// Exchange an authorization code for tokens, then enrich with bootstrap data.
async fn finish_exchange(
    verifier: String,
    original_state: String,
    auth_code: String,
    code_state: String,
) -> Result<vkdg_provider_sdk::LoginResult, ProviderError> {
    let client_id =
        std::env::var("CLAUDE_OAUTH_CLIENT_ID").unwrap_or_else(|_| CLAUDE_CLIENT_ID.into());
    let effective_state = if code_state.is_empty() {
        &original_state
    } else {
        &code_state
    };

    let client = reqwest::Client::new();
    let resp = client
        .post(CLAUDE_TOKEN_URL)
        .json(&json!({
            "code": auth_code,
            "state": effective_state,
            "grant_type": "authorization_code",
            "client_id": client_id,
            "redirect_uri": claude_redirect_uri(),
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

    // Post-exchange: fetch bootstrap data (email, org) — best-effort.
    let mut extra: HashMap<String, String> = HashMap::new();
    if let Ok(bs_resp) = client
        .get(CLAUDE_BOOTSTRAP_URL)
        .header("Authorization", format!("Bearer {access_token}"))
        .header("anthropic-beta", "oauth-2025-04-20")
        .header("User-Agent", "claude-cli/2.1.280.096 (external, cli)")
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
    {
        if let Ok(bs) = bs_resp.json::<Value>().await {
            let acct = &bs["oauth_account"];
            if let Some(email) = acct["account_email"].as_str() {
                extra.insert("account_email".into(), email.into());
            }
            if let Some(uuid) = acct["account_uuid"].as_str() {
                extra.insert("account_uuid".into(), uuid.into());
            }
            if let Some(org) = acct["organization_uuid"].as_str() {
                extra.insert("organization_uuid".into(), org.into());
            }
            if let Some(org_name) = acct["organization_name"].as_str() {
                extra.insert("organization_name".into(), org_name.into());
            }
        }
    }

    let label = extra.get("account_email").cloned().unwrap_or_default();

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

fn base_url(config: &ConnectionConfig) -> String {
    match &config.provider {
        ProviderKind::Custom { base_url } => base_url.clone(),
        _ => "https://api.anthropic.com".into(),
    }
}

fn build_body(req: &ConversationRequest) -> Bytes {
    let model = vkdg_provider_sdk::upstream_model(req, "claude-opus-4-5").to_owned();

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
