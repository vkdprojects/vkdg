use std::collections::HashMap;

use futures::future::BoxFuture;

use crate::{ProviderAdapter, ProviderError};

/// OAuth flow type supported by this provider.
#[derive(Debug, Clone)]
pub enum OAuthFlow {
    /// Standard authorization_code with PKCE (S256).
    /// Claude Code, Codex.
    AuthorizationCodePkce,
    /// Device authorization grant (RFC 8628).
    /// Kiro/Amazon Q, Kimi, GitHub Copilot.
    DeviceCode,
    /// Import an existing token (no interactive OAuth).
    /// Cursor, Raycast.
    ImportToken,
}

/// Static OAuth configuration for a provider.
/// Returned by [`OAuthProvider::oauth_config`]; used by admin UI to initiate the flow.
#[derive(Debug, Clone)]
pub struct OAuthConfig {
    pub flow: OAuthFlow,
    /// Authorization endpoint (None for device_code flows that skip it).
    pub authorize_url: Option<String>,
    /// Token endpoint — used for exchange AND refresh.
    pub token_url: String,
    /// Public client_id.
    pub client_id: String,
    pub scopes: Vec<String>,
    /// Redirect URI for PKCE flows.
    pub redirect_uri: Option<String>,
    /// Extra query params to append to the authorize URL.
    pub extra_auth_params: HashMap<String, String>,
}

/// Successful token response from exchange or refresh.
pub struct TokenPair {
    pub access_token: String,
    pub refresh_token: Option<String>,
    /// Seconds until access_token expires. None = unknown.
    pub expires_in_secs: Option<u64>,
    /// Provider-specific data that MUST be persisted and passed back on next refresh.
    /// Examples: kiro clientId/clientSecret, kimi deviceId, antigravity projectId.
    pub extra: HashMap<String, String>,
}

/// Extension trait for OAuth-backed providers.
/// Implement this alongside [`ProviderAdapter`].
pub trait OAuthProvider: ProviderAdapter {
    /// Static OAuth config used by admin UI to initiate connection.
    fn oauth_config(&self) -> OAuthConfig;

    /// Refresh an expired access token.
    ///
    /// `extra` = provider-specific data from the previous [`TokenPair`] (or initial exchange).
    /// Returns a new [`TokenPair`]; the gateway persists the updated tokens + extra.
    fn refresh_token<'a>(
        &'a self,
        refresh_token: &'a str,
        extra: &'a HashMap<String, String>,
    ) -> BoxFuture<'a, Result<TokenPair, ProviderError>>;
}
