//! Persisted virtual key store (SQLite, `0600`).
//!
//! The data plane authenticates every request through [`VirtualKeyStore::authenticate`].
//! Lookups hit a small in-process cache; a miss or an expired entry re-reads the
//! row by its indexed `token_hash`. That bounds how long a key revoked by another
//! process (`vkdg keys revoke`, a second gateway on the same file) keeps working
//! to the cache TTL, instead of "until restart".

use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension};

use crate::key::{hash_token, KeyScope, VirtualKey, VirtualKeyId};

/// How long a successful lookup is trusted before re-reading SQLite.
pub const DEFAULT_CACHE_TTL: Duration = Duration::from_secs(5);

/// `last_used_at` is written at most this often per key, so the hot path does
/// not turn every request into a SQLite write.
const LAST_USED_RESOLUTION: chrono::Duration = chrono::Duration::seconds(60);

#[derive(Debug, thiserror::Error)]
pub enum KeyStoreError {
    #[error("key store {op}: {source}")]
    Sqlite {
        op: &'static str,
        #[source]
        source: rusqlite::Error,
    },
    #[error("key store {op} {path}: {source}")]
    Io {
        op: &'static str,
        path: String,
        #[source]
        source: std::io::Error,
    },
}

pub type Result<T, E = KeyStoreError> = std::result::Result<T, E>;

struct Cached {
    key: VirtualKey,
    at: Instant,
}

pub struct VirtualKeyStore {
    conn: Mutex<Connection>,
    /// Keyed by `token_hash`. Holds hits only: caching misses would let anyone on
    /// a public listener grow this map without bound with random tokens. A miss
    /// costs one indexed SELECT on `token_hash`.
    cache: Mutex<HashMap<String, Cached>>,
    ttl: Duration,
}

const SELECT: &str = "SELECT id, name, tenant_id, token_hash, prefix, scopes, created_at, \
                      last_used_at, revoked_at FROM keys";

impl VirtualKeyStore {
    /// Open (or create) the store at `path`. The file is created `0600` on Unix
    /// before any row is written.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(|e| io_err("create dir", parent, e))?;
        }
        create_private_file(path)?;
        let conn = Connection::open(path).map_err(sql("open"))?;
        Self::init(conn, DEFAULT_CACHE_TTL)
    }

    pub fn in_memory() -> Result<Self> {
        Self::init(
            Connection::open_in_memory().map_err(sql("open"))?,
            DEFAULT_CACHE_TTL,
        )
    }

    /// Override the lookup cache TTL (tests use zero to observe every write).
    #[must_use]
    pub fn with_cache_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = ttl;
        self
    }

    fn init(conn: Connection, ttl: Duration) -> Result<Self> {
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA busy_timeout = 2000;
             CREATE TABLE IF NOT EXISTS keys (
                 id           TEXT PRIMARY KEY,
                 name         TEXT NOT NULL,
                 tenant_id    TEXT NOT NULL,
                 token_hash   TEXT NOT NULL UNIQUE,
                 prefix       TEXT NOT NULL,
                 scopes       TEXT NOT NULL,
                 created_at   TEXT NOT NULL,
                 last_used_at TEXT,
                 revoked_at   TEXT
             );",
        )
        .map_err(sql("init"))?;
        Ok(Self {
            conn: Mutex::new(conn),
            cache: Mutex::new(HashMap::new()),
            ttl,
        })
    }

    /// Mint and persist a key. Returns the record and the raw token (shown once).
    pub fn create(
        &self,
        name: &str,
        tenant_id: &str,
        scopes: Vec<KeyScope>,
    ) -> Result<(VirtualKey, String)> {
        let (key, raw) = VirtualKey::new(name.to_owned(), tenant_id.to_owned(), scopes);
        self.conn
            .lock()
            .execute(
                "INSERT INTO keys (id, name, tenant_id, token_hash, prefix, scopes, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    key.id.0,
                    key.name,
                    key.tenant_id,
                    key.token_hash,
                    key.prefix,
                    encode_scopes(&key.scopes),
                    key.created_at.to_rfc3339(),
                ],
            )
            .map_err(sql("insert"))?;
        Ok((key, raw))
    }

    /// Resolve a raw token to an active key. `None` for unknown or revoked tokens.
    ///
    /// Storage errors fail closed: they are logged by the caller as `None` would
    /// be, so a broken database never lets a request through.
    pub fn authenticate(&self, raw_token: &str) -> Result<Option<VirtualKey>> {
        let hash = hash_token(raw_token);
        let now = Instant::now();
        {
            let mut cache = self.cache.lock();
            match cache.get(&hash) {
                Some(hit) if now.duration_since(hit.at) < self.ttl => {
                    return Ok(Some(hit.key.clone()));
                }
                // Expired: drop it so the map only ever holds live, valid keys.
                Some(_) => {
                    cache.remove(&hash);
                }
                None => {}
            }
        }

        let Some(mut key) = self.get_by_hash(&hash)?.filter(|k| !k.is_revoked()) else {
            return Ok(None);
        };
        self.touch(&mut key)?;
        self.cache.lock().insert(
            hash,
            Cached {
                key: key.clone(),
                at: now,
            },
        );
        Ok(Some(key))
    }

    /// Revoke by id. Returns `false` if no such key exists. Revoking twice keeps
    /// the first timestamp.
    pub fn revoke(&self, id: &VirtualKeyId) -> Result<bool> {
        let changed = self
            .conn
            .lock()
            .execute(
                "UPDATE keys SET revoked_at = COALESCE(revoked_at, ?2) WHERE id = ?1",
                params![id.0, Utc::now().to_rfc3339()],
            )
            .map_err(sql("revoke"))?;
        // This process sees its own revocation immediately; others within the TTL.
        self.cache.lock().clear();
        Ok(changed > 0)
    }

    /// All keys, newest first, including revoked ones (the console shows both).
    pub fn list(&self) -> Result<Vec<VirtualKey>> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare(&format!("{SELECT} ORDER BY created_at DESC"))
            .map_err(sql("list"))?;
        let rows = stmt.query_map([], row_to_key).map_err(sql("list"))?;
        rows.collect::<std::result::Result<_, _>>()
            .map_err(sql("list"))
    }

    fn get_by_hash(&self, hash: &str) -> Result<Option<VirtualKey>> {
        self.conn
            .lock()
            .query_row(
                &format!("{SELECT} WHERE token_hash = ?1"),
                [hash],
                row_to_key,
            )
            .optional()
            .map_err(sql("lookup"))
    }

    fn touch(&self, key: &mut VirtualKey) -> Result<()> {
        let now = Utc::now();
        if key
            .last_used_at
            .is_some_and(|t| now - t < LAST_USED_RESOLUTION)
        {
            return Ok(());
        }
        self.conn
            .lock()
            .execute(
                "UPDATE keys SET last_used_at = ?2 WHERE id = ?1",
                params![key.id.0, now.to_rfc3339()],
            )
            .map_err(sql("touch"))?;
        key.last_used_at = Some(now);
        Ok(())
    }
}

fn encode_scopes(scopes: &[KeyScope]) -> String {
    serde_json::to_string(scopes).unwrap_or_else(|_| "[]".to_owned())
}

fn parse_time(s: Option<String>) -> Option<DateTime<Utc>> {
    s.and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
        .map(|t| t.with_timezone(&Utc))
}

fn row_to_key(row: &rusqlite::Row<'_>) -> rusqlite::Result<VirtualKey> {
    let scopes: String = row.get(5)?;
    let created: String = row.get(6)?;
    Ok(VirtualKey {
        id: VirtualKeyId(row.get(0)?),
        name: row.get(1)?,
        tenant_id: row.get(2)?,
        token_hash: row.get(3)?,
        prefix: row.get(4)?,
        // Unknown scope names (from a newer build) drop the whole list to empty:
        // failing closed beats granting access the operator never saw.
        scopes: serde_json::from_str(&scopes).unwrap_or_default(),
        created_at: parse_time(Some(created)).unwrap_or_else(Utc::now),
        last_used_at: parse_time(row.get(7)?),
        revoked_at: parse_time(row.get(8)?),
    })
}

fn sql(op: &'static str) -> impl Fn(rusqlite::Error) -> KeyStoreError {
    move |source| KeyStoreError::Sqlite { op, source }
}

fn io_err(op: &'static str, path: &Path, source: std::io::Error) -> KeyStoreError {
    KeyStoreError::Io {
        op,
        path: path.display().to_string(),
        source,
    }
}

#[cfg(unix)]
fn create_private_file(path: &Path) -> Result<()> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| io_err("create", path, e))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| io_err("chmod", path, e))
}

#[cfg(not(unix))]
fn create_private_file(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scopes() -> Vec<KeyScope> {
        KeyScope::DEFAULT.to_vec()
    }

    // Plausible wrong impl: lookup compares the raw token, or accepts a prefix.
    #[test]
    fn only_the_exact_raw_token_authenticates() {
        let store = VirtualKeyStore::in_memory().unwrap();
        let (key, raw) = store.create("ci", "default", scopes()).unwrap();
        assert_eq!(store.authenticate(&raw).unwrap().unwrap().id, key.id);
        assert!(store.authenticate(&key.prefix).unwrap().is_none());
        assert!(store.authenticate(&key.token_hash).unwrap().is_none());
        assert!(store.authenticate("").unwrap().is_none());
    }

    // Plausible wrong impl: a cached positive lookup outlives revocation.
    #[test]
    fn revoked_key_stops_authenticating_in_the_same_process_at_once() {
        let store = VirtualKeyStore::in_memory().unwrap();
        let (key, raw) = store.create("ci", "default", scopes()).unwrap();
        assert!(store.authenticate(&raw).unwrap().is_some());
        assert!(store.revoke(&key.id).unwrap());
        assert!(store.authenticate(&raw).unwrap().is_none());
        assert!(!store.revoke(&VirtualKeyId("nope".into())).unwrap());
    }

    // Plausible wrong impl: keys live only in memory and vanish on restart, or a
    // revocation made by another process (CLI, second gateway) is never seen.
    #[test]
    fn keys_and_revocations_are_shared_through_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keys.db");
        let gateway = VirtualKeyStore::open(&path)
            .unwrap()
            .with_cache_ttl(Duration::ZERO);
        let cli = VirtualKeyStore::open(&path).unwrap();

        let (key, raw) = cli.create("ci", "default", scopes()).unwrap();
        assert!(
            gateway.authenticate(&raw).unwrap().is_some(),
            "created elsewhere"
        );

        cli.revoke(&key.id).unwrap();
        assert!(
            gateway.authenticate(&raw).unwrap().is_none(),
            "revoked elsewhere"
        );

        drop(gateway);
        let restarted = VirtualKeyStore::open(&path).unwrap();
        let listed = restarted.list().unwrap();
        assert_eq!(listed.len(), 1);
        assert!(listed[0].is_revoked(), "revocation survives restart");
    }

    // Plausible wrong impl: the database stores the raw token.
    #[test]
    fn raw_token_never_reaches_the_database_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keys.db");
        let store = VirtualKeyStore::open(&path).unwrap();
        let (_key, raw) = store.create("ci", "default", scopes()).unwrap();
        drop(store);
        let mut bytes = std::fs::read(&path).unwrap();
        for wal in ["keys.db-wal", "keys.db-shm"] {
            if let Ok(b) = std::fs::read(dir.path().join(wal)) {
                bytes.extend(b);
            }
        }
        let secret = &raw.as_bytes()[crate::key::TOKEN_PREFIX.len() + 8..];
        assert!(
            !bytes.windows(secret.len()).any(|w| w == secret),
            "raw token found on disk"
        );
    }

    #[cfg(unix)]
    #[test]
    fn database_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keys.db");
        VirtualKeyStore::open(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    // Plausible wrong impl: failed lookups are cached. On a public listener every
    // random token then adds a map entry that is never evicted: unbounded memory.
    #[test]
    fn bogus_tokens_do_not_grow_the_cache() {
        let store = VirtualKeyStore::in_memory().unwrap();
        for i in 0..1000 {
            assert!(store
                .authenticate(&format!("vkdg_bogus{i}"))
                .unwrap()
                .is_none());
        }
        assert_eq!(store.cache.lock().len(), 0);
    }
}
