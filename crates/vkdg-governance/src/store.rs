//! In-memory virtual key store.
//! Phase E: SQLite persistence.

use std::collections::HashMap;
use std::sync::Arc;

use sha2::{Digest, Sha256};
use std::sync::atomic::Ordering;
use tokio::sync::RwLock;

use crate::key::{KeyScope, VirtualKey, VirtualKeyId};

pub struct VirtualKeyStore {
    /// Keyed by `token_hash` for O(1) lookup on the hot path.
    by_hash: RwLock<HashMap<String, Arc<VirtualKey>>>,
    /// Keyed by id for admin operations.
    by_id: RwLock<HashMap<VirtualKeyId, Arc<VirtualKey>>>,
}

impl VirtualKeyStore {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            by_hash: RwLock::new(HashMap::new()),
            by_id: RwLock::new(HashMap::new()),
        })
    }

    /// Authenticate a raw bearer token. Returns the key or `None`.
    ///
    /// Hot path: single `RwLock` read + `HashMap` lookup.
    pub async fn authenticate(&self, raw_token: &str) -> Option<Arc<VirtualKey>> {
        let hash = format!("{:x}", Sha256::digest(raw_token.as_bytes()));
        let guard = self.by_hash.read().await;
        let key = guard.get(&hash)?.clone();
        if key.is_revoked() {
            None
        } else {
            Some(key)
        }
    }

    /// Create a new key. Returns `(Arc<VirtualKey>, raw_token)`.
    pub async fn create(
        &self,
        name: String,
        tenant_id: String,
        scopes: Vec<KeyScope>,
        budget_microdollars: Option<u64>,
    ) -> (Arc<VirtualKey>, String) {
        let (key, raw) = VirtualKey::new(name, tenant_id, scopes, budget_microdollars);
        let key = Arc::new(key);
        let mut by_hash = self.by_hash.write().await;
        let mut by_id = self.by_id.write().await;
        by_hash.insert(key.token_hash.clone(), Arc::clone(&key));
        by_id.insert(key.id.clone(), Arc::clone(&key));
        (key, raw)
    }

    /// Revoke a key by id. Returns `true` if the key was found.
    pub async fn revoke(&self, id: &VirtualKeyId) -> bool {
        let guard = self.by_id.read().await;
        if let Some(key) = guard.get(id) {
            key.revoked.store(true, Ordering::Release);
            true
        } else {
            false
        }
    }

    pub async fn list(&self) -> Vec<Arc<VirtualKey>> {
        self.by_id.read().await.values().cloned().collect()
    }
}

impl Default for VirtualKeyStore {
    fn default() -> Self {
        Self {
            by_hash: RwLock::new(HashMap::new()),
            by_id: RwLock::new(HashMap::new()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Plausible wrong impl: `authenticate` returns key after revoke.
    #[tokio::test]
    async fn revoked_key_not_authenticated() {
        let store = VirtualKeyStore::new();
        let (key, raw) = store.create("k".into(), "t".into(), vec![], None).await;
        store.revoke(&key.id).await;
        let result = store.authenticate(&raw).await;
        assert!(result.is_none(), "revoked key must not authenticate");
    }
}
