use std::collections::HashMap;
use std::sync::Arc;

use futures::future::BoxFuture;
use vkdg_connections::{TokenPair, TokenRefresher};
use vkdg_core::VkdgError;

use crate::ProviderAdapter;

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
                .map_err(|e| VkdgError::PluginError {
                    plugin_id: provider.to_owned(),
                    message: e.to_string(),
                })
        })
    }
}
