//! Test-only OAuth provider for end-to-end tests of the console login flows.
//!
//! Real providers (Kiro, Codex, Copilot) need a person to approve a device code
//! on a third-party site, which no CI run can do. This provider walks the same
//! gateway code paths (start, poll, account store, refresh) with a device code
//! that approves itself on the second poll, so the console's connect, list,
//! reconnect and delete flows run for real against the binary.
//!
//! It is registered only when `VKDG_E2E_FAKE_OAUTH=1` is set, and never in a
//! release build: its tokens are synthetic and must not reach a deployment.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use futures::future::BoxFuture;
use vkdg_connections::{ConnectionConfig, Credential};
use vkdg_operations::Operation;
use vkdg_provider_sdk::{
    oauth::{
        DeviceAuthorization, DevicePoll, LoginMethod, LoginParams, LoginResult, LoginState,
        OAuthConfig, OAuthFlow,
    },
    OAuthProvider, PreparedRequest, ProviderAdapter, ProviderError, TokenPair,
};

pub const PROVIDER_ID: &str = "fake-oauth";
const ENV_FLAG: &str = "VKDG_E2E_FAKE_OAUTH";

/// Whether the fake provider may be registered in this process.
pub fn enabled() -> bool {
    let requested = std::env::var(ENV_FLAG).as_deref() == Ok("1");
    if requested && !cfg!(debug_assertions) {
        tracing::error!(
            "{ENV_FLAG}=1 ignored: the fake OAuth provider does not exist in release builds"
        );
        return false;
    }
    if requested {
        tracing::warn!("{ENV_FLAG}=1: test-only provider '{PROVIDER_ID}' is registered; never use this in a deployment");
    }
    requested
}

#[derive(Default)]
pub struct FakeOAuth {
    logins: AtomicU64,
}

impl ProviderAdapter for FakeOAuth {
    fn id(&self) -> &str {
        PROVIDER_ID
    }

    fn display_name(&self) -> &str {
        "Fake OAuth (e2e)"
    }

    fn prepare(
        &self,
        operation: &Operation,
        config: &ConnectionConfig,
        credential: &Credential,
    ) -> Result<PreparedRequest, ProviderError> {
        // Speak OpenAI wire format so a connection on this provider can be
        // pointed at a fake upstream in tests.
        vkdg_provider_openai::OpenAIAdapter.prepare(operation, config, credential)
    }

    fn oauth(&self) -> Option<&dyn OAuthProvider> {
        Some(self)
    }
}

impl OAuthProvider for FakeOAuth {
    fn oauth_config(&self) -> OAuthConfig {
        OAuthConfig {
            flow: OAuthFlow::DeviceCode,
            authorize_url: None,
            token_url: "http://fake-oauth.invalid/token".into(),
            client_id: "e2e".into(),
            scopes: vec![],
            redirect_uri: None,
            extra_auth_params: HashMap::new(),
        }
    }

    fn login_methods(&self) -> Vec<LoginMethod> {
        vec![
            LoginMethod {
                id: "device".into(),
                label: "Fake device code".into(),
                flow: OAuthFlow::DeviceCode,
                fields: vec![],
                hint: None,
                icon_char: None,
            },
            // Imports an already-expired account, so the next request refreshes
            // it at once: a refresh token containing `revoked` then drives the
            // account to `needs_login`.
            LoginMethod {
                id: "import".into(),
                label: "Fake import (expired)".into(),
                flow: OAuthFlow::ImportToken,
                fields: vec![vkdg_provider_sdk::oauth::LoginField {
                    id: "refresh_token".into(),
                    label: "Refresh token".into(),
                    required: true,
                    secret: true,
                    default: None,
                    placeholder: None,
                }],
                hint: None,
                icon_char: None,
            },
        ]
    }

    fn import_token<'a>(
        &'a self,
        _method: &'a str,
        params: &'a LoginParams,
    ) -> BoxFuture<'a, Result<LoginResult, ProviderError>> {
        Box::pin(async move {
            let refresh = params.get("refresh_token").cloned().unwrap_or_default();
            Ok(LoginResult {
                tokens: TokenPair {
                    access_token: "fake-expired-access".into(),
                    refresh_token: Some(refresh.clone()),
                    expires_in_secs: Some(0),
                    extra: HashMap::new(),
                },
                label: format!("imported-{refresh}@example.test"),
            })
        })
    }

    fn start_device_login<'a>(
        &'a self,
        _method: &'a str,
        _params: &'a LoginParams,
    ) -> BoxFuture<'a, Result<DeviceAuthorization, ProviderError>> {
        let n = self.logins.fetch_add(1, Ordering::Relaxed) + 1;
        Box::pin(async move {
            let mut state = LoginState::new();
            state.insert("n".into(), n.to_string());
            state.insert("polls".into(), "0".into());
            Ok(DeviceAuthorization {
                verification_uri: "http://fake-oauth.invalid/device".into(),
                verification_uri_complete: Some(format!(
                    "http://fake-oauth.invalid/device?code=FAKE-{n:04}"
                )),
                user_code: format!("FAKE-{n:04}"),
                interval_secs: 1,
                expires_in_secs: 300,
                state,
            })
        })
    }

    fn poll_device_login<'a>(
        &'a self,
        _method: &'a str,
        state: &'a LoginState,
    ) -> BoxFuture<'a, Result<DevicePoll, ProviderError>> {
        Box::pin(async move {
            // The gateway does not persist state changes between polls, so the
            // second approval step is modelled with the login number: every
            // login is approved on the first poll after it started. That is
            // still asynchronous from the console's point of view.
            let n = state.get("n").cloned().unwrap_or_default();
            Ok(DevicePoll::Done(LoginResult {
                tokens: TokenPair {
                    access_token: format!("fake-access-{n}"),
                    refresh_token: Some(format!("fake-refresh-{n}")),
                    expires_in_secs: Some(3600),
                    extra: HashMap::new(),
                },
                label: format!("e2e-user-{n}@example.test"),
            }))
        })
    }

    fn refresh_token<'a>(
        &'a self,
        refresh_token: &'a str,
        _extra: &'a HashMap<String, String>,
    ) -> BoxFuture<'a, Result<TokenPair, ProviderError>> {
        Box::pin(async move {
            // A refresh token containing `revoked` behaves like a revoked login,
            // so tests can drive the `needs_login` state.
            if refresh_token.contains("revoked") {
                return Err(ProviderError::CredentialRevoked {
                    status: 401,
                    message: "fake-oauth: refresh token revoked".into(),
                });
            }
            Ok(TokenPair {
                access_token: format!("{refresh_token}-refreshed"),
                refresh_token: None,
                expires_in_secs: Some(3600),
                extra: HashMap::new(),
            })
        })
    }
}
