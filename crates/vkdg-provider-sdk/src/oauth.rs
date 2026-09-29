//! OAuth extension for provider plugins: interactive login + token refresh.
//!
//! A plugin opts in by implementing [`OAuthProvider`] and returning `Some(self)`
//! from [`ProviderAdapter::oauth`]. It advertises [`LoginMethod`]s (id, label,
//! kind, fields) so the CLI and admin console can render any provider generically,
//! and implements only the flows it supports; the rest default to
//! [`ProviderError::UnsupportedOperation`].
//!
//! The gateway owns persistence: login results become an `Account` in the
//! `AccountStore`, and refresh is dispatched back here through
//! [`ProviderRegistry`](crate::ProviderRegistry) (which implements `TokenRefresher`).

use std::collections::HashMap;
use std::future::Future;
use std::time::Duration;

use futures::future::BoxFuture;
use serde::Serialize;

use crate::{ProviderAdapter, ProviderError};

pub use vkdg_connections::TokenPair;

/// Plugin-defined login options (e.g. `region`, `start_url`, `refresh_token`).
pub type LoginParams = HashMap<String, String>;

/// Opaque plugin state carried between the start and completion of a login
/// (device code, PKCE verifier, client registration). Never sent to browsers.
pub type LoginState = HashMap<String, String>;

/// OAuth flow type supported by this provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OAuthFlow {
    /// Standard `authorization_code` with PKCE (S256).
    AuthorizationCodePkce,
    /// Device authorization grant (RFC 8628).
    DeviceCode,
    /// Import an existing token or credential blob (no interactive OAuth).
    ImportToken,
}

/// Static OAuth configuration for a provider.
#[derive(Debug, Clone)]
pub struct OAuthConfig {
    pub flow: OAuthFlow,
    /// Authorization endpoint (None for `device_code` flows that skip it).
    pub authorize_url: Option<String>,
    /// Token endpoint — used for exchange AND refresh.
    pub token_url: String,
    /// Public `client_id`.
    pub client_id: String,
    pub scopes: Vec<String>,
    /// Redirect URI for PKCE flows.
    pub redirect_uri: Option<String>,
    /// Extra query params to append to the authorize URL.
    pub extra_auth_params: HashMap<String, String>,
}

/// One input a login method needs (rendered as a form field / `--opt key=value`).
#[derive(Debug, Clone, Serialize)]
pub struct LoginField {
    pub id: String,
    pub label: String,
    pub required: bool,
    /// Mask the value in UIs (pasted tokens, secrets).
    pub secret: bool,
    pub default: Option<String>,
    /// Short placeholder or example shown in the input.
    #[serde(default)]
    pub placeholder: Option<String>,
}

/// A login method a plugin supports, e.g. `builder-id` (device code) or `import`.
#[derive(Debug, Clone, Serialize)]
pub struct LoginMethod {
    pub id: String,
    pub label: String,
    pub flow: OAuthFlow,
    pub fields: Vec<LoginField>,
    /// One-line description shown under the method name in the UI picker.
    #[serde(default)]
    pub hint: Option<String>,
    /// Single icon character (overrides the provider icon for this method).
    #[serde(default)]
    pub icon_char: Option<char>,
}

/// Returned by [`OAuthProvider::start_device_login`]; shown to the user.
#[derive(Clone)]
pub struct DeviceAuthorization {
    pub verification_uri: String,
    pub verification_uri_complete: Option<String>,
    pub user_code: String,
    /// Minimum seconds between polls.
    pub interval_secs: u64,
    /// Seconds until the device code expires.
    pub expires_in_secs: u64,
    /// Passed back to [`OAuthProvider::poll_device_login`].
    pub state: LoginState,
}

impl std::fmt::Debug for DeviceAuthorization {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceAuthorization")
            .field("verification_uri", &self.verification_uri)
            .field("user_code", &self.user_code)
            .field("interval_secs", &self.interval_secs)
            .field("expires_in_secs", &self.expires_in_secs)
            .finish_non_exhaustive()
    }
}

/// Returned by [`OAuthProvider::start_pkce_login`].
#[derive(Clone)]
pub struct PkceAuthorization {
    /// URL the user opens in a browser.
    pub authorize_url: String,
    /// Passed back to [`OAuthProvider::finish_pkce_login`] (verifier, OAuth `state`).
    pub state: LoginState,
}

/// A completed login.
#[derive(Debug)]
pub struct LoginResult {
    pub tokens: TokenPair,
    /// Account identity for listings (email, profile name). Empty = use provider id.
    pub label: String,
}

/// Result of one device-code poll.
#[derive(Debug)]
pub enum DevicePoll {
    /// User has not approved yet; poll again after the interval.
    Pending,
    /// Server asked to slow down; add 5 s to the interval (RFC 8628 §3.5).
    SlowDown,
    Done(LoginResult),
    /// Terminal failure (denied, expired, invalid). Message is safe to show.
    Failed(String),
}

/// Extension trait for OAuth-backed providers. Implement alongside
/// [`ProviderAdapter`] and override [`ProviderAdapter::oauth`] to return `Some(self)`.
pub trait OAuthProvider: ProviderAdapter {
    /// Static OAuth config (endpoints, client id, scopes).
    fn oauth_config(&self) -> OAuthConfig;

    /// Login methods this plugin supports. Empty = refresh-only.
    fn login_methods(&self) -> Vec<LoginMethod> {
        Vec::new()
    }

    /// Start a device-code login.
    fn start_device_login<'a>(
        &'a self,
        _method: &'a str,
        _params: &'a LoginParams,
    ) -> BoxFuture<'a, Result<DeviceAuthorization, ProviderError>> {
        Box::pin(async { Err(ProviderError::UnsupportedOperation) })
    }

    /// Poll a device-code login once.
    fn poll_device_login<'a>(
        &'a self,
        _method: &'a str,
        _state: &'a LoginState,
    ) -> BoxFuture<'a, Result<DevicePoll, ProviderError>> {
        Box::pin(async { Err(ProviderError::UnsupportedOperation) })
    }

    /// Start a PKCE authorization-code login.
    fn start_pkce_login<'a>(
        &'a self,
        _method: &'a str,
        _params: &'a LoginParams,
    ) -> BoxFuture<'a, Result<PkceAuthorization, ProviderError>> {
        Box::pin(async { Err(ProviderError::UnsupportedOperation) })
    }

    /// Exchange the authorization code (or pasted callback URL) for tokens.
    fn finish_pkce_login<'a>(
        &'a self,
        _method: &'a str,
        _state: &'a LoginState,
        _code: &'a str,
    ) -> BoxFuture<'a, Result<LoginResult, ProviderError>> {
        Box::pin(async { Err(ProviderError::UnsupportedOperation) })
    }

    /// Validate a pasted refresh token / credential blob and return tokens.
    fn import_token<'a>(
        &'a self,
        _method: &'a str,
        _params: &'a LoginParams,
    ) -> BoxFuture<'a, Result<LoginResult, ProviderError>> {
        Box::pin(async { Err(ProviderError::UnsupportedOperation) })
    }

    /// Refresh an access token.
    ///
    /// `extra` = persisted provider data from the account. Keys returned in
    /// [`TokenPair::extra`] overwrite stored keys; missing keys are kept.
    fn refresh_token<'a>(
        &'a self,
        refresh_token: &'a str,
        extra: &'a HashMap<String, String>,
    ) -> BoxFuture<'a, Result<TokenPair, ProviderError>>;
}

/// Find a login method by id, or the first one when `method` is `None`.
pub fn find_login_method(
    provider: &dyn OAuthProvider,
    method: Option<&str>,
) -> Result<LoginMethod, ProviderError> {
    let methods = provider.login_methods();
    let found = match method {
        Some(id) => methods.into_iter().find(|m| m.id == id),
        None => methods.into_iter().next(),
    };
    found.ok_or_else(|| {
        ProviderError::Config(format!(
            "provider '{}' has no login method '{}'",
            provider.id(),
            method.unwrap_or("<default>")
        ))
    })
}

/// Check required fields and fill defaults. Returns the effective params.
pub fn resolve_login_params(
    method: &LoginMethod,
    params: &LoginParams,
) -> Result<LoginParams, ProviderError> {
    let mut out = params.clone();
    for field in &method.fields {
        if out.get(&field.id).is_some_and(|v| !v.is_empty()) {
            continue;
        }
        match &field.default {
            Some(d) => {
                out.insert(field.id.clone(), d.clone());
            }
            None if field.required => {
                return Err(ProviderError::Config(format!(
                    "login method '{}' requires '{}' ({})",
                    method.id, field.id, field.label
                )));
            }
            None => {}
        }
    }
    Ok(out)
}

/// Drive a device-code login to completion.
///
/// Polls every `interval_secs` (increased by 5 s on `SlowDown`) until `Done`,
/// `Failed`, or the device code expires. `sleep` is injected so callers use
/// `tokio::time::sleep` and tests stay deterministic.
pub async fn run_device_login<S, F>(
    provider: &dyn OAuthProvider,
    method: &str,
    auth: &DeviceAuthorization,
    mut sleep: S,
) -> Result<LoginResult, ProviderError>
where
    S: FnMut(Duration) -> F,
    F: Future<Output = ()>,
{
    let mut interval = auth.interval_secs.max(1);
    let mut elapsed = 0u64;
    loop {
        if elapsed >= auth.expires_in_secs {
            return Err(ProviderError::TokenRefresh(
                "device code expired before the login was approved".into(),
            ));
        }
        sleep(Duration::from_secs(interval)).await;
        elapsed += interval;
        match provider.poll_device_login(method, &auth.state).await? {
            DevicePoll::Pending => {}
            DevicePoll::SlowDown => interval += 5,
            DevicePoll::Done(result) => return Ok(result),
            DevicePoll::Failed(msg) => return Err(ProviderError::TokenRefresh(msg)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PreparedRequest;
    use parking_lot::Mutex;
    use vkdg_connections::{ConnectionConfig, Credential};
    use vkdg_operations::Operation;

    /// Fake provider replaying a scripted sequence of poll outcomes.
    struct Scripted {
        script: Mutex<Vec<&'static str>>,
        polls: Mutex<Vec<String>>,
    }

    impl ProviderAdapter for Scripted {
        fn id(&self) -> &'static str {
            "scripted"
        }
        fn display_name(&self) -> &'static str {
            "Scripted"
        }
        fn prepare(
            &self,
            _: &Operation,
            _: &ConnectionConfig,
            _: &Credential,
        ) -> Result<PreparedRequest, ProviderError> {
            Err(ProviderError::UnsupportedOperation)
        }
        fn oauth(&self) -> Option<&dyn OAuthProvider> {
            Some(self)
        }
    }

    impl OAuthProvider for Scripted {
        fn oauth_config(&self) -> OAuthConfig {
            OAuthConfig {
                flow: OAuthFlow::DeviceCode,
                authorize_url: None,
                token_url: String::new(),
                client_id: String::new(),
                scopes: vec![],
                redirect_uri: None,
                extra_auth_params: HashMap::new(),
            }
        }
        fn login_methods(&self) -> Vec<LoginMethod> {
            vec![LoginMethod {
                id: "device".into(),
                label: "Device".into(),
                flow: OAuthFlow::DeviceCode,
                fields: vec![
                    LoginField {
                        id: "region".into(),
                        label: "Region".into(),
                        required: true,
                        secret: false,
                        default: Some("us-east-1".into()),
                        placeholder: None,
                    },
                    LoginField {
                        id: "start_url".into(),
                        label: "Start URL".into(),
                        required: true,
                        secret: false,
                        default: None,
                        placeholder: None,
                    },
                ],
                hint: None,
                icon_char: None,
            }]
        }
        fn poll_device_login<'a>(
            &'a self,
            _method: &'a str,
            state: &'a LoginState,
        ) -> BoxFuture<'a, Result<DevicePoll, ProviderError>> {
            Box::pin(async move {
                self.polls.lock().push(state["device_code"].clone());
                let next = self.script.lock().remove(0);
                Ok(match next {
                    "pending" => DevicePoll::Pending,
                    "slow" => DevicePoll::SlowDown,
                    "denied" => DevicePoll::Failed("access_denied".into()),
                    _ => DevicePoll::Done(LoginResult {
                        tokens: TokenPair {
                            access_token: "at".into(),
                            refresh_token: Some("rt".into()),
                            expires_in_secs: Some(3600),
                            extra: HashMap::new(),
                        },
                        label: "me@example.com".into(),
                    }),
                })
            })
        }
        fn refresh_token<'a>(
            &'a self,
            _: &'a str,
            _: &'a HashMap<String, String>,
        ) -> BoxFuture<'a, Result<TokenPair, ProviderError>> {
            Box::pin(async { Err(ProviderError::UnsupportedOperation) })
        }
    }

    fn scripted(script: Vec<&'static str>) -> Scripted {
        Scripted {
            script: Mutex::new(script),
            polls: Mutex::new(vec![]),
        }
    }

    fn auth(interval: u64, expires: u64) -> DeviceAuthorization {
        DeviceAuthorization {
            verification_uri: "https://example.com/device".into(),
            verification_uri_complete: None,
            user_code: "ABCD-EFGH".into(),
            interval_secs: interval,
            expires_in_secs: expires,
            state: HashMap::from([("device_code".to_string(), "dc-1".to_string())]),
        }
    }

    // Plausible wrong impl: ignores slow_down (gets the client banned), polls without
    // waiting, or drops the plugin state between polls.
    #[tokio::test]
    async fn device_login_waits_backs_off_on_slow_down_and_returns_tokens() {
        let p = scripted(vec!["pending", "slow", "pending", "done"]);
        let sleeps = Mutex::new(vec![]);
        let result = run_device_login(&p, "device", &auth(2, 600), |d| {
            sleeps.lock().push(d.as_secs());
            async {}
        })
        .await
        .unwrap();
        assert_eq!(result.tokens.access_token, "at");
        assert_eq!(result.label, "me@example.com");
        assert_eq!(*sleeps.lock(), vec![2, 2, 7, 7]);
        assert_eq!(*p.polls.lock(), vec!["dc-1"; 4]);
    }

    // Plausible wrong impl: treats a terminal failure as pending and loops forever.
    #[tokio::test]
    async fn device_login_stops_on_failure() {
        let p = scripted(vec!["pending", "denied", "done"]);
        let err = run_device_login(&p, "device", &auth(1, 600), |_| async {})
            .await
            .unwrap_err();
        assert!(err.to_string().contains("access_denied"), "{err}");
        assert_eq!(p.polls.lock().len(), 2);
    }

    // Plausible wrong impl: keeps polling past the device-code lifetime.
    #[tokio::test]
    async fn device_login_gives_up_after_expiry() {
        let p = scripted(vec!["pending"; 10]);
        let err = run_device_login(&p, "device", &auth(5, 12), |_| async {})
            .await
            .unwrap_err();
        assert!(err.to_string().contains("expired"), "{err}");
        // Polls at t=5 and t=10; t=15 would exceed 12.
        assert_eq!(p.polls.lock().len(), 3);
    }

    // Plausible wrong impl: missing required option accepted, or defaults not applied.
    #[test]
    fn login_params_apply_defaults_and_reject_missing_required() {
        let p = scripted(vec![]);
        let m = find_login_method(&p, None).unwrap();
        let err = resolve_login_params(&m, &LoginParams::new()).unwrap_err();
        assert!(err.to_string().contains("start_url"), "{err}");
        let ok = resolve_login_params(
            &m,
            &LoginParams::from([("start_url".to_string(), "https://x".to_string())]),
        )
        .unwrap();
        assert_eq!(ok["region"], "us-east-1");
        assert!(find_login_method(&p, Some("nope")).is_err());
    }
}
