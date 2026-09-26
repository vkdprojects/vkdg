//! SQLite-backed exact-match cache.
//!
//! Default backend when no cache is explicitly configured.
//! Zero infra -- a single SQLite file, or :memory: for tests.
//! Uses WAL mode for concurrent reads.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use rusqlite::{params, Connection};
use tokio::task::spawn_blocking;

use vkdg_core::VkdgError;

use crate::backend::{CacheBackend, CacheEntry, CacheResult};

pub struct SqliteExactCache {
    conn: Arc<tokio::sync::Mutex<Connection>>,
    name: String,
}

impl SqliteExactCache {
    pub fn open(path: &str) -> Result<Self, VkdgError> {
        let conn = Connection::open(path)
            .map_err(|e| VkdgError::Internal(format!("cache sqlite open: {e}")))?;
        conn.execute_batch(
            "
            PRAGMA journal_mode=WAL;
            PRAGMA synchronous=NORMAL;
            CREATE TABLE IF NOT EXISTS cache (
                key         TEXT PRIMARY KEY,
                tenant_id   TEXT NOT NULL DEFAULT '',
                model       TEXT NOT NULL,
                response    TEXT NOT NULL,
                cached_at   INTEGER NOT NULL,
                ttl_secs    INTEGER,
                cache_type  TEXT NOT NULL DEFAULT 'exact'
            );
            CREATE INDEX IF NOT EXISTS idx_tenant ON cache(tenant_id);
            CREATE INDEX IF NOT EXISTS idx_expiry  ON cache(cached_at, ttl_secs);
        ",
        )
        .map_err(|e| VkdgError::Internal(format!("cache init: {e}")))?;
        Ok(Self {
            conn: Arc::new(tokio::sync::Mutex::new(conn)),
            name: "sqlite-exact".into(),
        })
    }

    pub fn in_memory() -> Result<Self, VkdgError> {
        Self::open(":memory:")
    }
}

#[async_trait]
impl CacheBackend for SqliteExactCache {
    fn name(&self) -> &str {
        &self.name
    }

    async fn lookup(&self, key: &str) -> Result<CacheResult, VkdgError> {
        let conn = Arc::clone(&self.conn);
        let key = key.to_string();
        let now = unix_secs();
        spawn_blocking(move || {
            let conn = conn.blocking_lock();
            let mut stmt = conn
                .prepare(
                    "SELECT model, response, cached_at, ttl_secs, cache_type \
                     FROM cache WHERE key = ?1 \
                     AND (ttl_secs IS NULL OR cached_at + ttl_secs > ?2)",
                )
                .map_err(|e| VkdgError::Internal(e.to_string()))?;
            let mut rows = stmt
                .query(params![key, now as i64])
                .map_err(|e| VkdgError::Internal(e.to_string()))?;
            if let Some(row) = rows.next().map_err(|e| VkdgError::Internal(e.to_string()))? {
                Ok(CacheResult::Hit(CacheEntry {
                    model: row.get(0).unwrap_or_default(),
                    response_json: row.get(1).unwrap_or_default(),
                    cached_at_secs: row.get::<_, i64>(2).unwrap_or(0) as u64,
                    ttl_secs: row
                        .get::<_, Option<i64>>(3)
                        .unwrap_or(None)
                        .map(|v| v as u32),
                    cache_type: row.get(4).unwrap_or_else(|_| "exact".into()),
                    hit_score: None,
                }))
            } else {
                Ok(CacheResult::Miss)
            }
        })
        .await
        .map_err(|e| VkdgError::Internal(e.to_string()))?
    }

    async fn store(&self, key: &str, entry: CacheEntry) -> Result<(), VkdgError> {
        let conn = Arc::clone(&self.conn);
        let key = key.to_string();
        let now = unix_secs() as i64;
        spawn_blocking(move || {
            let conn = conn.blocking_lock();
            conn.execute(
                "INSERT OR REPLACE INTO cache \
                 (key, model, response, cached_at, ttl_secs, cache_type) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    key,
                    entry.model,
                    entry.response_json,
                    now,
                    entry.ttl_secs.map(|v| v as i64),
                    entry.cache_type,
                ],
            )
            .map_err(|e| VkdgError::Internal(e.to_string()))?;
            Ok(())
        })
        .await
        .map_err(|e| VkdgError::Internal(e.to_string()))?
    }

    async fn invalidate_tenant(&self, tenant_id: &str) -> Result<u64, VkdgError> {
        let conn = Arc::clone(&self.conn);
        let tid = tenant_id.to_string();
        spawn_blocking(move || {
            let conn = conn.blocking_lock();
            let n = conn
                .execute("DELETE FROM cache WHERE tenant_id = ?1", params![tid])
                .map_err(|e| VkdgError::Internal(e.to_string()))?;
            Ok(n as u64)
        })
        .await
        .map_err(|e| VkdgError::Internal(e.to_string()))?
    }
}

fn unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn make_cache() -> SqliteExactCache {
        SqliteExactCache::in_memory().unwrap()
    }

    fn make_entry(model: &str) -> CacheEntry {
        CacheEntry {
            response_json: r#"{"content":"hello"}"#.to_string(),
            model: model.to_string(),
            cached_at_secs: 0,
            ttl_secs: None,
            cache_type: "exact".into(),
            hit_score: None,
        }
    }

    // Plausible wrong impl: store succeeds but lookup returns Miss (key mismatch)
    #[tokio::test]
    async fn store_and_lookup_hit() {
        let c = make_cache().await;
        c.store("key1", make_entry("claude-3-5-haiku")).await.unwrap();
        assert!(matches!(c.lookup("key1").await.unwrap(), CacheResult::Hit(_)));
    }

    // Plausible wrong impl: different keys share the same row (collision)
    #[tokio::test]
    async fn different_keys_independent() {
        let c = make_cache().await;
        c.store("key1", make_entry("m1")).await.unwrap();
        assert!(matches!(c.lookup("key2").await.unwrap(), CacheResult::Miss));
    }

    // Plausible wrong impl: expired entries returned as hits
    #[tokio::test]
    async fn expired_entry_is_miss() {
        let c = make_cache().await;
        c.store("key1", make_entry("m1")).await.unwrap();
        // Force cached_at=0, ttl_secs=1 so it's expired relative to any real timestamp
        {
            let conn = c.conn.lock().await;
            conn.execute(
                "UPDATE cache SET cached_at = 0, ttl_secs = 1 WHERE key = 'key1'",
                [],
            )
            .unwrap();
        }
        assert!(matches!(c.lookup("key1").await.unwrap(), CacheResult::Miss));
    }

    // Plausible wrong impl: invalidate_tenant removes entries from other tenants
    #[tokio::test]
    async fn invalidate_tenant_scoped() {
        let c = make_cache().await;
        {
            let conn = c.conn.lock().await;
            conn.execute(
                "INSERT INTO cache (key, tenant_id, model, response, cached_at, cache_type) \
                 VALUES ('k1', 'tenant-a', 'm', '{}', 0, 'exact'), \
                        ('k2', 'tenant-b', 'm', '{}', 0, 'exact')",
                [],
            )
            .unwrap();
        }
        c.invalidate_tenant("tenant-a").await.unwrap();
        // k1 gone
        assert!(matches!(c.lookup("k1").await.unwrap(), CacheResult::Miss));
        // k2 still present -- update cached_at to far future so TTL check passes
        {
            let conn = c.conn.lock().await;
            conn.execute(
                "UPDATE cache SET cached_at = 9999999999, ttl_secs = NULL WHERE key = 'k2'",
                [],
            )
            .unwrap();
        }
        assert!(matches!(c.lookup("k2").await.unwrap(), CacheResult::Hit(_)));
    }
}
