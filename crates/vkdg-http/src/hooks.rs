//! Per-route plugin hooks.
//!
//! A route's `hooks.auth` names plugins that run after the client's vkdg key
//! was accepted. They can only narrow access: a missing plugin, a plugin error,
//! or anything but an explicit allow denies the request.
//!
//! Plugins see who is calling (key id, tenant, address), never the client's
//! credentials: headers are not forwarded, so a community plugin cannot harvest
//! gateway keys.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;

use serde::Serialize;
use vkdg_core::VkdgError;

/// What an auth hook is told about a request.
#[derive(Debug, Clone, Serialize)]
pub struct HookRequest {
    pub key_id: String,
    pub tenant_id: String,
    pub client_ip: Option<IpAddr>,
    pub model: String,
    pub route_id: String,
}

/// An auth decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookVerdict {
    Allow,
    /// Reason is logged, not sent to the client.
    Deny(String),
}

/// A route auth hook. Implementations may block (WASM calls do); the pipeline
/// runs them off the async workers.
pub trait AuthHook: Send + Sync {
    fn check(&self, request: &HookRequest) -> HookVerdict;
}

/// Auth hooks by plugin id, as loaded at startup.
#[derive(Default, Clone)]
pub struct HookRegistry {
    auth: HashMap<String, Arc<dyn AuthHook>>,
}

impl HookRegistry {
    pub fn register_auth(&mut self, id: impl Into<String>, hook: Arc<dyn AuthHook>) {
        self.auth.insert(id.into(), hook);
    }

    pub fn has_auth(&self, id: &str) -> bool {
        self.auth.contains_key(id)
    }

    /// Run `ids` in order; the first denial wins.
    pub async fn authorize(&self, ids: &[String], request: HookRequest) -> Result<(), VkdgError> {
        for id in ids {
            let Some(hook) = self.auth.get(id).cloned() else {
                tracing::warn!(hook = %id, route = %request.route_id, "route auth hook not installed; denying");
                return Err(VkdgError::Unauthorized);
            };
            // Plugin calls block (a WASM instance runs to completion), so keep
            // them off the async workers.
            let req = request.clone();
            let verdict = tokio::task::spawn_blocking(move || hook.check(&req))
                .await
                .unwrap_or_else(|e| HookVerdict::Deny(format!("hook panicked: {e}")));
            if let HookVerdict::Deny(reason) = verdict {
                tracing::info!(hook = %id, route = %request.route_id, %reason, "route auth hook denied request");
                return Err(VkdgError::Unauthorized);
            }
        }
        Ok(())
    }
}
