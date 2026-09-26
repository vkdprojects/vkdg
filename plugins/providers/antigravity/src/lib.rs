//! VKDG provider plugin: antigravity
//! Google Cloud Code (Antigravity) — Authorization Code OAuth.
//!
//! prepare() is a stub; full proto API implementation is Phase E.

use std::collections::HashMap;

use futures::future::BoxFuture;
use vkdg_connections::ConnectionConfig;
use vkdg_operations::Operation;
use vkdg_provider_sdk::{
    OAuthConfig, OAuthFlow, OAuthProvider, PreparedRequest, ProviderAdapter, ProviderError,
    TokenPair,
};

/// Fallback client_id for Antigravity (Google Cloud Code native app).
const ANTIGRAVITY_CLIENT_ID_FALLBACK: &str = "1071006060591-tmhssin2h21lcre235vtolojh4g403ep";

pub struct AntigravityAdapter;

impl ProviderAdapter for AntigravityAdapter {
    fn id(&self) -> &str {
        "antigravity"
    }

    fn display_name(&self) -> &str {
        "Antigravity (Google Cloud Code)"
    }

    fn prepare(
        &self,
        _operation: &Operation,
        _config: &ConnectionConfig,
        _token: &str,
    ) -> Result<PreparedRequest, ProviderError> {
        // Phase E: Antigravity uses a custom proto API over cloudcode-pa.googleapis.com.
        Err(ProviderError::UnsupportedOperation)
    }
}

impl OAuthProvider for AntigravityAdapter {
    fn oauth_config(&self) -> OAuthConfig {
        let mut extra_auth_params = HashMap::new();
        extra_auth_params.insert("access_type".into(), "offline".into());
        extra_auth_params.insert("prompt".into(), "consent".into());

        // Note: Antigravity uses standard auth_code without PKCE; AuthorizationCodePkce
        // is the closest available variant in OAuthFlow.
        OAuthConfig {
            flow: OAuthFlow::AuthorizationCodePkce,
            authorize_url: Some("https://accounts.google.com/o/oauth2/v2/auth".into()),
            token_url: "https://oauth2.googleapis.com/token".into(),
            client_id: std::env::var("ANTIGRAVITY_OAUTH_CLIENT_ID")
                .unwrap_or_else(|_| ANTIGRAVITY_CLIENT_ID_FALLBACK.into()),
            scopes: vec![
                "https://www.googleapis.com/auth/cloud-platform".into(),
                "https://www.googleapis.com/auth/userinfo.email".into(),
            ],
            redirect_uri: None,
            extra_auth_params,
        }
    }

    fn refresh_token<'a>(
        &'a self,
        refresh_token: &'a str,
        extra: &'a HashMap<String, String>,
    ) -> BoxFuture<'a, Result<TokenPair, ProviderError>> {
        Box::pin(async move {
            let client_id = std::env::var("ANTIGRAVITY_OAUTH_CLIENT_ID")
                .unwrap_or_else(|_| ANTIGRAVITY_CLIENT_ID_FALLBACK.into());
            let client_secret = extra.get("client_secret").cloned().unwrap_or_default();

            let client = reqwest::Client::new();
            let resp = client
                .post("https://oauth2.googleapis.com/token")
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

            // Preserve project_id and client_secret for next refresh.
            let mut out_extra = HashMap::new();
            for key in ["project_id", "client_secret"] {
                if let Some(v) = extra.get(key) {
                    out_extra.insert(key.to_string(), v.clone());
                }
            }

            Ok(TokenPair {
                access_token: json["access_token"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                // Google does not return a new refresh_token on refresh.
                refresh_token: json["refresh_token"].as_str().map(str::to_string),
                expires_in_secs: json["expires_in"].as_u64(),
                extra: out_extra,
            })
        })
    }
}
