//! Codex model discovery.
//!
//! A `ChatGPT` (OAuth) account asks the same endpoint the Codex CLI uses to fill
//! its model picker, so the list is exactly what that account may call:
//!
//! ```text
//! GET https://chatgpt.com/backend-api/codex/models?client_version=0.162.0
//!   authorization: Bearer <oauth access token>
//!   chatgpt-account-id: <account id from the token's JWT claims>
//! => 200 { models:[{ slug:"gpt-5.6-sol", display_name:"…", visibility:"list",
//!                    supported_in_api:true, … }] }
//! ```
//!
//! `client_version` is required (400 without it) and gates the answer: an older
//! version is served fewer models, so it tracks a current Codex CLI release.
//!
//! API-key and custom-endpoint connections have no such endpoint for Codex; they
//! keep reading the admin-managed `provider_catalog` table.

use std::sync::Arc;

use futures::future::BoxFuture;
use serde_json::Value;
use vkdg_connections::{AuthKind, ConnectionConfig, Credential, ProviderKind};
use vkdg_provider_sdk::{DynamicModelCatalog, ModelCatalog, ProviderError};

use crate::extract_chatgpt_account_id;

const MODELS_URL: &str = "https://chatgpt.com/backend-api/codex/models";

/// Codex CLI release reported as `client_version`; the backend hides models
/// newer than the client. Override with `CODEX_CLIENT_VERSION`.
const CODEX_CLIENT_VERSION: &str = "0.162.0";

/// Codex's [`ModelCatalog`], reachable from [`crate::CodexAdapter::model_catalog`].
pub struct CodexModelCatalog {
    /// Admin-managed list for connections the live endpoint cannot serve.
    stored: Option<DynamicModelCatalog>,
}

impl CodexModelCatalog {
    pub fn new(store: Option<Arc<vkdg_config::GatewayStore>>) -> Self {
        Self {
            stored: store.map(|store| DynamicModelCatalog {
                store,
                provider_id: "codex",
            }),
        }
    }
}

impl ModelCatalog for CodexModelCatalog {
    fn list_models<'a>(
        &'a self,
        config: &'a ConnectionConfig,
        credential: &'a Credential,
    ) -> BoxFuture<'a, Result<Vec<String>, ProviderError>> {
        if uses_chatgpt_backend(config) {
            return Box::pin(fetch_models(credential));
        }
        match &self.stored {
            Some(stored) => stored.list_models(config, credential),
            None => Box::pin(async {
                Err(ProviderError::Config(
                    "API-key Codex connections list models from the gateway catalog, \
                     which is unavailable"
                        .into(),
                ))
            }),
        }
    }
}

/// Same rule as `prepare`: an OAuth account on the default endpoint talks to
/// the `ChatGPT` backend.
fn uses_chatgpt_backend(config: &ConnectionConfig) -> bool {
    !matches!(config.auth, AuthKind::ApiKey { .. })
        && !matches!(config.provider, ProviderKind::Custom { .. })
}

fn client_version() -> String {
    std::env::var("CODEX_CLIENT_VERSION").unwrap_or_else(|_| CODEX_CLIENT_VERSION.into())
}

async fn fetch_models(credential: &Credential) -> Result<Vec<String>, ProviderError> {
    let account_id = credential
        .extra
        .get("chatgpt_account_id")
        .cloned()
        .or_else(|| extract_chatgpt_account_id(&credential.token));
    let mut req = reqwest::Client::new()
        .get(MODELS_URL)
        .query(&[("client_version", client_version())])
        .bearer_auth(&credential.token)
        .header("accept", "application/json");
    if let Some(id) = account_id {
        req = req.header("chatgpt-account-id", id);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| ProviderError::Http(e.to_string()))?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| ProviderError::Http(e.to_string()))?;
    if !status.is_success() {
        return Err(ProviderError::Http(format!(
            "codex models returned {}: {}",
            status.as_u16(),
            text.chars().take(200).collect::<String>()
        )));
    }
    let body: Value = serde_json::from_str(&text)
        .map_err(|e| ProviderError::Http(format!("models body: {e}")))?;
    parse_models(&body)
}

/// Slugs the Codex CLI offers in its picker: `visibility: "list"` and callable
/// through the Responses API. Hidden entries are internal (`codex-auto-review`)
/// or reserved.
fn parse_models(body: &Value) -> Result<Vec<String>, ProviderError> {
    let models = body
        .get("models")
        .and_then(Value::as_array)
        .ok_or_else(|| ProviderError::Http("codex models carried no models list".into()))?;
    Ok(models
        .iter()
        .filter(|m| m.get("visibility").and_then(Value::as_str) == Some("list"))
        .filter(|m| m.get("supported_in_api").and_then(Value::as_bool) != Some(false))
        .filter_map(|m| m.get("slug").and_then(Value::as_str))
        .map(str::trim)
        .filter(|slug| !slug.is_empty())
        .map(str::to_owned)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn config(provider: ProviderKind, auth: AuthKind) -> ConnectionConfig {
        ConnectionConfig {
            id: vkdg_core::ConnectionId("c".into()),
            provider,
            auth,
            models: vec!["gpt-*".into()],
            max_concurrent: 1,
            weight: 1,
            tags: vec![],
            endpoint: None,
            capabilities: vkdg_operations::CapabilitySet::default(),
        }
    }

    // Shaped like the live answer for a Plus account (2026-10).
    fn live_body() -> Value {
        json!({ "models": [
            { "slug": "gpt-6-astra", "display_name": "GPT-6 Astra", "visibility": "list", "supported_in_api": true },
            { "slug": "gpt-reserve", "visibility": "hide", "supported_in_api": true },
            { "slug": "gpt-5.6-sol", "visibility": "list", "supported_in_api": true },
            { "slug": "codex-auto-review", "visibility": "hide", "supported_in_api": true },
            { "slug": "gpt-legacy", "visibility": "list", "supported_in_api": false }
        ]})
    }

    // Refutes listing hidden/internal slugs or ones the Responses API rejects,
    // and reading `display_name` instead of `slug`.
    #[test]
    fn keeps_listed_api_models_by_slug() {
        assert_eq!(
            parse_models(&live_body()).unwrap(),
            ["gpt-6-astra", "gpt-5.6-sol"]
        );
    }

    // Refutes treating an error body as an empty list, which would fail the
    // sync with a misleading "no models" instead of the upstream reason.
    #[test]
    fn a_body_without_models_is_an_error() {
        assert!(parse_models(&json!({ "error": { "message": "unauthorized" } })).is_err());
        assert!(parse_models(&json!({ "models": null })).is_err());
    }

    // Refutes sending API-key or custom-endpoint connections to the ChatGPT
    // backend, which only accepts ChatGPT OAuth tokens.
    #[test]
    fn only_oauth_on_the_default_endpoint_goes_live() {
        let plugin = || ProviderKind::Plugin { id: "codex".into() };
        let account = || AuthKind::Account {
            account_id: "a".into(),
        };
        let api_key = || AuthKind::ApiKey {
            env_var: "OPENAI_API_KEY".into(),
        };
        let custom = || ProviderKind::Custom {
            base_url: "http://proxy.local".into(),
        };
        assert!(uses_chatgpt_backend(&config(plugin(), account())));
        assert!(!uses_chatgpt_backend(&config(plugin(), api_key())));
        assert!(!uses_chatgpt_backend(&config(custom(), account())));
    }
}
