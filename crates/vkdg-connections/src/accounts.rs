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

/// Where a deployment mounts its persistent volume. Used when the process has no
/// usable home directory, which is the normal case inside a container image.
const SERVICE_DATA_DIR: &str = "/var/lib/vkdg";

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
    /// Why the upstream rejected this account's refresh token, if it did.
    ///
    /// Set when a refresh fails with a revocation (not a transient error). The
    /// gateway then stops calling the refresh endpoint on every request and
    /// reports the account as needing a new login.
    pub revoked: Option<String>,
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
            revoked: None,
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

    /// True when the access token expires within `margin`, or when its expiry is
    /// unknown.
    ///
    /// Unknown expiry means "refresh now", not "never expires". Treating it as
    /// never disabled refresh entirely for any provider that omits `expires_in`:
    /// Kiro's social login does, so its accounts died silently about an hour after
    /// login while a perfectly good refresh token sat unused.
    pub fn expires_within(&self, margin: chrono::Duration) -> bool {
        // `is_none_or` postdates the project MSRV.
        self.expires_at
            .map_or(true, |exp| Utc::now() + margin >= exp)
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

    /// `$VKDG_ACCOUNTS_DB`, else `$HOME/.config/vkdg/accounts.db`, else the
    /// service data dir.
    ///
    /// A container image has no real home: `HOME` is unset or `/`, which put the
    /// database at `/.config/vkdg/accounts.db` on the container's own filesystem,
    /// so every account vanished on the next `docker compose up` and users had to
    /// log in again after each deploy. With no usable home, fall back to
    /// `/var/lib/vkdg`, which is where deployments mount their volume.
    pub fn default_path() -> std::path::PathBuf {
        if let Ok(p) = std::env::var("VKDG_ACCOUNTS_DB") {
            return p.into();
        }
        match std::env::var_os("HOME") {
            // `/` is what a scratch container reports; it is not a home directory.
            Some(h) if h != "/" && !h.is_empty() => Path::new(&h).join(".config/vkdg/accounts.db"),
            _ => Path::new(SERVICE_DATA_DIR).join("accounts.db"),
        }
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
                 updated_at    TEXT NOT NULL,
                 revoked       TEXT
             );",
        )
        .map_err(|e| store_err("init", &e))?;
        // Stores created before `revoked` existed lack the column. `CREATE TABLE
        // IF NOT EXISTS` does not add it, so add it here; the check keeps this
        // idempotent across restarts.
        let has_revoked = conn
            .prepare("SELECT 1 FROM pragma_table_info('accounts') WHERE name = 'revoked'")
            .and_then(|mut st| st.exists([]))
            .map_err(|e| store_err("migrate", &e))?;
        if !has_revoked {
            conn.execute("ALTER TABLE accounts ADD COLUMN revoked TEXT", [])
                .map_err(|e| store_err("migrate", &e))?;
        }
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
                     (id, provider, label, access_token, refresh_token, expires_at, extra, updated_at, revoked)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    account.id,
                    account.provider,
                    account.label,
                    account.access_token,
                    account.refresh_token,
                    account.expires_at.map(|t| t.to_rfc3339()),
                    extra,
                    Utc::now().to_rfc3339(),
                    account.revoked,
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
    "SELECT id, provider, label, access_token, refresh_token, expires_at, extra, revoked FROM accounts";

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
        revoked: row.get(7)?,
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

    /// Refutes: resolving the store to the container's own filesystem.
    ///
    /// A scratch image reports `HOME=/`, which produced `/.config/vkdg/accounts.db`
    /// — outside any mounted volume — so every account was lost on the next
    /// `docker compose up` and users had to log in again after each deploy.
    ///
    /// The env var is process-global, so this test does not run in parallel with
    /// others that read it; it restores what it found.
    #[test]
    fn default_path_avoids_the_container_filesystem() {
        let saved_db = std::env::var_os("VKDG_ACCOUNTS_DB");
        let saved_home = std::env::var_os("HOME");
        std::env::remove_var("VKDG_ACCOUNTS_DB");

        // An explicit override always wins.
        std::env::set_var("VKDG_ACCOUNTS_DB", "/custom/accounts.db");
        assert_eq!(
            AccountStore::default_path(),
            std::path::PathBuf::from("/custom/accounts.db")
        );
        std::env::remove_var("VKDG_ACCOUNTS_DB");

        // A real home keeps the per-user location.
        std::env::set_var("HOME", "/home/dev");
        assert_eq!(
            AccountStore::default_path(),
            std::path::PathBuf::from("/home/dev/.config/vkdg/accounts.db")
        );

        // `HOME=/` and an unset HOME both mean "no home": use the volume.
        let expected = std::path::Path::new(SERVICE_DATA_DIR).join("accounts.db");
        std::env::set_var("HOME", "/");
        assert_eq!(
            AccountStore::default_path(),
            expected,
            "HOME=/ is not a home"
        );
        std::env::set_var("HOME", "");
        assert_eq!(AccountStore::default_path(), expected, "empty HOME");
        std::env::remove_var("HOME");
        assert_eq!(AccountStore::default_path(), expected, "unset HOME");

        match saved_home {
            Some(h) => std::env::set_var("HOME", h),
            None => std::env::remove_var("HOME"),
        }
        if let Some(db) = saved_db {
            std::env::set_var("VKDG_ACCOUNTS_DB", db);
        }
    }

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
            revoked: None,
        }
    }

    /// Refutes: reading an unknown expiry as "never expires".
    ///
    /// Kiro's social login omits `expires_in`, so `expires_at` was null and
    /// `expires_within` answered false for every margin. Refresh sits behind that
    /// check, so it never ran: accounts died about an hour after login with a
    /// perfectly good refresh token unused on disk. Found against a live account.
    #[test]
    fn unknown_expiry_means_refresh_now_not_never() {
        let mut acct = sample();
        acct.expires_at = None;
        assert!(
            acct.expires_within(chrono::Duration::zero()),
            "an account with no known expiry must be treated as due for refresh"
        );
        assert!(acct.expires_within(chrono::Duration::minutes(5)));

        // A known expiry still behaves normally on both sides of the margin.
        acct.expires_at = Some(Utc::now() + chrono::Duration::hours(1));
        assert!(!acct.expires_within(chrono::Duration::minutes(5)));
        acct.expires_at = Some(Utc::now() + chrono::Duration::minutes(2));
        assert!(acct.expires_within(chrono::Duration::minutes(5)));
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
