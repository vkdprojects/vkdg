//! Persisted provider accounts (OAuth logins) and the per-request credential.
//!
//! An [`Account`] is created by an interactive login (device code, PKCE) or a
//! token import, and referenced from config with `auth: { type: account, account: <id> }`.
//! [`AccountStore`] persists accounts in a SQLite file created with `0600` permissions.
//! Tokens and `extra` values are never printed by `Debug`.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use vkdg_core::{Result, VkdgError};

// ── Token pair ────────────────────────────────────────────────────────────────

/// Successful token response from a login, exchange, or refresh.
///
/// Produced by provider plugins; persisted by the gateway into an [`Account`].
pub struct TokenPair {
    pub access_token: String,
    /// `None` on refresh = keep the previous refresh token.
    pub refresh_token: Option<String>,
    /// Seconds until `access_token` expires. `None` = unknown (never refreshed ahead).
    pub expires_in_secs: Option<u64>,
    /// Provider-specific data that MUST be persisted and passed back on the next
    /// refresh and to `prepare()`. Examples: client registration, device id, region.
    /// On refresh, keys returned here overwrite stored keys; other keys are kept.
    pub extra: HashMap<String, String>,
}

impl std::fmt::Debug for TokenPair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenPair")
            .field("expires_in_secs", &self.expires_in_secs)
            .field("extra_keys", &sorted_keys(&self.extra))
            .finish_non_exhaustive()
    }
}

// ── Credential (what prepare() receives) ──────────────────────────────────────

/// Resolved credential for one upstream request.
///
/// `extra` carries per-account plugin data (empty for API keys).
#[derive(Clone)]
pub struct Credential {
    pub token: String,
    pub extra: Arc<HashMap<String, String>>,
}

impl Credential {
    /// Credential with a bare token and no extra data (API keys, tests).
    pub fn bearer(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
            extra: Arc::default(),
        }
    }
}

impl std::fmt::Debug for Credential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credential")
            .field("extra_keys", &sorted_keys(&self.extra))
            .finish_non_exhaustive()
    }
}

// ── Account ───────────────────────────────────────────────────────────────────

/// One logged-in provider account.
#[derive(Clone)]
pub struct Account {
    pub id: String,
    /// Provider plugin id (`ProviderAdapter::id`) that owns login and refresh.
    pub provider: String,
    /// Human-readable identity (email, profile name, or operator-given label).
    pub label: String,
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub extra: HashMap<String, String>,
}

impl Account {
    /// Build a new account from a login result. The id is `<provider>-<8 hex>`.
    pub fn from_token_pair(provider: &str, label: &str, pair: TokenPair) -> Self {
        let suffix = uuid::Uuid::new_v4().simple().to_string();
        Self {
            id: format!("{provider}-{}", &suffix[..8]),
            provider: provider.to_owned(),
            label: label.to_owned(),
            access_token: pair.access_token,
            refresh_token: pair.refresh_token,
            expires_at: expires_at_from(pair.expires_in_secs),
            extra: pair.extra,
        }
    }

    /// Apply a refresh result: new tokens, merged extra, kept refresh token if absent.
    pub fn apply_refresh(&mut self, pair: TokenPair) {
        self.access_token = pair.access_token;
        if let Some(rt) = pair.refresh_token {
            self.refresh_token = Some(rt);
        }
        self.expires_at = expires_at_from(pair.expires_in_secs);
        self.extra.extend(pair.extra);
    }

    /// True when the access token expires within `margin` (unknown expiry = never).
    pub fn expires_within(&self, margin: chrono::Duration) -> bool {
        self.expires_at
            .is_some_and(|exp| Utc::now() + margin >= exp)
    }

    pub fn credential(&self) -> Credential {
        Credential {
            token: self.access_token.clone(),
            extra: Arc::new(self.extra.clone()),
        }
    }
}

impl std::fmt::Debug for Account {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Account")
            .field("id", &self.id)
            .field("provider", &self.provider)
            .field("label", &self.label)
            .field("expires_at", &self.expires_at)
            .field("has_refresh_token", &self.refresh_token.is_some())
            .field("extra_keys", &sorted_keys(&self.extra))
            .finish_non_exhaustive()
    }
}

fn expires_at_from(secs: Option<u64>) -> Option<DateTime<Utc>> {
    secs.map(|s| {
        Utc::now() + chrono::Duration::seconds(i64::try_from(s).unwrap_or(i64::MAX / 1000))
    })
}

fn sorted_keys(map: &HashMap<String, String>) -> Vec<&str> {
    let mut keys: Vec<&str> = map.keys().map(String::as_str).collect();
    keys.sort_unstable();
    keys
}

// ── Store ─────────────────────────────────────────────────────────────────────

/// SQLite-backed account store. Single writer behind a mutex; operations are
/// small and run on the cold path (login, first use, refresh), never per token.
pub struct AccountStore {
    conn: Mutex<Connection>,
}

impl AccountStore {
    /// Open (or create) the store at `path`. On Unix the parent directory is
    /// created `0700` and the file is forced to `0600` before any token is written.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            create_private_dir(parent)?;
        }
        create_private_file(path)?;
        let conn = Connection::open(path).map_err(|e| store_err("open", &e))?;
        Self::init(conn)
    }

    /// `$VKDG_ACCOUNTS_DB`, else `$HOME/.config/vkdg/accounts.db`, else `./accounts.db`.
    pub fn default_path() -> std::path::PathBuf {
        if let Ok(p) = std::env::var("VKDG_ACCOUNTS_DB") {
            return p.into();
        }
        std::env::var_os("HOME").map_or_else(
            || "accounts.db".into(),
            |h| Path::new(&h).join(".config/vkdg/accounts.db"),
        )
    }

    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().map_err(|e| store_err("open", &e))?;
        Self::init(conn)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS accounts (
                 id            TEXT PRIMARY KEY,
                 provider      TEXT NOT NULL,
                 label         TEXT NOT NULL,
                 access_token  TEXT NOT NULL,
                 refresh_token TEXT,
                 expires_at    TEXT,
                 extra         TEXT NOT NULL,
                 updated_at    TEXT NOT NULL
             );",
        )
        .map_err(|e| store_err("init", &e))?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Insert or replace an account.
    pub fn upsert(&self, account: &Account) -> Result<()> {
        let extra = serde_json::to_string(&account.extra)
            .map_err(|e| VkdgError::Internal(format!("account store: encode extra: {e}")))?;
        self.lock()
            .execute(
                "INSERT OR REPLACE INTO accounts
                     (id, provider, label, access_token, refresh_token, expires_at, extra, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    account.id,
                    account.provider,
                    account.label,
                    account.access_token,
                    account.refresh_token,
                    account.expires_at.map(|t| t.to_rfc3339()),
                    extra,
                    Utc::now().to_rfc3339(),
                ],
            )
            .map_err(|e| store_err("upsert", &e))?;
        Ok(())
    }

    pub fn get(&self, id: &str) -> Result<Option<Account>> {
        self.lock()
            .query_row(
                &format!("{SELECT_ACCOUNT} WHERE id = ?1"),
                params![id],
                row_to_account,
            )
            .optional()
            .map_err(|e| store_err("get", &e))
    }

    /// All accounts ordered by id.
    pub fn list(&self) -> Result<Vec<Account>> {
        let conn = self.lock();
        let mut stmt = conn
            .prepare(&format!("{SELECT_ACCOUNT} ORDER BY id"))
            .map_err(|e| store_err("list", &e))?;
        let rows = stmt
            .query_map([], row_to_account)
            .map_err(|e| store_err("list", &e))?;
        rows.collect::<std::result::Result<_, _>>()
            .map_err(|e| store_err("list", &e))
    }

    /// Delete an account. Returns `false` when it did not exist.
    pub fn remove(&self, id: &str) -> Result<bool> {
        let n = self
            .lock()
            .execute("DELETE FROM accounts WHERE id = ?1", params![id])
            .map_err(|e| store_err("remove", &e))?;
        Ok(n > 0)
    }

    fn lock(&self) -> parking_lot::MutexGuard<'_, Connection> {
        self.conn.lock()
    }
}

const SELECT_ACCOUNT: &str =
    "SELECT id, provider, label, access_token, refresh_token, expires_at, extra FROM accounts";

fn row_to_account(row: &rusqlite::Row<'_>) -> rusqlite::Result<Account> {
    let expires_at: Option<String> = row.get(5)?;
    let extra: String = row.get(6)?;
    Ok(Account {
        id: row.get(0)?,
        provider: row.get(1)?,
        label: row.get(2)?,
        access_token: row.get(3)?,
        refresh_token: row.get(4)?,
        expires_at: expires_at
            .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
            .map(|t| t.with_timezone(&Utc)),
        extra: serde_json::from_str(&extra).unwrap_or_default(),
    })
}

fn store_err(op: &str, e: &rusqlite::Error) -> VkdgError {
    VkdgError::Internal(format!("account store {op}: {e}"))
}

fn io_err(op: &str, path: &Path, e: &std::io::Error) -> VkdgError {
    VkdgError::Internal(format!("account store {op} {}: {e}", path.display()))
}

#[cfg(unix)]
fn create_private_dir(dir: &Path) -> Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    if dir.exists() {
        return Ok(());
    }
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(|e| io_err("create dir", dir, &e))
}

#[cfg(not(unix))]
fn create_private_dir(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir).map_err(|e| io_err("create dir", dir, &e))
}

#[cfg(unix)]
fn create_private_file(path: &Path) -> Result<()> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| io_err("create", path, &e))?;
    // Tighten a pre-existing file too.
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| io_err("chmod", path, &e))
}

#[cfg(not(unix))]
fn create_private_file(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Account {
        let mut extra = HashMap::new();
        extra.insert("client_secret".into(), "extra-secret-value".into());
        Account {
            id: "kiro-1".into(),
            provider: "kiro".into(),
            label: "dev@example.com".into(),
            access_token: "access-secret-value".into(),
            refresh_token: Some("refresh-secret-value".into()),
            expires_at: Some(Utc::now() + chrono::Duration::hours(1)),
            extra,
        }
    }

    // Plausible wrong impl: derived Debug leaks tokens or extra secrets into logs.
    #[test]
    fn debug_redacts_tokens_and_extra_values() {
        let acct = sample();
        let cred = acct.credential();
        for text in [format!("{acct:?}"), format!("{cred:?}")] {
            assert!(!text.contains("access-secret-value"), "{text}");
            assert!(!text.contains("refresh-secret-value"), "{text}");
            assert!(!text.contains("extra-secret-value"), "{text}");
            assert!(text.contains("client_secret"), "keys stay visible: {text}");
        }
    }

    // Plausible wrong impl: file created with default umask (0644) → world-readable tokens;
    // or persisted fields lost/garbled on reopen.
    #[cfg(unix)]
    #[test]
    fn file_is_private_and_round_trips_across_reopen() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/accounts.db");
        {
            let store = AccountStore::open(&path).unwrap();
            store.upsert(&sample()).unwrap();
        }
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let dir_mode = std::fs::metadata(path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(dir_mode, 0o700);

        let store = AccountStore::open(&path).unwrap();
        let got = store.get("kiro-1").unwrap().expect("persisted");
        assert_eq!(got.access_token, "access-secret-value");
        assert_eq!(got.refresh_token.as_deref(), Some("refresh-secret-value"));
        assert_eq!(got.extra["client_secret"], "extra-secret-value");
        assert!(got.expires_at.is_some());
        assert!(store.remove("kiro-1").unwrap());
        assert!(!store.remove("kiro-1").unwrap());
        assert!(store.list().unwrap().is_empty());
    }

    // Plausible wrong impl: refresh drops the stored refresh token or wipes extra keys
    // the provider did not echo back (e.g. client registration).
    #[test]
    fn apply_refresh_keeps_refresh_token_and_merges_extra() {
        let mut acct = sample();
        let mut extra = HashMap::new();
        extra.insert("profile_arn".into(), "arn:new".into());
        acct.apply_refresh(TokenPair {
            access_token: "new-access".into(),
            refresh_token: None,
            expires_in_secs: Some(3600),
            extra,
        });
        assert_eq!(acct.access_token, "new-access");
        assert_eq!(acct.refresh_token.as_deref(), Some("refresh-secret-value"));
        assert_eq!(acct.extra["client_secret"], "extra-secret-value");
        assert_eq!(acct.extra["profile_arn"], "arn:new");
    }
}
