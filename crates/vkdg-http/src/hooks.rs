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

type AuthTable = HashMap<String, Arc<dyn AuthHook>>;

/// Auth hooks by plugin id. Swapped whole by [`HookRegistry::replace`] when
/// plugins are installed or removed, so the next request sees the change.
#[derive(Default)]
pub struct HookRegistry {
    auth: parking_lot::RwLock<Arc<AuthTable>>,
}

impl HookRegistry {
    pub fn register_auth(&mut self, id: impl Into<String>, hook: Arc<dyn AuthHook>) {
        Arc::make_mut(self.auth.get_mut()).insert(id.into(), hook);
    }

    pub fn has_auth(&self, id: &str) -> bool {
        self.auth.read().contains_key(id)
    }

    /// Take `other`'s hooks. Requests already authorizing finish on the old set.
    pub fn replace(&self, other: HookRegistry) {
        *self.auth.write() = other.auth.into_inner();
    }

    /// Run `ids` in order; the first denial wins.
    pub async fn authorize(&self, ids: &[String], request: HookRequest) -> Result<(), VkdgError> {
        let table = Arc::clone(&self.auth.read());
        for id in ids {
            let Some(hook) = table.get(id).cloned() else {
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

#[cfg(test)]
mod tests {
    use super::*;

    struct Always(HookVerdict);
    impl AuthHook for Always {
        fn check(&self, _: &HookRequest) -> HookVerdict {
            self.0.clone()
        }
    }

    fn req() -> HookRequest {
        HookRequest {
            key_id: "k".into(),
            tenant_id: "t".into(),
            client_ip: None,
            model: "m".into(),
            route_id: "r".into(),
        }
    }

    // Installing a plugin used to need a restart: the registry was built once.
    // Removing one must keep failing closed for routes that still name it.
    #[tokio::test]
    async fn replaced_hooks_apply_to_the_next_request_and_removal_denies() {
        let live = Arc::new(HookRegistry::default());
        let ids = ["gate".to_string()];
        assert!(
            live.authorize(&ids, req()).await.is_err(),
            "absent hook denies"
        );

        let mut installed = HookRegistry::default();
        installed.register_auth("gate", Arc::new(Always(HookVerdict::Allow)));
        live.replace(installed);
        assert!(live.authorize(&ids, req()).await.is_ok());

        live.replace(HookRegistry::default());
        assert!(
            live.authorize(&ids, req()).await.is_err(),
            "removed hook denies"
        );
    }
}
