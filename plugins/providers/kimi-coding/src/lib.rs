//! VKDG provider plugin: kimi-coding
//! Kimi Coding — Device Code OAuth with `OpenAI` Chat Completions wire format.

use std::collections::HashMap;

use futures::future::BoxFuture;
use http::HeaderMap;
use serde_json::{json, Value};
use vkdg_connections::{ConnectionConfig, ProviderKind};
use vkdg_operations::Operation;
use vkdg_provider_sdk::openai_compat::chat_completions_body;
use vkdg_provider_sdk::{
    Credential, OAuthConfig, OAuthFlow, OAuthProvider, PreparedRequest, ProviderAdapter,
    ProviderError, TokenPair,
};

pub struct KimiCodingAdapter;

impl ProviderAdapter for KimiCodingAdapter {
    fn oauth(&self) -> Option<&dyn OAuthProvider> {
        Some(self)
    }

    fn id(&self) -> &'static str {
        "kimi-coding"
    }

    fn display_name(&self) -> &'static str {
        "Kimi Coding"
    }

    fn wire_format(&self, _config: &ConnectionConfig) -> Option<vkdg_operations::WireFormat> {
        Some(vkdg_operations::WireFormat::OpenAiChat)
    }

    fn default_models(&self) -> Vec<String> {
        vec!["kimi-*".into()]
    }

    fn meta(&self) -> vkdg_provider_sdk::ProviderMeta {
        vkdg_provider_sdk::ProviderMeta {
            icon_char: 'K',
            icon_color: "#1DB4C4",
            category: vkdg_provider_sdk::ProviderCategory::OauthIde,
            site_url: Some("https://kimi.ai"),
            description: Some("Kimi Coding — Moonshot AI coding assistant."),
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

        let body = chat_completions_body(
            req,
            vkdg_provider_sdk::upstream_model(req, "kimi-k1-5-turbo"),
        );
        let url = format!("{}/v1/chat/completions", base_url(config));

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
        headers.insert("x-client-type", http::HeaderValue::from_static("CLI"));
        // x-device-id would be set here if available via ConnectionConfig;
        // currently only available in OAuth extra during token refresh.

        Ok(PreparedRequest {
            url,
            headers,
            body,
            is_streaming: req.stream,
        })
    }
}

impl OAuthProvider for KimiCodingAdapter {
    fn oauth_config(&self) -> OAuthConfig {
        OAuthConfig {
            flow: OAuthFlow::DeviceCode,
            authorize_url: None,
            token_url: "https://auth.kimi.com/api/oauth/token".into(),
            client_id: std::env::var("KIMI_CODING_OAUTH_CLIENT_ID").unwrap_or_default(),
            scopes: vec![],
            redirect_uri: None,
            extra_auth_params: HashMap::new(),
        }
    }

    fn refresh_token<'a>(
        &'a self,
        refresh_token: &'a str,
        extra: &'a HashMap<String, String>,
    ) -> BoxFuture<'a, Result<TokenPair, ProviderError>> {
        Box::pin(async move {
            let client_id = std::env::var("KIMI_CODING_OAUTH_CLIENT_ID").unwrap_or_default();
            let device_id = extra.get("device_id").cloned().unwrap_or_default();

            let client = reqwest::Client::new();
            let resp = client
                .post("https://auth.kimi.com/api/oauth/token")
                // CRITICAL: device_id must be stable across refreshes
                .header("x-device-id", &device_id)
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

            // Preserve device_id for next refresh.
            let mut out_extra = HashMap::new();
            if let Some(v) = extra.get("device_id") {
                out_extra.insert("device_id".to_string(), v.clone());
            }

            Ok(TokenPair {
                access_token: json["access_token"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                refresh_token: json["refresh_token"].as_str().map(str::to_string),
                expires_in_secs: json["expires_in"].as_u64(),
                extra: out_extra,
            })
        })
    }
}

fn base_url(config: &ConnectionConfig) -> String {
    match &config.provider {
        ProviderKind::Custom { base_url } => base_url.clone(),
        _ => "https://api.kimi.ai".into(),
    }
}
