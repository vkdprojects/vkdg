//! CacheBackend trait -- the extension point for all cache plugins.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use vkdg_core::VkdgError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheEntry {
    /// Serialized response JSON
    pub response_json: String,
    pub model: String,
    pub cached_at_secs: u64,
    pub ttl_secs: Option<u32>,
    /// "exact" | "semantic"
    pub cache_type: String,
    pub hit_score: Option<f32>,
}

#[derive(Debug)]
pub enum CacheResult {
    Hit(CacheEntry),
    Miss,
}

#[async_trait]
pub trait CacheBackend: Send + Sync {
    fn name(&self) -> &str;

    async fn lookup(&self, key: &str) -> Result<CacheResult, VkdgError>;

    async fn store(&self, key: &str, entry: CacheEntry) -> Result<(), VkdgError>;

    async fn invalidate_tenant(&self, tenant_id: &str) -> Result<u64, VkdgError>;
}
