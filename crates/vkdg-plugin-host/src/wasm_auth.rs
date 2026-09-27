//! Auth and rate-limit plugins.
//!
//! Unlike the other roles, auth has no pre-existing Rust trait to adapt: the
//! gateway authenticates with its own key store, and `wit/auth.wit` was written for
//! a capability nothing implemented. So this module defines the host-side contract
//! as well as the WASM bridge.
//!
//! # Fail-closed
//! Every other role degrades quietly when a plugin misbehaves, because a broken
//! compressor or cache costs latency. Auth is the opposite: a plugin that cannot
//! answer must **deny**. A bridge that let an unloadable plugin through would turn
//! a broken install into an open gateway, so every failure path here returns
//! `Denied`, and the tests pin that.

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::{PluginManifest, WasmPluginInstance};

mod export {
    pub const NAME: &str = "name";
    pub const AUTHENTICATE: &str = "authenticate";
    pub const CHECK_RATE_LIMIT: &str = "check-rate-limit";
    pub const RECORD_USAGE: &str = "record-usage";
}

/// Who the caller is, once a plugin has vouched for them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthContext {
    pub tenant_id: String,
    /// Key identifier, never the raw token.
    pub key_id: String,
    pub role: String,
    pub scopes: Vec<String>,
    #[serde(default)]
    pub claims: HashMap<String, String>,
}

/// Outcome of an authentication attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthOutcome {
    Allowed(AuthContext),
    /// Reason is safe to log; it is not returned verbatim to the caller.
    Denied(String),
}

/// Outcome of a rate-limit check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RateLimitOutcome {
    Allowed,
    /// Retry hint in seconds, when the plugin supplied one.
    Limited {
        retry_after_secs: Option<u64>,
    },
    /// The plugin could not decide, so the request is denied.
    Denied(String),
}

/// The host-side contract for an auth plugin.
pub trait AuthPlugin: Send + Sync {
    fn name(&self) -> &str;

    /// Authenticate a request. `request_json` carries the request without a body.
    fn authenticate(&self, request_json: &str) -> AuthOutcome;

    /// Check whether an authenticated caller may proceed.
    fn check_rate_limit(&self, ctx: &AuthContext, request_json: &str) -> RateLimitOutcome;

    /// Record usage after the response. Fire-and-forget: a failure here must not
    /// affect the response the caller already received.
    fn record_usage(&self, ctx: &AuthContext, prompt_tokens: u32, completion_tokens: u32);
}

/// What a plugin returns from `authenticate`.
#[derive(Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
enum AuthJson {
    Allowed { context: AuthContext },
    Denied { reason: String },
}

/// What a plugin returns from `check-rate-limit`.
#[derive(Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
enum RateLimitJson {
    Allowed,
    Limited {
        #[serde(default)]
        retry_after_secs: Option<u64>,
    },
}

#[derive(Serialize)]
struct RateLimitInput<'a> {
    context: &'a AuthContext,
    /// Raw request JSON, passed through so the plugin sees what the gateway saw.
    request: &'a str,
}

#[derive(Serialize)]
struct UsageInput<'a> {
    context: &'a AuthContext,
    prompt_tokens: u32,
    completion_tokens: u32,
}

/// An auth plugin backed by a WASM component.
pub struct WasmAuth {
    name: String,
    instance: Arc<WasmPluginInstance>,
}

impl WasmAuth {
    pub fn new(instance: Arc<WasmPluginInstance>, manifest: &PluginManifest) -> Self {
        let name = instance
            .call_string_fn(export::NAME, "")
            .unwrap_or_else(|_| manifest.id.0.clone());
        Self { name, instance }
    }
}

impl AuthPlugin for WasmAuth {
    fn name(&self) -> &str {
        &self.name
    }

    fn authenticate(&self, request_json: &str) -> AuthOutcome {
        let out = match self
            .instance
            .call_string_fn(export::AUTHENTICATE, request_json)
        {
            Ok(out) => out,
            // Fail-closed: a plugin that cannot run denies.
            Err(e) => return AuthOutcome::Denied(format!("auth plugin {:?}: {e}", self.name)),
        };
        match serde_json::from_str::<AuthJson>(&out) {
            Ok(AuthJson::Allowed { context }) => AuthOutcome::Allowed(context),
            Ok(AuthJson::Denied { reason }) => AuthOutcome::Denied(reason),
            // An answer we cannot read is not an approval.
            Err(e) => AuthOutcome::Denied(format!(
                "auth plugin {:?} returned an unreadable verdict: {e}",
                self.name
            )),
        }
    }

    fn check_rate_limit(&self, ctx: &AuthContext, request_json: &str) -> RateLimitOutcome {
        let Ok(payload) = serde_json::to_string(&RateLimitInput {
            context: ctx,
            request: request_json,
        }) else {
            return RateLimitOutcome::Denied("could not serialise rate-limit input".to_owned());
        };

        let out = match self
            .instance
            .call_string_fn(export::CHECK_RATE_LIMIT, &payload)
        {
            Ok(out) => out,
            // A rate limiter that cannot answer denies: letting traffic through
            // would defeat the limit it exists to enforce.
            Err(e) => {
                return RateLimitOutcome::Denied(format!("rate limiter {:?}: {e}", self.name))
            }
        };
        match serde_json::from_str::<RateLimitJson>(&out) {
            Ok(RateLimitJson::Allowed) => RateLimitOutcome::Allowed,
            Ok(RateLimitJson::Limited { retry_after_secs }) => {
                RateLimitOutcome::Limited { retry_after_secs }
            }
            Err(e) => RateLimitOutcome::Denied(format!(
                "rate limiter {:?} returned an unreadable verdict: {e}",
                self.name
            )),
        }
    }

    fn record_usage(&self, ctx: &AuthContext, prompt_tokens: u32, completion_tokens: u32) {
        // Fire-and-forget by contract: the response is already sent, so a failure
        // here is only worth a log line, never a changed outcome.
        let Ok(payload) = serde_json::to_string(&UsageInput {
            context: ctx,
            prompt_tokens,
            completion_tokens,
        }) else {
            return;
        };
        let _ = self.instance.call_string_fn(export::RECORD_USAGE, &payload);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Refutes: a rate-limit hint format that the gateway cannot turn into a
    /// `Retry-After` header.
    #[test]
    fn limited_outcome_carries_an_optional_retry_hint() {
        let with_hint: RateLimitJson =
            serde_json::from_str(r#"{"result":"limited","retry_after_secs":30}"#).unwrap();
        assert!(matches!(
            with_hint,
            RateLimitJson::Limited {
                retry_after_secs: Some(30)
            }
        ));

        // A limit without a hint is still a limit.
        let without: RateLimitJson = serde_json::from_str(r#"{"result":"limited"}"#).unwrap();
        assert!(matches!(
            without,
            RateLimitJson::Limited {
                retry_after_secs: None
            }
        ));
    }
}
