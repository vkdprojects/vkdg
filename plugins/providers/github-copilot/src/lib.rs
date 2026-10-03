//! VKDG provider plugin: github-copilot
//! GitHub Copilot — Device Code OAuth with `OpenAI` Chat Completions wire format.

use std::collections::HashMap;

use futures::future::BoxFuture;
use http::HeaderMap;
use vkdg_connections::{ConnectionConfig, ProviderKind};
use vkdg_operations::Operation;
use vkdg_provider_sdk::openai_compat::chat_completions_body;
use vkdg_provider_sdk::{
    Credential, OAuthConfig, OAuthFlow, OAuthProvider, PreparedRequest, ProviderAdapter,
    ProviderError, TokenPair,
};

pub struct GitHubCopilotAdapter;

impl ProviderAdapter for GitHubCopilotAdapter {
    fn oauth(&self) -> Option<&dyn OAuthProvider> {
        Some(self)
    }

    fn id(&self) -> &'static str {
        "github-copilot"
    }

    fn display_name(&self) -> &'static str {
        "GitHub Copilot"
    }

    fn wire_format(&self, _config: &ConnectionConfig) -> Option<vkdg_operations::WireFormat> {
        Some(vkdg_operations::WireFormat::OpenAiChat)
    }

    fn default_models(&self) -> Vec<String> {
        ["gpt-*", "o1-*", "o3-*", "o4-*"].map(String::from).into()
    }

    fn meta(&self) -> vkdg_provider_sdk::ProviderMeta {
        vkdg_provider_sdk::ProviderMeta {
            icon_char: 'G',
            icon_color: "#24292f",
            category: vkdg_provider_sdk::ProviderCategory::OauthIde,
            site_url: Some("https://github.com/features/copilot"),
            description: Some("GitHub Copilot — AI coding assistant."),
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

        // Note: production use requires fetching a short-lived copilot_token from
        //   /copilot_internal/v2/token first; Phase E will add that layer.
        let body = chat_completions_body(req, vkdg_provider_sdk::upstream_model(req, "gpt-4o"));
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
