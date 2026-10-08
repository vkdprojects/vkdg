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

use crate::key::{
    display_prefix, hash_token, mint_token, KeyPatch, KeyScope, NewKey, VirtualKey, VirtualKeyId,
};

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

/// A key's usage in one period.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KeyUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub requests: u64,
}

impl KeyUsage {
    pub fn tokens(&self) -> u64 {
        self.input_tokens + self.output_tokens
    }
}

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
                      last_used_at, revoked_at, expires_at, allowed_models, allowed_ips, \
                      monthly_token_limit, requests_per_minute, disabled_at, no_log FROM keys";

/// Columns added after the first release, with their definitions. `init` adds
/// any that an existing `keys.db` lacks, so upgrades need no manual step.
const LATER_COLUMNS: &[(&str, &str)] = &[
    ("expires_at", "TEXT"),
    ("allowed_models", "TEXT NOT NULL DEFAULT '[]'"),
    ("allowed_ips", "TEXT NOT NULL DEFAULT '[]'"),
    ("monthly_token_limit", "INTEGER"),
    ("requests_per_minute", "INTEGER"),
    ("disabled_at", "TEXT"),
    ("no_log", "INTEGER NOT NULL DEFAULT 0"),
];

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
             );
             CREATE TABLE IF NOT EXISTS key_usage (
                 key_id        TEXT NOT NULL,
                 period        TEXT NOT NULL,
                 input_tokens  INTEGER NOT NULL DEFAULT 0,
                 output_tokens INTEGER NOT NULL DEFAULT 0,
                 requests      INTEGER NOT NULL DEFAULT 0,
                 PRIMARY KEY (key_id, period)
             );",
        )
        .map_err(sql("init"))?;
        for (name, def) in LATER_COLUMNS {
            let present = conn
                .prepare("SELECT 1 FROM pragma_table_info('keys') WHERE name = ?1")
                .and_then(|mut st| st.exists([name]))
                .map_err(sql("migrate"))?;
            if !present {
                conn.execute(&format!("ALTER TABLE keys ADD COLUMN {name} {def}"), [])
                    .map_err(sql("migrate"))?;
            }
        }
        Ok(Self {
            conn: Mutex::new(conn),
            cache: Mutex::new(HashMap::new()),
            ttl,
        })
    }

    /// Mint and persist a key. Returns the record and the raw token (shown once).
    pub fn create(&self, spec: NewKey) -> Result<(VirtualKey, String)> {
        let (key, raw) = VirtualKey::new(spec);
        let ips: Vec<String> = key.allowed_ips.iter().map(ToString::to_string).collect();
        self.conn
            .lock()
            .execute(
                "INSERT INTO keys (id, name, tenant_id, token_hash, prefix, scopes, created_at,
                                   expires_at, allowed_models, allowed_ips,
                                   monthly_token_limit, requests_per_minute, no_log)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                params![
                    key.id.0,
                    key.name,
                    key.tenant_id,
                    key.token_hash,
                    key.prefix,
                    encode_scopes(&key.scopes),
                    key.created_at.to_rfc3339(),
                    key.expires_at.map(|t| t.to_rfc3339()),
                    json(&key.allowed_models),
                    json(&ips),
                    key.monthly_token_limit.map(to_i64),
                    key.requests_per_minute,
                    key.no_log,
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
                    // Expiry is a wall-clock fact, not a cache decision.
                    if hit.key.is_expired_at(Utc::now()) {
                        cache.remove(&hash);
                        return Ok(None);
                    }
                    return Ok(Some(hit.key.clone()));
                }
                // Expired: drop it so the map only ever holds live, valid keys.
                Some(_) => {
                    cache.remove(&hash);
                }
                None => {}
            }
        }

        let Some(mut key) = self
            .get_by_hash(&hash)?
            .filter(|k| !k.is_revoked() && !k.is_disabled() && !k.is_expired_at(Utc::now()))
        else {
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

    /// Add a response's tokens to the key's current-month usage. One statement,
    /// so concurrent requests and processes never lose an update.
    pub fn record_usage(&self, key_id: &VirtualKeyId, input: u64, output: u64) -> Result<()> {
        self.conn
            .lock()
            .execute(
                "INSERT INTO key_usage (key_id, period, input_tokens, output_tokens, requests)
                 VALUES (?1, ?2, ?3, ?4, 1)
                 ON CONFLICT (key_id, period) DO UPDATE SET
                     input_tokens  = input_tokens + excluded.input_tokens,
                     output_tokens = output_tokens + excluded.output_tokens,
                     requests      = requests + 1",
                params![key_id.0, current_period(), to_i64(input), to_i64(output)],
            )
            .map_err(sql("record usage"))?;
        Ok(())
    }

    /// Usage of `key_id` in the current calendar month (UTC).
    pub fn usage_this_month(&self, key_id: &VirtualKeyId) -> Result<KeyUsage> {
        self.conn
            .lock()
            .query_row(
                "SELECT input_tokens, output_tokens, requests FROM key_usage
                 WHERE key_id = ?1 AND period = ?2",
                params![key_id.0, current_period()],
                |r| {
                    let n = |i| {
                        r.get::<_, i64>(i)
                            .map(|v| u64::try_from(v.max(0)).unwrap_or(0))
                    };
                    Ok(KeyUsage {
                        input_tokens: n(0)?,
                        output_tokens: n(1)?,
                        requests: n(2)?,
                    })
                },
            )
            .optional()
            .map(Option::unwrap_or_default)
            .map_err(sql("read usage"))
    }

    /// Apply a policy change. Returns the updated key, or `None` if the id does
    /// not exist. Takes effect in this process at once, elsewhere within the TTL.
    pub fn update(&self, id: &VirtualKeyId, patch: KeyPatch) -> Result<Option<VirtualKey>> {
        let Some(mut key) = self.get_by_id(id)? else {
            return Ok(None);
        };
        if let Some(v) = patch.name {
            key.name = v;
        }
        if let Some(v) = patch.scopes {
            key.scopes = v;
        }
        if let Some(v) = patch.expires_at {
            key.expires_at = v;
        }
        if let Some(v) = patch.allowed_models {
            key.allowed_models = v;
        }
        if let Some(v) = patch.allowed_ips {
            key.allowed_ips = v;
        }
        if let Some(v) = patch.monthly_token_limit {
            key.monthly_token_limit = v;
        }
        if let Some(v) = patch.requests_per_minute {
            key.requests_per_minute = v;
        }
        if let Some(v) = patch.no_log {
            key.no_log = v;
        }
        let ips: Vec<String> = key.allowed_ips.iter().map(ToString::to_string).collect();
        self.conn
            .lock()
            .execute(
                "UPDATE keys SET name = ?2, scopes = ?3, expires_at = ?4, allowed_models = ?5,
                     allowed_ips = ?6, monthly_token_limit = ?7, requests_per_minute = ?8,
                     no_log = ?9
                 WHERE id = ?1",
                params![
                    key.id.0,
                    key.name,
                    encode_scopes(&key.scopes),
                    key.expires_at.map(|t| t.to_rfc3339()),
                    json(&key.allowed_models),
                    json(&ips),
                    key.monthly_token_limit.map(to_i64),
                    key.requests_per_minute,
                    key.no_log,
                ],
            )
            .map_err(sql("update"))?;
        // A cached hit would keep serving the old, possibly wider, policy.
        self.cache.lock().clear();
        Ok(Some(key))
    }

    /// Replace a key's secret, keeping its id, policy and usage. The old token
    /// stops working at once in this process. Returns the new raw token.
    pub fn regenerate(&self, id: &VirtualKeyId) -> Result<Option<(VirtualKey, String)>> {
        let raw = mint_token();
        let changed = self
            .conn
            .lock()
            .execute(
                "UPDATE keys SET token_hash = ?2, prefix = ?3 WHERE id = ?1 AND revoked_at IS NULL",
                params![id.0, hash_token(&raw), display_prefix(&raw)],
            )
            .map_err(sql("regenerate"))?;
        self.cache.lock().clear();
        if changed == 0 {
            return Ok(None);
        }
        Ok(self.get_by_id(id)?.map(|k| (k, raw)))
    }

    /// Disable or re-enable a key. A disabled key does not authenticate.
    pub fn set_disabled(&self, id: &VirtualKeyId, disabled: bool) -> Result<bool> {
        let changed = self
            .conn
            .lock()
            .execute(
                "UPDATE keys SET disabled_at = CASE WHEN ?2 THEN COALESCE(disabled_at, ?3) END
                 WHERE id = ?1",
                params![id.0, disabled, Utc::now().to_rfc3339()],
            )
            .map_err(sql("disable"))?;
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

    fn get_by_id(&self, id: &VirtualKeyId) -> Result<Option<VirtualKey>> {
        self.conn
            .lock()
            .query_row(&format!("{SELECT} WHERE id = ?1"), [&id.0], row_to_key)
            .optional()
            .map_err(sql("lookup"))
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

/// Usage bucket for "now": the calendar month in UTC, e.g. `2026-09`.
fn current_period() -> String {
    Utc::now().format("%Y-%m").to_string()
}

fn to_i64(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

fn encode_scopes(scopes: &[KeyScope]) -> String {
    json(scopes)
}

fn json<T: serde::Serialize + ?Sized>(v: &T) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "[]".to_owned())
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
        revoked_at: policy_time(row, 8)?,
        expires_at: policy_time(row, 9)?,
        allowed_models: policy_json(row, 10)?,
        allowed_ips: policy_json::<Vec<String>>(row, 11)?
            .iter()
            .map(|e| e.parse().map_err(|m: String| policy_err(11, m)))
            .collect::<rusqlite::Result<_>>()?,
        monthly_token_limit: row
            .get::<_, Option<i64>>(12)?
            .map(|v| u64::try_from(v.max(0)).unwrap_or(0)),
        requests_per_minute: row.get(13)?,
        disabled_at: policy_time(row, 14)?,
        no_log: row.get(15)?,
    })
}

// Policy columns restrict a key. A value that does not parse must never read
// as "no restriction" (empty list, no expiry): it is a row error instead, which
// the data plane turns into a rejected request.

fn policy_err(col: usize, msg: impl Into<String>) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        col,
        rusqlite::types::Type::Text,
        format!("corrupt key policy column: {}", msg.into()).into(),
    )
}

fn policy_time(row: &rusqlite::Row<'_>, col: usize) -> rusqlite::Result<Option<DateTime<Utc>>> {
    row.get::<_, Option<String>>(col)?
        .map(|s| {
            DateTime::parse_from_rfc3339(&s)
                .map(|t| t.with_timezone(&Utc))
                .map_err(|e| policy_err(col, e.to_string()))
        })
        .transpose()
}

fn policy_json<T: serde::de::DeserializeOwned>(
    row: &rusqlite::Row<'_>,
    col: usize,
) -> rusqlite::Result<T> {
    serde_json::from_str(&row.get::<_, String>(col)?).map_err(|e| policy_err(col, e.to_string()))
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

    // Plausible wrong impl: lookup compares the raw token, or accepts a prefix.
    #[test]
    fn only_the_exact_raw_token_authenticates() {
        let store = VirtualKeyStore::in_memory().unwrap();
        let (key, raw) = store.create(NewKey::named("ci")).unwrap();
        assert_eq!(store.authenticate(&raw).unwrap().unwrap().id, key.id);
        assert!(store.authenticate(&key.prefix).unwrap().is_none());
        assert!(store.authenticate(&key.token_hash).unwrap().is_none());
        assert!(store.authenticate("").unwrap().is_none());
    }

    // Plausible wrong impl: a cached positive lookup outlives revocation.
    #[test]
    fn revoked_key_stops_authenticating_in_the_same_process_at_once() {
        let store = VirtualKeyStore::in_memory().unwrap();
        let (key, raw) = store.create(NewKey::named("ci")).unwrap();
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

        let (key, raw) = cli.create(NewKey::named("ci")).unwrap();
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
        let (_key, raw) = store.create(NewKey::named("ci")).unwrap();
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

    // Plausible wrong impl: expiry is stored but never checked on the hot path,
    // or a cached hit outlives it.
    #[test]
    fn expired_key_stops_authenticating() {
        let store = VirtualKeyStore::in_memory()
            .unwrap()
            .with_cache_ttl(Duration::from_secs(60));
        let mut spec = NewKey::named("short");
        spec.expires_at = Some(Utc::now() + chrono::Duration::milliseconds(300));
        let (_key, raw) = store.create(spec).unwrap();
        assert!(store.authenticate(&raw).unwrap().is_some());
        std::thread::sleep(std::time::Duration::from_millis(400));
        assert!(
            store.authenticate(&raw).unwrap().is_none(),
            "expired, even while cached"
        );
    }

    #[test]
    fn limits_round_trip_through_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keys.db");
        let mut spec = NewKey::named("scoped");
        spec.allowed_models = vec!["claude-*".into()];
        spec.allowed_ips = vec![
            "10.0.0.0/8".parse().unwrap(),
            "2001:db8::/32".parse().unwrap(),
        ];
        spec.expires_at = Some(Utc::now() + chrono::Duration::days(1));
        VirtualKeyStore::open(&path)
            .unwrap()
            .create(spec.clone())
            .unwrap();
        let k = &VirtualKeyStore::open(&path).unwrap().list().unwrap()[0];
        assert_eq!(k.allowed_models, spec.allowed_models);
        assert_eq!(k.allowed_ips, spec.allowed_ips);
        assert!(k.expires_at.is_some());
    }

    // Upgrading a keys.db created before these columns existed.
    #[test]
    fn old_store_gains_the_new_columns_and_keeps_its_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keys.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE keys (id TEXT PRIMARY KEY, name TEXT NOT NULL, tenant_id TEXT NOT NULL,
                  token_hash TEXT NOT NULL UNIQUE, prefix TEXT NOT NULL, scopes TEXT NOT NULL,
                  created_at TEXT NOT NULL, last_used_at TEXT, revoked_at TEXT);
                 INSERT INTO keys VALUES ('k1','old','default','h','vkdg_x','[\"data_inference\"]',
                  '2026-01-01T00:00:00Z',NULL,NULL);",
            )
            .unwrap();
        }
        let keys = VirtualKeyStore::open(&path).unwrap().list().unwrap();
        assert_eq!(keys.len(), 1);
        assert!(keys[0].allowed_models.is_empty() && keys[0].allowed_ips.is_empty());
        VirtualKeyStore::open(&path).unwrap(); // idempotent
    }

    // A corrupt policy column must never widen a key: garbage in expires_at,
    // allowed_models, or allowed_ips makes the key unusable, not unrestricted.
    #[test]
    fn corrupt_policy_columns_fail_closed() {
        for (col, garbage) in [
            ("expires_at", "not-a-time"),
            // A revoked key must not come back because its timestamp is garbage.
            ("revoked_at", "not-a-time"),
            ("disabled_at", "not-a-time"),
            ("allowed_models", "{broken"),
            ("allowed_ips", "{broken"),
            ("allowed_ips", "[\"192.168.\"]"),
        ] {
            let store = VirtualKeyStore::in_memory().unwrap();
            let (key, raw) = store.create(NewKey::named("k")).unwrap();
            store
                .conn
                .lock()
                .execute(
                    &format!("UPDATE keys SET {col} = ?1 WHERE id = ?2"),
                    params![garbage, key.id.0],
                )
                .unwrap();
            assert!(
                !matches!(store.authenticate(&raw), Ok(Some(_))),
                "{col}={garbage:?} must not authenticate"
            );
        }
    }

    // Plausible wrong impl: usage kept in memory (lost on restart) or written
    // read-modify-write (concurrent updates lost).
    #[test]
    fn usage_accumulates_and_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keys.db");
        let a = VirtualKeyStore::open(&path).unwrap();
        let b = VirtualKeyStore::open(&path).unwrap();
        let (key, _) = a.create(NewKey::named("k")).unwrap();
        a.record_usage(&key.id, 10, 5).unwrap();
        b.record_usage(&key.id, 1, 2).unwrap();
        drop((a, b));
        let u = VirtualKeyStore::open(&path)
            .unwrap()
            .usage_this_month(&key.id)
            .unwrap();
        assert_eq!(
            u,
            KeyUsage {
                input_tokens: 11,
                output_tokens: 7,
                requests: 2
            }
        );
        assert_eq!(u.tokens(), 18);
    }

    #[test]
    fn limits_persist() {
        let store = VirtualKeyStore::in_memory().unwrap();
        let mut spec = NewKey::named("k");
        spec.monthly_token_limit = Some(1_000);
        spec.requests_per_minute = Some(60);
        store.create(spec).unwrap();
        let k = &store.list().unwrap()[0];
        assert_eq!(
            (k.monthly_token_limit, k.requests_per_minute),
            (Some(1_000), Some(60))
        );
    }

    #[test]
    fn update_changes_policy_and_applies_to_the_next_request() {
        let store = VirtualKeyStore::in_memory()
            .unwrap()
            .with_cache_ttl(Duration::from_secs(60));
        let (key, raw) = store.create(NewKey::named("k")).unwrap();
        assert_eq!(
            store
                .authenticate(&raw)
                .unwrap()
                .unwrap()
                .allowed_models
                .len(),
            0
        );
        let updated = store
            .update(
                &key.id,
                KeyPatch {
                    name: Some("renamed".into()),
                    allowed_models: Some(vec!["claude-*".into()]),
                    monthly_token_limit: Some(Some(10)),
                    ..KeyPatch::default()
                },
            )
            .unwrap()
            .expect("key exists");
        assert_eq!(updated.name, "renamed");
        // Cached hit must not keep the old, wider policy.
        let live = store.authenticate(&raw).unwrap().unwrap();
        assert_eq!(live.allowed_models, ["claude-*"]);
        assert_eq!(live.monthly_token_limit, Some(10));
        let cleared = store
            .update(
                &key.id,
                KeyPatch {
                    monthly_token_limit: Some(None),
                    ..KeyPatch::default()
                },
            )
            .unwrap()
            .unwrap();
        assert_eq!(cleared.monthly_token_limit, None);
        assert!(store
            .update(&VirtualKeyId("nope".into()), KeyPatch::default())
            .unwrap()
            .is_none());
    }

    #[test]
    fn regenerate_keeps_identity_and_kills_the_old_secret() {
        let store = VirtualKeyStore::in_memory()
            .unwrap()
            .with_cache_ttl(Duration::from_secs(60));
        let (key, old) = store.create(NewKey::named("k")).unwrap();
        store.record_usage(&key.id, 5, 5).unwrap();
        assert!(store.authenticate(&old).unwrap().is_some());
        let (again, new) = store.regenerate(&key.id).unwrap().expect("key exists");
        assert_eq!(again.id, key.id);
        assert_ne!(new, old);
        assert!(
            store.authenticate(&old).unwrap().is_none(),
            "old secret dies at once"
        );
        assert!(store.authenticate(&new).unwrap().is_some());
        assert_eq!(
            store.usage_this_month(&key.id).unwrap().tokens(),
            10,
            "usage kept"
        );
    }

    #[test]
    fn disable_is_reversible_unlike_revoke() {
        let store = VirtualKeyStore::in_memory()
            .unwrap()
            .with_cache_ttl(Duration::from_secs(60));
        let (key, raw) = store.create(NewKey::named("k")).unwrap();
        assert!(store.authenticate(&raw).unwrap().is_some());
        assert!(store.set_disabled(&key.id, true).unwrap());
        assert!(store.authenticate(&raw).unwrap().is_none());
        assert!(store.set_disabled(&key.id, false).unwrap());
        assert!(store.authenticate(&raw).unwrap().is_some());
        store.revoke(&key.id).unwrap();
        store.set_disabled(&key.id, false).unwrap();
        assert!(
            store.authenticate(&raw).unwrap().is_none(),
            "enable never un-revokes"
        );
    }
}
