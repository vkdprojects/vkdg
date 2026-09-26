//! Cache plugin infrastructure for VKDG.
//!
//! The CacheBackend trait is the extension point -- any backend (SQLite, Redis,
//! semantic vector) implements it. The gateway calls lookup() before the
//! provider and store() after. Bypass rules (multi-turn, tool calls,
//! x-vkdg-cache: none header) are enforced by the gateway core, not here.

pub mod backend;
pub mod exact;
pub mod key;

pub use backend::{CacheBackend, CacheEntry, CacheResult};
pub use exact::SqliteExactCache;
#[cfg(feature = "redis")]
pub mod redis_backend;
#[cfg(feature = "redis")]
pub use redis_backend::RedisExactCache;
pub use key::cache_key;
