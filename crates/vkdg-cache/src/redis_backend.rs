//! Redis-backed exact-match cache.
//!
//! Activated with feature flag `redis` on vkdg-cache.
//! Compatible with Redis, Valkey, and DragonflyDB (all Redis-compatible).
//!
//! Keys are namespaced: `vkdg:cache:<key>`
//! TTL is set via Redis EXPIRE. Tenant invalidation is a stub in Phase D;
//! full per-tenant scoping requires a secondary index (Phase E).

use async_trait::async_trait;
use vkdg_core::VkdgError;

use crate::backend::{CacheBackend, CacheEntry, CacheResult};

pub struct RedisExactCache {
    client: redis::Client,
    key_prefix: String,
}

impl RedisExactCache {
    /// Create a new `RedisExactCache`.
    ///
    /// `redis_url` follows the `redis://[user:pass@]host[:port][/db]` format.
    /// `key_prefix` defaults to `"vkdg:cache"` when `None`.
    pub fn new(redis_url: &str, key_prefix: Option<&str>) -> Result<Self, VkdgError> {
        let client = redis::Client::open(redis_url)
            .map_err(|e| VkdgError::Internal(format!("redis connect: {e}")))?;
        Ok(Self {
            client,
            key_prefix: key_prefix.unwrap_or("vkdg:cache").to_string(),
        })
    }

    fn full_key(&self, key: &str) -> String {
        format!("{}:{}", self.key_prefix, key)
    }
}

#[async_trait]
impl CacheBackend for RedisExactCache {
    fn name(&self) -> &str {
        "redis-exact"
    }

    async fn lookup(&self, key: &str) -> Result<CacheResult, VkdgError> {
        use redis::AsyncCommands as _;
        let mut conn = self
            .client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| VkdgError::Internal(format!("redis conn: {e}")))?;
        let full = self.full_key(key);
        let val: Option<String> = conn
            .get(&full)
            .await
            .map_err(|e| VkdgError::Internal(format!("redis get: {e}")))?;
        match val {
            None => Ok(CacheResult::Miss),
            Some(json) => {
                let entry: CacheEntry = serde_json::from_str(&json)
                    .map_err(|e| VkdgError::Internal(format!("cache deserialize: {e}")))?;
                Ok(CacheResult::Hit(entry))
            }
        }
    }

    async fn store(&self, key: &str, entry: CacheEntry) -> Result<(), VkdgError> {
        use redis::AsyncCommands as _;
        let mut conn = self
            .client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| VkdgError::Internal(format!("redis conn: {e}")))?;
        let full = self.full_key(key);
        let json = serde_json::to_string(&entry)
            .map_err(|e| VkdgError::Internal(format!("cache serialize: {e}")))?;
        if let Some(ttl) = entry.ttl_secs {
            let _: () = conn
                .set_ex(&full, &json, ttl as u64)
                .await
                .map_err(|e| VkdgError::Internal(format!("redis setex: {e}")))?;
        } else {
            let _: () = conn
                .set(&full, &json)
                .await
                .map_err(|e| VkdgError::Internal(format!("redis set: {e}")))?;
        }
        Ok(())
    }

    async fn invalidate_tenant(&self, tenant_id: &str) -> Result<u64, VkdgError> {
        // Pattern scan is O(N) but acceptable for admin operations.
        // Full tenant-scoped invalidation requires a secondary index (Phase E).
        // For Phase D: log a warning and return 0.
        let _ = tenant_id; // suppress unused-variable warning
        tracing::warn!("redis tenant invalidation is a stub in Phase D — no keys deleted");
        Ok(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Plausible wrong impl: key_prefix is ignored and keys collide across
    // different prefixes, causing cross-tenant cache pollution.
    #[test]
    fn full_key_uses_prefix() {
        let cache = RedisExactCache {
            client: redis::Client::open("redis://127.0.0.1/").unwrap(),
            key_prefix: "vkdg:cache".into(),
        };
        assert_eq!(cache.full_key("abc"), "vkdg:cache:abc");
    }

    // Plausible wrong impl: custom prefix is ignored, always using the default.
    #[test]
    fn custom_prefix_is_respected() {
        let cache = RedisExactCache {
            client: redis::Client::open("redis://127.0.0.1/").unwrap(),
            key_prefix: "test:ns".into(),
        };
        assert_eq!(cache.full_key("x"), "test:ns:x");
    }

    // Plausible wrong impl: name() returns the wrong string, breaking backend
    // selection logic that uses the name to route requests.
    #[test]
    fn name_is_redis_exact() {
        let cache = RedisExactCache {
            client: redis::Client::open("redis://127.0.0.1/").unwrap(),
            key_prefix: "vkdg:cache".into(),
        };
        assert_eq!(cache.name(), "redis-exact");
    }
}
