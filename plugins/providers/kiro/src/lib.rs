//! VKDG provider plugin: kiro
//! Amazon Q / CodeWhisperer — Device Code OAuth.
//!
//! prepare() is a stub; full CodeWhisperer protocol is Phase E.

use std::collections::HashMap;

use futures::future::BoxFuture;
use vkdg_connections::ConnectionConfig;
use vkdg_operations::Operation;
use vkdg_provider_sdk::{
    OAuthConfig, OAuthFlow, OAuthProvider, PreparedRequest, ProviderAdapter, ProviderError,
    TokenPair,
};

pub struct KiroAdapter;

impl ProviderAdapter for KiroAdapter {
    fn id(&self) -> &str {
        "kiro"
    }

    fn display_name(&self) -> &str {
        "Kiro / Amazon Q"
    }

    fn prepare(
        &self,
        _operation: &Operation,
        _config: &ConnectionConfig,
        _token: &str,
    ) -> Result<PreparedRequest, ProviderError> {
        // Phase E: full CodeWhisperer protocol implementation over
        // https://codewhisperer.us-east-1.amazonaws.com
        Err(ProviderError::UnsupportedOperation)
    }
}

impl OAuthProvider for KiroAdapter {
    fn oauth_config(&self) -> OAuthConfig {
        OAuthConfig {
            flow: OAuthFlow::DeviceCode,
            authorize_url: None,
            // Default region; individual connections may use a regional endpoint.
            token_url: "https://oidc.us-east-1.amazonaws.com/token".into(),
            // client_id is dynamically registered per-connection at flow init; empty here.
            client_id: String::new(),
            scopes: vec![
                "codewhisperer:completions".into(),
                "codewhisperer:analysis".into(),
                "codewhisperer:conversations".into(),
            ],
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
            let region = extra
                .get("region")
                .map(String::as_str)
                .unwrap_or("us-east-1");
            let url = format!("https://oidc.{region}.amazonaws.com/token");
            let client_id = extra.get("client_id").cloned().unwrap_or_default();
            let client_secret = extra.get("client_secret").cloned().unwrap_or_default();

            let client = reqwest::Client::new();
            let resp = client
                .post(&url)
                .json(&serde_json::json!({
                    "grant_type": "refresh_token",
                    "refresh_token": refresh_token,
                    "client_id": client_id,
                    "client_secret": client_secret,
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

            let json: serde_json::Value = resp
                .json()
                .await
                .map_err(|e| ProviderError::Serialization(e.to_string()))?;

            // Preserve client_id, client_secret, region for next refresh.
            let mut out_extra = HashMap::new();
            for key in ["client_id", "client_secret", "region"] {
                if let Some(v) = extra.get(key) {
                    out_extra.insert(key.to_string(), v.clone());
                }
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
