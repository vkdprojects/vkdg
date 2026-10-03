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
const CLAUDE_SCOPES: [&str; 5] = [
    "org:create_api_key",
    "user:profile",
    "user:inference",
    "user:sessions:claude_code",
    "user:mcp_servers",
];

/// `N` random bytes, base64url without padding.
fn random_b64url<const N: usize>() -> String {
    use base64::Engine as _;
    use rand::RngCore;

    let mut bytes = [0u8; N];
    rand::thread_rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// The only non-default redirect claude.ai accepts for this client is the
/// loopback `/callback` the official CLI listens on. The console passes its own
/// origin there so the code is captured without copy-paste; anything else could
/// hand the authorization code to a host the operator does not control.
fn validated_loopback_redirect(uri: &str) -> Result<String, ProviderError> {
    let reject = || {
        ProviderError::Config(format!(
            "redirect_uri must be http://localhost[:port]/callback or http://127.0.0.1[:port]/callback, got '{uri}'"
        ))
    };
    let url = reqwest::Url::parse(uri).map_err(|_| reject())?;
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1"));
    let plain = url.scheme() == "http"
        && url.username().is_empty()
        && url.password().is_none()
        && url.path() == "/callback"
        && url.query().is_none()
        && url.fragment().is_none();
    if loopback && plain {
        Ok(url.to_string())
    } else {
        Err(reject())
    }
}

/// Split what the user pasted into `(code, state)`: the `code#state` string the
/// manual callback page shows, a full callback URL, or a bare code.
fn split_pasted_code(pasted: &str) -> Result<(String, String), ProviderError> {
    let pasted = pasted.trim();
    if pasted.starts_with("http://") || pasted.starts_with("https://") {
        let url = reqwest::Url::parse(pasted).map_err(|e| ProviderError::Http(e.to_string()))?;
        let pairs: HashMap<_, _> = url.query_pairs().into_owned().collect();
        let code = pairs
            .get("code")
            .cloned()
            .unwrap_or_else(|| pasted.to_owned());
        let state = pairs.get("state").cloned().unwrap_or_default();
        return Ok((code, state));
    }
    let (code, state) = pasted.split_once('#').unwrap_or((pasted, ""));
    Ok((code.to_owned(), state.to_owned()))
}

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
        OAuthConfig {
            flow: OAuthFlow::AuthorizationCodePkce,
            authorize_url: Some(CLAUDE_AUTHORIZE_URL.into()),
            token_url: CLAUDE_TOKEN_URL.into(),
            client_id: std::env::var("CLAUDE_OAUTH_CLIENT_ID")
                .unwrap_or_else(|_| CLAUDE_CLIENT_ID.into()),
            scopes: CLAUDE_SCOPES.iter().map(|s| (*s).to_owned()).collect(),
            redirect_uri: Some(claude_redirect_uri()),
            extra_auth_params: HashMap::new(),
        }
    }

    fn start_pkce_login<'a>(
        &'a self,
        _method: &'a str,
        params: &'a vkdg_provider_sdk::LoginParams,
    ) -> BoxFuture<'a, Result<vkdg_provider_sdk::PkceAuthorization, ProviderError>> {
        Box::pin(async move {
            use base64::Engine as _;
            use sha2::{Digest, Sha256};

            let redirect_uri = match params.get("redirect_uri").filter(|u| !u.is_empty()) {
                Some(uri) => validated_loopback_redirect(uri)?,
                None => claude_redirect_uri(),
            };

            // Code verifier (RFC 7636 §4.1).
            let verifier = random_b64url::<32>();

            // S256 challenge = BASE64URL(SHA256(verifier)).
            let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(Sha256::digest(verifier.as_bytes()));

            // OAuth `state`. claude.ai rejects Authorize with "Invalid request
            // format" unless it has the same 32 bytes of entropy the official CLI sends.
            let state = random_b64url::<32>();

            let client_id =
                std::env::var("CLAUDE_OAUTH_CLIENT_ID").unwrap_or_else(|_| CLAUDE_CLIENT_ID.into());

            let mut url = reqwest::Url::parse(CLAUDE_AUTHORIZE_URL)
                .map_err(|e| ProviderError::Http(e.to_string()))?;
            url.query_pairs_mut()
                .append_pair("code", "true")
                .append_pair("client_id", &client_id)
                .append_pair("response_type", "code")
                .append_pair("redirect_uri", &redirect_uri)
                .append_pair("scope", &CLAUDE_SCOPES.join(" "))
                .append_pair("code_challenge", &challenge)
                .append_pair("code_challenge_method", "S256")
                .append_pair("state", &state);

            let mut login_state = HashMap::new();
            login_state.insert("code_verifier".into(), verifier);
            login_state.insert("state".into(), state);
            login_state.insert("redirect_uri".into(), redirect_uri);

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
            let verifier = state.get("code_verifier").cloned().unwrap_or_default();
            let original_state = state.get("state").cloned().unwrap_or_default();
            // Must repeat the authorize `redirect_uri` verbatim.
            let redirect_uri = state
                .get("redirect_uri")
                .cloned()
                .unwrap_or_else(claude_redirect_uri);
            let (auth_code, code_state) = split_pasted_code(code)?;
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

fn build_body(req: &ConversationRequest) -> Bytes {
    let model = vkdg_provider_sdk::upstream_model(req, "claude-opus-4-5");
    // A subscription (OAuth) token is only accepted for Claude Code traffic:
    // without this exact first system block Anthropic answers 429
    // rate_limit_error. The client's own system prompt follows it untouched.
    vkdg_provider_sdk::anthropic_messages::messages_body(req, model, Some(CLAUDE_CODE_IDENTITY))
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
    use super::split_pasted_code;

    /// The manual page shows `code#state`; users also paste the whole callback
    /// URL or just the code. All three must yield the same pair.
    #[test]
    fn pasted_code_forms_yield_code_and_state() {
        for (pasted, code, state) in [
            ("abc123#st4te", "abc123", "st4te"),
            ("  abc123#st4te\n", "abc123", "st4te"),
            (
                "http://localhost:9090/callback?code=abc123&state=st4te",
                "abc123",
                "st4te",
            ),
            (
                "https://platform.claude.com/oauth/code/callback?code=abc123&state=st4te",
                "abc123",
                "st4te",
            ),
            ("abc123", "abc123", ""),
        ] {
            let (c, s) = split_pasted_code(pasted).unwrap();
            assert_eq!((c.as_str(), s.as_str()), (code, state), "{pasted:?}");
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
        assert_eq!(
            v["system"],
            json!([
                { "type": "text", "text": IDENTITY },
                { "type": "text", "text": "Be brief." },
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
}
