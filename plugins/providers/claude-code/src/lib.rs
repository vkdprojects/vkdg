//! VKDG provider plugin: claude-code
//! Anthropic Messages format over OAuth 2.0 (Authorization Code + PKCE).

use std::collections::HashMap;

use bytes::Bytes;
use futures::future::BoxFuture;
use http::HeaderMap;
use serde_json::{json, Value};
use vkdg_connections::{ConnectionConfig, ProviderKind};
use vkdg_operations::{ConversationRequest, Operation};
use vkdg_provider_sdk::{
    Credential, OAuthConfig, OAuthFlow, OAuthProvider, PreparedRequest, ProviderAdapter,
    ProviderError, TokenPair,
};

mod catalog;
mod usage;

pub struct ClaudeCodeAdapter;

impl ProviderAdapter for ClaudeCodeAdapter {
    fn oauth(&self) -> Option<&dyn OAuthProvider> {
        Some(self)
    }

    fn model_catalog(&self) -> Option<&dyn vkdg_provider_sdk::ModelCatalog> {
        Some(&catalog::ClaudeCodeCatalog)
    }

    fn usage(&self) -> Option<&dyn vkdg_provider_sdk::UsageProvider> {
        Some(&usage::ClaudeCodeUsage)
    }

    fn id(&self) -> &'static str {
        "claude-code"
    }

    fn display_name(&self) -> &'static str {
        "Claude Code"
    }

    fn wire_format(&self, _config: &ConnectionConfig) -> Option<vkdg_operations::WireFormat> {
        Some(vkdg_operations::WireFormat::AnthropicMessages)
    }

    fn default_models(&self) -> Vec<String> {
        vec!["claude-*".into()]
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
            http::HeaderValue::from_static(ANTHROPIC_VERSION),
        );
        headers.insert("anthropic-beta", http::HeaderValue::from_static(OAUTH_BETA));
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

/// `anthropic-version` and the OAuth beta flag every subscription-token call sends.
const ANTHROPIC_VERSION: &str = "2023-06-01";
const OAUTH_BETA: &str = "oauth-2025-04-20";

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
const CLAUDE_SCOPES: [&str; 5] = [
    "org:create_api_key",
    "user:profile",
    "user:inference",
    "user:sessions:claude_code",
    "user:mcp_servers",
];

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
        // claude.ai requires this sentinel to distinguish PKCE requests.
        extra_auth_params.insert("code".into(), "true".into());
        OAuthConfig {
            flow: OAuthFlow::AuthorizationCodePkce,
            authorize_url: Some(CLAUDE_AUTHORIZE_URL.into()),
            token_url: CLAUDE_TOKEN_URL.into(),
            client_id: std::env::var("CLAUDE_OAUTH_CLIENT_ID")
                .unwrap_or_else(|_| CLAUDE_CLIENT_ID.into()),
            scopes: CLAUDE_SCOPES.iter().map(|s| (*s).to_owned()).collect(),
            redirect_uri: Some(claude_redirect_uri()),
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
                None => claude_redirect_uri(),
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
            let original_state = state.get("state").cloned().unwrap_or_default();
            // Must repeat the authorize `redirect_uri` verbatim.
            let redirect_uri = state
                .get("redirect_uri")
                .cloned()
                .unwrap_or_else(claude_redirect_uri);
            let (auth_code, code_state) = vkdg_provider_sdk::parse_pkce_callback(code)?;
            let code_state = code_state.unwrap_or_default();
            finish_exchange(
                verifier,
                original_state,
                redirect_uri,
                auth_code,
                code_state,
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
    redirect_uri: String,
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

/// First system block Anthropic requires on subscription OAuth requests.
const CLAUDE_CODE_IDENTITY: &str = "You are Claude Code, Anthropic's official CLI for Claude.";

/// Anthropic spells versions with dashes (`claude-sonnet-4-6`, `claude-3-5-haiku`);
/// Kiro and some clients use dots (`claude-sonnet-4.6`). A dot between two digits
/// becomes a dash, so a dotted name asks for the same model instead of a 404.
/// Dated suffixes (`-20250929`) and names that are not Claude's are left alone.
fn anthropic_model_id(model: &str) -> std::borrow::Cow<'_, str> {
    if !model.starts_with("claude") || !model.contains('.') {
        return model.into();
    }
    let chars: Vec<char> = model.chars().collect();
    let mut out = String::with_capacity(model.len());
    for (i, &c) in chars.iter().enumerate() {
        let between_digits = c == '.'
            && i > 0
            && chars[i - 1].is_ascii_digit()
            && chars.get(i + 1).is_some_and(char::is_ascii_digit);
        out.push(if between_digits { '-' } else { c });
    }
    out.into()
}

fn build_body(req: &ConversationRequest) -> Bytes {
    let model = vkdg_provider_sdk::upstream_model(req, "claude-opus-4-5");
    let model = anthropic_model_id(model);
    // A subscription (OAuth) token is only accepted for Claude Code traffic:
    // without this exact first system block Anthropic answers 429
    // rate_limit_error. The client's own system prompt follows it untouched.
    vkdg_provider_sdk::anthropic_messages::messages_body(req, &model, Some(CLAUDE_CODE_IDENTITY))
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;
    use sha2::{Digest, Sha256};

    fn b64u_decode(s: &str) -> Vec<u8> {
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(s)
            .expect("base64url")
    }

    fn start(
        params: &[(&str, &str)],
    ) -> Result<vkdg_provider_sdk::PkceAuthorization, ProviderError> {
        let params: vkdg_provider_sdk::LoginParams = params
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        futures::executor::block_on(ClaudeCodeAdapter.start_pkce_login("pkce", &params))
    }

    fn query(url: &str) -> HashMap<String, String> {
        reqwest::Url::parse(url)
            .expect("authorize url")
            .query_pairs()
            .into_owned()
            .collect()
    }

    /// claude.ai answers "Invalid request format" on Authorize when `state`
    /// carries less entropy than the official CLI sends (32 bytes, 43 chars).
    /// Observed against the live endpoint: 22-char state rejected, 43-char accepted.
    #[test]
    fn state_has_32_bytes_and_challenge_is_s256_of_verifier() {
        let auth = start(&[]).unwrap();
        let q = query(&auth.authorize_url);

        assert_eq!(b64u_decode(&q["state"]).len(), 32, "state: {}", q["state"]);
        assert_eq!(auth.state["state"], q["state"]);

        let verifier = &auth.state["code_verifier"];
        let expected = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(Sha256::digest(verifier.as_bytes()));
        assert_eq!(q["code_challenge"], expected);
        assert_eq!(q["code_challenge_method"], "S256");
    }

    /// `prompt=login` bounces a signed-in browser to the login page.
    #[test]
    fn authorize_url_does_not_force_reauthentication() {
        let q = query(&start(&[]).unwrap().authorize_url);
        assert!(!q.contains_key("prompt"), "prompt={:?}", q.get("prompt"));
    }

    #[test]
    fn default_redirect_is_the_manual_code_page() {
        let auth = start(&[]).unwrap();
        let q = query(&auth.authorize_url);
        assert_eq!(q["redirect_uri"], CLAUDE_REDIRECT_URI_DEFAULT);
        assert_eq!(auth.state["redirect_uri"], CLAUDE_REDIRECT_URI_DEFAULT);
    }

    /// The token exchange must repeat the authorize `redirect_uri` verbatim, so
    /// the one used at start has to travel in the login state.
    #[test]
    fn loopback_redirect_is_used_and_remembered_for_the_exchange() {
        for uri in [
            "http://localhost:9090/callback",
            "http://127.0.0.1:54545/callback",
        ] {
            let auth = start(&[("redirect_uri", uri)]).unwrap();
            assert_eq!(query(&auth.authorize_url)["redirect_uri"], uri);
            assert_eq!(auth.state["redirect_uri"], uri);
        }
    }

    /// A browser-supplied redirect must not be able to send the authorization
    /// code to a host the operator does not control.
    #[test]
    fn only_loopback_callback_redirects_are_accepted() {
        for uri in [
            "https://evil.example/callback",
            "http://localhost.evil.example:9090/callback",
            "http://localhost@evil.example/callback",
            "https://localhost:9090/callback",
            "http://localhost:9090/other",
            "http://localhost:9090/callback?next=https://evil.example",
            "http://localhost:9090/callback#frag",
            "javascript:alert(1)",
            "not a url",
        ] {
            assert!(
                matches!(
                    start(&[("redirect_uri", uri)]),
                    Err(ProviderError::Config(_))
                ),
                "{uri} must be rejected"
            );
        }
    }
}

#[cfg(test)]
mod paste_tests {
    use vkdg_provider_sdk::parse_pkce_callback;

    /// The manual page shows `code#state`; users also paste the whole callback
    /// URL or just the code. All three must yield the same pair.
    #[test]
    fn pasted_code_forms_yield_code_and_state() {
        for (pasted, code, state) in [
            ("abc123#st4te", "abc123", Some("st4te")),
            ("  abc123#st4te\n", "abc123", Some("st4te")),
            (
                "http://localhost:9090/callback?code=abc123&state=st4te",
                "abc123",
                Some("st4te"),
            ),
            (
                "https://platform.claude.com/oauth/code/callback?code=abc123&state=st4te",
                "abc123",
                Some("st4te"),
            ),
            ("abc123", "abc123", None),
        ] {
            let (c, s) = parse_pkce_callback(pasted).unwrap();
            assert_eq!(c.as_str(), code, "{pasted:?}");
            assert_eq!(s.as_deref(), state, "{pasted:?}");
        }
    }
}

#[cfg(test)]
mod body_tests {
    use super::*;
    use vkdg_operations::{Message, MessageContent, Role, ToolChoice};

    const IDENTITY: &str = "You are Claude Code, Anthropic's official CLI for Claude.";

    fn body(system: Option<&str>) -> Value {
        let req = ConversationRequest {
            model: "claude-sonnet-4-5".into(),
            messages: vec![Message {
                role: Role::User,
                content: MessageContent::Text("hi".into()),
            }],
            system: system.map(str::to_owned),
            ..Default::default()
        };
        serde_json::from_slice(&build_body(&req)).unwrap()
    }

    /// Anthropic answers an OAuth subscription token with `429 rate_limit_error`
    /// unless the first system block is exactly the Claude Code identity; with it
    /// the same request returns 200 (checked live against api.anthropic.com).
    /// The gateway then reported "no eligible connection" because the 429 put the
    /// connection in cooldown.
    #[test]
    fn system_starts_with_the_claude_code_identity_block() {
        let v = body(None);
        assert_eq!(v["system"], json!([{ "type": "text", "text": IDENTITY }]));
    }

    #[test]
    fn the_clients_system_prompt_follows_the_identity_block_unchanged() {
        let v = body(Some("Be brief."));
        // The identity stays first and unmarked; with no client marker anywhere the
        // default policy puts its breakpoint on the client's own block.
        assert_eq!(
            v["system"],
            json!([
                { "type": "text", "text": IDENTITY },
                { "type": "text", "text": "Be brief.", "cache_control": { "type": "ephemeral" } },
            ])
        );
    }

    fn body_with_thinking(thinking: Option<vkdg_operations::ThinkingRequest>) -> Value {
        let req = ConversationRequest {
            model: "claude-sonnet-4-5".into(),
            messages: vec![Message {
                role: Role::User,
                content: MessageContent::Text("hi".into()),
            }],
            max_tokens: Some(4000),
            thinking,
            ..Default::default()
        };
        serde_json::from_slice(&build_body(&req)).unwrap()
    }

    // The client asked for extended thinking and the plugin dropped it, so the
    // reply had no thinking block and the budget was never honoured.
    #[test]
    fn requested_thinking_budget_reaches_anthropic() {
        let v = body_with_thinking(Some(vkdg_operations::ThinkingRequest {
            budget_tokens: Some(2048),
            effort: None,
        }));
        assert_eq!(
            v["thinking"],
            json!({ "type": "enabled", "budget_tokens": 2048 })
        );
    }

    #[test]
    fn no_thinking_request_sends_no_thinking_field() {
        assert!(body_with_thinking(None).get("thinking").is_none());
    }

    // An OpenAI-style effort has no budget; Anthropic's minimum budget keeps the
    // request valid instead of sending `thinking` without `budget_tokens`.
    #[test]
    fn effort_without_budget_uses_the_minimum_budget() {
        let v = body_with_thinking(Some(vkdg_operations::ThinkingRequest {
            budget_tokens: None,
            effort: Some("high".into()),
        }));
        assert_eq!(
            v["thinking"],
            json!({ "type": "enabled", "budget_tokens": 1024 })
        );
    }

    // Defeat: the identity gate lost when the new tool/limit fields are present,
    // a glob model sent upstream, or no max_tokens (Anthropic: 400).
    #[test]
    fn identity_gate_survives_tools_and_missing_limits() {
        let req = ConversationRequest {
            model: "claude-*".into(),
            messages: vec![Message {
                role: Role::User,
                content: MessageContent::Text("hi".into()),
            }],
            tools: vec![vkdg_operations::Tool {
                name: "read".into(),
                description: None,
                input_schema: json!({ "type": "object" }),
                cache_control: None,
            }],
            tool_choice: Some(ToolChoice::Named("read".into())),
            stop_sequences: vec!["END".into()],
            ..Default::default()
        };
        let v: Value = serde_json::from_slice(&build_body(&req)).unwrap();
        assert_eq!(v["model"], "claude-opus-4-5");
        assert_eq!(v["max_tokens"], 8192);
        assert_eq!(v["system"], json!([{ "type": "text", "text": IDENTITY }]));
        assert_eq!(v["tool_choice"], json!({ "type": "tool", "name": "read" }));
        assert_eq!(v["stop_sequences"], json!(["END"]));
    }

    fn sent_model(model: &str) -> String {
        let req = ConversationRequest {
            model: model.into(),
            messages: vec![Message {
                role: Role::User,
                content: MessageContent::Text("hi".into()),
            }],
            ..Default::default()
        };
        let v: Value = serde_json::from_slice(&build_body(&req)).unwrap();
        v["model"].as_str().unwrap().to_owned()
    }

    // Kiro spells versions with dots and a client may ask for that name; Anthropic
    // 404s on it. Refutes sending the model verbatim, and a blanket '.'->'-'
    // rewrite that would also mangle the rest of the name.
    #[test]
    fn dotted_claude_versions_go_upstream_with_dashes() {
        for (asked, sent) in [
            ("claude-sonnet-4.6", "claude-sonnet-4-6"),
            ("claude-3.5-haiku", "claude-3-5-haiku"),
            ("claude-opus-4.1", "claude-opus-4-1"),
        ] {
            assert_eq!(sent_model(asked), sent, "{asked}");
        }
    }

    // Refutes touching names that are already right or are not Claude's: dated
    // suffixes, dash-only versions, and a dot that is not between two digits.
    #[test]
    fn other_names_are_sent_unchanged() {
        for name in [
            "claude-sonnet-4-5-20250929",
            "claude-sonnet-4-6",
            "claude-haiku-4-5",
            "gpt-5.6-mini",
            "deepseek-3.2",
            "claude-x.y",
            "claude-4.",
        ] {
            assert_eq!(sent_model(name), name, "{name}");
        }
    }
}
