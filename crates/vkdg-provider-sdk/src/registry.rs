use std::collections::HashMap;
use std::sync::Arc;

use futures::future::BoxFuture;
use vkdg_connections::{TokenPair, TokenRefresher};
use vkdg_core::VkdgError;

use crate::{ProviderAdapter, ProviderError};

/// Holds all registered provider adapters.
///
/// Populated at startup with built-in providers.
/// Operators can add custom providers via [`register`][Self::register].
pub struct ProviderRegistry {
    adapters: HashMap<String, Arc<dyn ProviderAdapter>>,
}

impl ProviderRegistry {
    pub fn empty() -> Self {
        Self {
            adapters: HashMap::new(),
        }
    }

    /// Register a provider adapter. [`id()`][ProviderAdapter::id] is used as the key.
    /// Overwrites if already registered.
    pub fn register(&mut self, adapter: Arc<dyn ProviderAdapter>) {
        self.adapters.insert(adapter.id().to_string(), adapter);
    }

    /// Register an adapter only if its id is free.
    ///
    /// Used for out-of-tree plugins: an adapter receives the credentials of every
    /// connection that names its id, so a plugin claiming `anthropic` or `kiro`
    /// must not silently replace the built-in one.
    pub fn try_register(&mut self, adapter: Arc<dyn ProviderAdapter>) -> Result<(), String> {
        let id = adapter.id().to_owned();
        if self.adapters.contains_key(&id) {
            return Err(format!("provider id '{id}' is already registered"));
        }
        self.adapters.insert(id, adapter);
        Ok(())
    }

    /// Look up by provider id (e.g. "anthropic", "openai", "gemini").
    pub fn get(&self, id: &str) -> Option<Arc<dyn ProviderAdapter>> {
        self.adapters.get(id).cloned()
    }

    /// All registered provider ids.
    pub fn ids(&self) -> Vec<&str> {
        self.adapters.keys().map(|s| s.as_str()).collect()
    }

    /// Number of registered providers.
    pub fn len(&self) -> usize {
        self.adapters.len()
    }

    pub fn is_empty(&self) -> bool {
        self.adapters.is_empty()
    }
}

/// Dispatches account refresh to the owning plugin's [`OAuthProvider`](crate::OAuthProvider).
impl TokenRefresher for ProviderRegistry {
    fn refresh<'a>(
        &'a self,
        provider: &'a str,
        refresh_token: &'a str,
        extra: &'a HashMap<String, String>,
    ) -> BoxFuture<'a, vkdg_core::Result<TokenPair>> {
        Box::pin(async move {
            let adapter = self.get(provider).ok_or_else(|| VkdgError::ConfigInvalid {
                field: "account.provider".into(),
                message: format!("no provider plugin registered for '{provider}'"),
            })?;
            let oauth = adapter.oauth().ok_or_else(|| VkdgError::ConfigInvalid {
                field: "account.provider".into(),
                message: format!("provider '{provider}' does not support OAuth refresh"),
            })?;
            oauth
                .refresh_token(refresh_token, extra)
                .await
                .map_err(|e| match e {
                    // Keep revocation typed so the credential manager can stop
                    // retrying instead of hitting the refresh endpoint every request.
                    ProviderError::CredentialRevoked { status, message } => {
                        VkdgError::CredentialRevoked { status, message }
                    }
                    other => VkdgError::PluginError {
                        plugin_id: provider.to_owned(),
                        message: other.to_string(),
                    },
                })
        })
    }
}

#[cfg(test)]
mod try_register_tests {
    use super::*;
    use crate::{PreparedRequest, ProviderError};
    use vkdg_connections::{ConnectionConfig, Credential};
    use vkdg_operations::Operation;

    struct Named(&'static str, &'static str);

    impl ProviderAdapter for Named {
        fn id(&self) -> &str {
            self.0
        }
        fn display_name(&self) -> &str {
            self.1
        }
        fn prepare(
            &self,
            _: &Operation,
            _: &ConnectionConfig,
            _: &Credential,
        ) -> Result<PreparedRequest, ProviderError> {
            Err(ProviderError::UnsupportedOperation)
        }
    }

    // A plugin claiming a built-in id would receive that provider's credentials.
    #[test]
    fn taken_id_is_refused_and_the_original_stays() {
        let mut r = ProviderRegistry::empty();
        r.register(Arc::new(Named("anthropic", "built-in")));
        let err = r
            .try_register(Arc::new(Named("anthropic", "impostor")))
            .unwrap_err();
        assert!(err.contains("anthropic"), "{err}");
        assert_eq!(r.get("anthropic").unwrap().display_name(), "built-in");
        assert!(r
            .try_register(Arc::new(Named("community", "plugin")))
            .is_ok());
        assert!(r.get("community").is_some());
    }
}
