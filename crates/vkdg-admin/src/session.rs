//! Admin console sign-in: bootstrap token, persistent password, sessions.
//!
//! First run: the operator signs in with the bootstrap token and sets a
//! password (`POST /admin/v1/setup`). From then on only the password works;
//! the token is refused for as long as the password file exists, across
//! restarts. The password is stored as an argon2id PHC hash in a `0600` file
//! next to `accounts.db`. Recovery is `vkdg admin set-password` on the host.
//!
//! Sessions live in memory and expire after [`SESSION_IDLE`] without use; a
//! restart signs everyone out. Failed sign-ins are throttled per client
//! address (resolved like the data plane, trusted proxies included).

use parking_lot::{Mutex, RwLock};
use serde::Serialize;
use std::collections::HashMap;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;
use vkdg_core::net::IpNet;

/// A session unused for this long is gone.
pub const SESSION_IDLE: Duration = Duration::from_secs(12 * 3600);
/// Failed sign-ins allowed per address per [`FAILURE_WINDOW`].
pub const MAX_FAILURES: u32 = 5;
pub const FAILURE_WINDOW: Duration = Duration::from_secs(15 * 60);
/// Addresses tracked for throttling. When full, new addresses are not added
/// (evicting old ones would let an attacker rotating addresses reset their own
/// count); the global limit still covers them.
const MAX_TRACKED: usize = 10_000;
/// Failed sign-ins allowed across all addresses per [`FAILURE_WINDOW`]. Under a
/// distributed guess this locks the console for everyone until the window
/// ends; `vkdg admin set-password` on the host still works.
pub const MAX_GLOBAL_FAILURES: u32 = 100;
pub const MIN_PASSWORD_LEN: usize = 12;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Viewer,
    Operator,
    Admin,
}

#[derive(Debug, Clone)]
pub struct Session {
    pub session_id: String,
    pub user_id: String,
    pub role: Role,
}

struct Live {
    session: Session,
    last_seen: Instant,
}

pub struct SessionStore {
    sessions: RwLock<HashMap<String, Live>>,
    bootstrap_token: String,
    bootstrap_used: RwLock<bool>,
    /// Where the password hash lives. `None` (tests) = token-only sign-in.
    password_file: Option<PathBuf>,
    trusted_proxies: Vec<IpNet>,
    /// Failed sign-ins per client address: window start and count.
    failures: Mutex<HashMap<Option<IpAddr>, (Instant, u32)>>,
    global: Mutex<(Instant, u32)>,
    idle: Duration,
}

impl SessionStore {
    /// Token-only store, no password file (tests and embedded use).
    pub fn new(bootstrap_token: String) -> Arc<Self> {
        Arc::new(Self::build(bootstrap_token, None, Vec::new()))
    }

    /// Store backed by a password file; `trusted_proxies` resolve the client
    /// address behind a reverse proxy for sign-in throttling.
    pub fn with_password_file(
        bootstrap_token: String,
        password_file: PathBuf,
        trusted_proxies: Vec<IpNet>,
    ) -> Arc<Self> {
        Arc::new(Self::build(
            bootstrap_token,
            Some(password_file),
            trusted_proxies,
        ))
    }

    fn build(token: String, password_file: Option<PathBuf>, trusted_proxies: Vec<IpNet>) -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
            bootstrap_token: token,
            bootstrap_used: RwLock::new(false),
            password_file,
            trusted_proxies,
            failures: Mutex::new(HashMap::new()),
            global: Mutex::new((Instant::now(), 0)),
            idle: SESSION_IDLE,
        }
    }

    // ── Password ──────────────────────────────────────────────────────────────

    /// Whether an admin password exists. Read on every call, so a password set
    /// by another process (the CLI) takes effect at once. Only a missing file
    /// means "no password": an unreadable one fails closed, or a permission
    /// error would reopen bootstrap-token sign-in.
    pub fn password_set(&self) -> bool {
        match &self.password_file {
            None => false,
            Some(path) => !matches!(
                std::fs::metadata(path),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound
            ),
        }
    }

    /// Hash and store a new password (argon2id, `0600`, atomic replace).
    pub fn set_password(&self, password: &str) -> Result<(), String> {
        use argon2::password_hash::{rand_core::OsRng, PasswordHasher, SaltString};
        let path = self
            .password_file
            .as_ref()
            .ok_or("no password file is configured")?;
        if password.chars().count() < MIN_PASSWORD_LEN {
            return Err(format!(
                "the password must be at least {MIN_PASSWORD_LEN} characters"
            ));
        }
        let salt = SaltString::generate(&mut OsRng);
        let hash = argon2::Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map_err(|e| format!("hashing failed: {e}"))?
            .to_string();
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        write_private(path, hash.as_bytes()).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Check a password against the stored hash. Slow on purpose (argon2):
    /// call it off the async workers.
    pub fn verify_password(&self, password: &str) -> bool {
        use argon2::password_hash::{PasswordHash, PasswordVerifier};
        let Some(path) = &self.password_file else {
            return false;
        };
        let Ok(stored) = std::fs::read_to_string(path) else {
            return false;
        };
        let Ok(hash) = PasswordHash::new(stored.trim()) else {
            return false;
        };
        argon2::Argon2::default()
            .verify_password(password.as_bytes(), &hash)
            .is_ok()
    }

    // ── Bootstrap token ───────────────────────────────────────────────────────

    /// Constant-time token comparison.
    pub fn token_matches(&self, token: &str) -> bool {
        let (a, b) = (token.as_bytes(), self.bootstrap_token.as_bytes());
        a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
    }

    /// Exchange the bootstrap token for a session. One use per process, and
    /// never once a password is set.
    pub fn bootstrap_login(&self) -> Result<Session, &'static str> {
        if self.password_set() {
            return Err("an admin password is set; sign in with the password");
        }
        let mut used = self.bootstrap_used.write();
        if *used {
            return Err("bootstrap token already used");
        }
        *used = true;
        Ok(self.issue())
    }

    pub fn bootstrap_token(&self) -> &str {
        &self.bootstrap_token
    }

    // ── Sessions ──────────────────────────────────────────────────────────────

    /// A new admin session.
    pub fn issue(&self) -> Session {
        let session = Session {
            session_id: Uuid::new_v4().to_string(),
            user_id: "admin".to_string(),
            role: Role::Admin,
        };
        self.sessions.write().insert(
            session.session_id.clone(),
            Live {
                session: session.clone(),
                last_seen: Instant::now(),
            },
        );
        session
    }

    /// Validate a session cookie value, sliding its idle deadline.
    pub fn get(&self, session_id: &str) -> Option<Session> {
        let now = Instant::now();
        let mut sessions = self.sessions.write();
        let live = sessions.get_mut(session_id)?;
        if now.duration_since(live.last_seen) >= self.idle {
            sessions.remove(session_id);
            return None;
        }
        live.last_seen = now;
        Some(live.session.clone())
    }

    pub fn revoke(&self, session_id: &str) {
        self.sessions.write().remove(session_id);
    }

    // ── Throttling ────────────────────────────────────────────────────────────

    /// The client address as the gateway trusts it.
    pub fn client_ip<'a>(
        &self,
        peer: Option<IpAddr>,
        forwarded_for: impl IntoIterator<Item = &'a str>,
    ) -> Option<IpAddr> {
        vkdg_core::net::resolve_client_ip(peer, forwarded_for, &self.trusted_proxies)
    }

    /// `Some(wait)` when `ip` has used up its failed attempts.
    pub fn throttled(&self, ip: Option<IpAddr>) -> Option<Duration> {
        let now = Instant::now();
        let over = |(start, count): (Instant, u32), limit: u32| {
            let elapsed = now.duration_since(start);
            (elapsed < FAILURE_WINDOW && count >= limit).then(|| FAILURE_WINDOW - elapsed)
        };
        if let Some(wait) = over(*self.global.lock(), MAX_GLOBAL_FAILURES) {
            return Some(wait);
        }
        self.failures
            .lock()
            .get(&ip)
            .and_then(|w| over(*w, MAX_FAILURES))
    }

    pub fn record_failure(&self, ip: Option<IpAddr>) {
        let now = Instant::now();
        let bump = |w: &mut (Instant, u32)| {
            if now.duration_since(w.0) >= FAILURE_WINDOW {
                *w = (now, 0);
            }
            w.1 = w.1.saturating_add(1);
        };
        bump(&mut self.global.lock());
        let mut failures = self.failures.lock();
        if let Some(w) = failures.get_mut(&ip) {
            bump(w);
            return;
        }
        failures.retain(|_, w| now.duration_since(w.0) < FAILURE_WINDOW);
        // Full: do not add the address. Evicting others would let an attacker
        // rotating addresses reset their own count; the global limit covers it.
        if failures.len() < MAX_TRACKED {
            failures.insert(ip, (now, 1));
        }
    }

    pub fn clear_failures(&self, ip: Option<IpAddr>) {
        self.failures.lock().remove(&ip);
    }

    #[cfg(test)]
    fn with_idle(mut self: Arc<Self>, idle: Duration) -> Arc<Self> {
        Arc::get_mut(&mut self).expect("sole owner").idle = idle;
        self
    }

    #[cfg(test)]
    fn tracked(&self) -> usize {
        self.failures.lock().len()
    }
}

#[cfg(unix)]
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let tmp = path.with_extension("tmp");
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    std::fs::rename(tmp, path)
}

#[cfg(not(unix))]
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (Arc<SessionStore>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let s = SessionStore::with_password_file(
            "boot-token".into(),
            dir.path().join("admin.password"),
            vec![],
        );
        (s, dir)
    }

    // The bootstrap token only worked once: after a sign-out, the only way back
    // in was restarting the gateway.
    #[test]
    fn password_survives_a_new_store_and_retires_the_token() {
        let (s, dir) = store();
        assert!(!s.password_set());
        s.set_password("correct horse battery").unwrap();
        assert!(s.verify_password("correct horse battery"));
        assert!(!s.verify_password("wrong password!!"));

        // A restart: a new store on the same file still knows the password
        // and refuses the token.
        let again = SessionStore::with_password_file(
            "boot-token".into(),
            dir.path().join("admin.password"),
            vec![],
        );
        assert!(again.password_set());
        assert!(again.verify_password("correct horse battery"));
        assert!(
            again.bootstrap_login().is_err(),
            "token refused once a password exists"
        );
    }

    #[test]
    fn short_passwords_are_refused_and_the_file_is_private() {
        let (s, dir) = store();
        assert!(s.set_password("short").is_err());
        assert!(!s.password_set());
        s.set_password("long enough password").unwrap();
        let raw = std::fs::read_to_string(dir.path().join("admin.password")).unwrap();
        assert!(raw.starts_with("$argon2id$"), "{raw}");
        assert!(!raw.contains("long enough password"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join("admin.password"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    // An unreadable password file must not reopen token sign-in.
    #[cfg(unix)]
    #[test]
    fn unreadable_password_file_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        // A directory where the file should be: reading it fails with an
        // error other than NotFound.
        std::fs::create_dir(dir.path().join("admin.password")).unwrap();
        let s = SessionStore::with_password_file(
            "boot-token".into(),
            dir.path().join("admin.password"),
            vec![],
        );
        assert!(s.password_set());
        assert!(s.bootstrap_login().is_err());
        assert!(!s.verify_password("anything at all"));
    }

    #[test]
    fn failures_are_throttled_per_address() {
        let (s, _d) = store();
        let a: Option<IpAddr> = Some("203.0.113.1".parse().unwrap());
        let b: Option<IpAddr> = Some("203.0.113.2".parse().unwrap());
        for _ in 0..MAX_FAILURES {
            assert!(s.throttled(a).is_none());
            s.record_failure(a);
        }
        let wait = s.throttled(a).expect("sixth attempt is throttled");
        assert!(wait <= FAILURE_WINDOW && wait > Duration::ZERO);
        assert!(s.throttled(b).is_none(), "another address is unaffected");
        s.clear_failures(a);
        assert!(s.throttled(a).is_none());
    }

    // A flood from many addresses must not grow memory without bound.
    #[test]
    fn throttle_map_is_bounded() {
        let (s, _d) = store();
        for i in 0..(MAX_TRACKED as u32 + 500) {
            s.record_failure(Some(IpAddr::from(i.to_be_bytes())));
        }
        assert!(s.tracked() <= MAX_TRACKED);
    }

    #[test]
    fn idle_sessions_expire_and_use_slides_the_deadline() {
        let s = SessionStore::new("t".into()).with_idle(Duration::from_millis(80));
        let session = s.issue();
        std::thread::sleep(Duration::from_millis(50));
        assert!(
            s.get(&session.session_id).is_some(),
            "used within the window"
        );
        std::thread::sleep(Duration::from_millis(50));
        assert!(
            s.get(&session.session_id).is_some(),
            "the use slid the deadline"
        );
        std::thread::sleep(Duration::from_millis(100));
        assert!(s.get(&session.session_id).is_none(), "idle past the window");
    }

    #[test]
    fn token_comparison_is_exact() {
        let s = SessionStore::new("boot-token".into());
        assert!(s.token_matches("boot-token"));
        assert!(!s.token_matches("boot-toke"));
        assert!(!s.token_matches("boot-tokeN"));
        assert!(!s.token_matches(""));
    }

    // Rotating addresses must not escape the limit.
    #[test]
    fn failures_spread_over_many_addresses_trip_the_global_limit() {
        let (s, _d) = store();
        for i in 0..MAX_GLOBAL_FAILURES {
            s.record_failure(Some(IpAddr::from((0x0a00_0000u32 + i).to_be_bytes())));
        }
        let fresh: Option<IpAddr> = Some("198.51.100.77".parse().unwrap());
        assert!(
            s.throttled(fresh).is_some(),
            "global limit applies to new addresses too"
        );
    }
}
