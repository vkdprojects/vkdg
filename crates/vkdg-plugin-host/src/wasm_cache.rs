//! Bridges a WASM component to the [`CacheBackend`] trait.
//!
//! A cache backend is the extension point with the widest range of plausible
//! implementations: Redis, Memcached, a semantic index, a shared team cache. None
//! of those belong in the gateway binary, and none should need a gateway release.
//!
//! The plugin has no network access, so a backend that talks to a remote store
//! cannot be implemented as WASM today — that needs a host-provided key/value
//! import, which is a deliberate future decision rather than an oversight. What
//! works now is any cache whose state fits in the plugin: fingerprinting,
//! normalisation, policy over an in-memory table.
//!
//! Cache is an optimisation, so every failure path degrades to a miss. A broken
//! plugin must never turn a cacheable request into a failed one.

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use vkdg_cache::{CacheBackend, CacheEntry, CacheResult};
use vkdg_core::VkdgError;

use crate::{PluginManifest, WasmPluginInstance};

mod export {
    pub const NAME: &str = "name";
    pub const LOOKUP: &str = "lookup";
    pub const STORE: &str = "store";
    pub const INVALIDATE_TENANT: &str = "invalidate-tenant";
}

/// What a plugin returns from `lookup`.
#[derive(Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
enum LookupJson {
    Hit { entry: EntryJson },
    Miss,
}

/// A cache entry as it crosses the boundary.
#[derive(Serialize, Deserialize)]
struct EntryJson {
    response_json: String,
    model: String,
    cached_at_secs: u64,
    #[serde(default)]
    ttl_secs: Option<u32>,
    #[serde(default = "default_cache_type")]
    cache_type: String,
    #[serde(default)]
    hit_score: Option<f32>,
}

fn default_cache_type() -> String {
    "exact".to_owned()
}

impl From<&CacheEntry> for EntryJson {
    fn from(e: &CacheEntry) -> Self {
        Self {
            response_json: e.response_json.clone(),
            model: e.model.clone(),
            cached_at_secs: e.cached_at_secs,
            ttl_secs: e.ttl_secs,
            cache_type: e.cache_type.clone(),
            hit_score: e.hit_score,
        }
    }
}

impl From<EntryJson> for CacheEntry {
    fn from(e: EntryJson) -> Self {
        Self {
            response_json: e.response_json,
            model: e.model,
            cached_at_secs: e.cached_at_secs,
            ttl_secs: e.ttl_secs,
            cache_type: e.cache_type,
            hit_score: e.hit_score,
        }
    }
}

/// What the gateway hands `store`.
#[derive(Serialize)]
struct StoreInput<'a> {
    key: &'a str,
    entry: EntryJson,
}

/// A cache backend backed by a WASM component.
pub struct WasmCache {
    name: String,
    instance: Arc<WasmPluginInstance>,
}

impl WasmCache {
    pub fn new(instance: Arc<WasmPluginInstance>, manifest: &PluginManifest) -> Self {
        let name = instance
            .call_string_fn(export::NAME, "")
            .unwrap_or_else(|_| manifest.id.0.clone());
        Self { name, instance }
    }
}

#[async_trait]
impl CacheBackend for WasmCache {
    fn name(&self) -> &str {
        &self.name
    }

    async fn lookup(&self, key: &str) -> Result<CacheResult, VkdgError> {
        // Any failure is a miss: the request then goes upstream, which is correct
        // but slower, rather than failing.
        let Ok(out) = self.instance.call_string_fn(export::LOOKUP, key) else {
            return Ok(CacheResult::Miss);
        };
        match serde_json::from_str::<LookupJson>(&out) {
            Ok(LookupJson::Hit { entry }) => Ok(CacheResult::Hit(entry.into())),
            // A malformed answer is treated as a miss, never as a hit with garbage.
            Ok(LookupJson::Miss) | Err(_) => Ok(CacheResult::Miss),
        }
    }

    async fn store(&self, key: &str, entry: CacheEntry) -> Result<(), VkdgError> {
        let payload = serde_json::to_string(&StoreInput {
            key,
            entry: (&entry).into(),
        })
        .map_err(|e| VkdgError::Internal(e.to_string()))?;

        // A store that fails costs a future cache hit, not this request, so the
        // error is surfaced for logging but the call already succeeded upstream.
        self.instance
            .call_string_fn(export::STORE, &payload)
            .map(|_| ())
            .map_err(|e| VkdgError::PluginError {
                plugin_id: self.name.clone(),
                message: e,
            })
    }

    async fn invalidate_tenant(&self, tenant_id: &str) -> Result<u64, VkdgError> {
        let out = self
            .instance
            .call_string_fn(export::INVALIDATE_TENANT, tenant_id)
            .map_err(|e| VkdgError::PluginError {
                plugin_id: self.name.clone(),
                message: e,
            })?;
        // The count is advisory; an unparsable answer means "unknown", not an error,
        // since the invalidation itself may well have happened.
        Ok(out.trim().parse::<u64>().unwrap_or(0))
    }
}
