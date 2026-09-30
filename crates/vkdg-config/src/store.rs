//! Persistent store for connections and routes.
//!
//! [`GatewayStore`] is the single source of truth for connections and routes
//! at runtime. It follows the same pattern as `AccountStore` in
//! `vkdg-connections`: SQLite + `parking_lot::Mutex`, WAL mode, 0600 perms.

use std::path::Path;

use parking_lot::Mutex;
use rusqlite::{params, Connection};
use vkdg_core::{Result, VkdgError};

use crate::schema::{ConnectionDef, RouteDef};

/// SQLite-backed store for gateway connections and routes.
pub struct GatewayStore {
    conn: Mutex<Connection>,
}

impl GatewayStore {
    /// Open (or create) the store at `path`. On Unix the parent directory is
    /// created `0700` and the file is forced to `0600` before any data is written.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            create_private_dir(parent)?;
        }
        create_private_file(path)?;
        let conn = Connection::open(path).map_err(|e| store_err("open", &e))?;
        Self::init(conn)
    }

    /// In-memory store for tests.
    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().map_err(|e| store_err("open", &e))?;
        Self::init(conn)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE IF NOT EXISTS connections (
                 id       TEXT PRIMARY KEY NOT NULL,
                 def_json TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS routes (
                 id       TEXT PRIMARY KEY NOT NULL,
                 def_json TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS meta (
                 key   TEXT PRIMARY KEY NOT NULL,
                 value TEXT NOT NULL
             );",
        )
        .map_err(|e| store_err("init", &e))?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Mark the store as seeded so restarts with an empty table don't re-seed.
    pub fn mark_seeded(&self) -> Result<()> {
        self.lock()
            .execute(
                "INSERT OR REPLACE INTO meta (key, value) VALUES ('seeded', '1')",
                [],
            )
            .map_err(|e| store_err("mark_seeded", &e))?;
        Ok(())
    }

    /// True only if the store has never been seeded (first boot).
    pub fn needs_seed(&self) -> Result<bool> {
        let conn = self.lock();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM meta WHERE key = 'seeded'", [], |r| {
                r.get(0)
            })
            .map_err(|e| store_err("needs_seed", &e))?;
        Ok(count == 0)
    }

    // ── Read ──────────────────────────────────────────────────────────────────

    /// Load all connections and routes. Empty vecs when no rows exist.
    pub fn load(&self) -> Result<(Vec<ConnectionDef>, Vec<RouteDef>)> {
        let conn = self.lock();

        let mut stmt = conn
            .prepare("SELECT def_json FROM connections ORDER BY id")
            .map_err(|e| store_err("load connections", &e))?;
        let connections: Vec<ConnectionDef> = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| store_err("load connections", &e))?
            .map(|r| {
                r.map_err(|e| store_err("load connections row", &e))
                    .and_then(|s| decode_connection(&s))
            })
            .collect::<Result<_>>()?;

        let mut stmt = conn
            .prepare("SELECT def_json FROM routes ORDER BY id")
            .map_err(|e| store_err("load routes", &e))?;
        let routes: Vec<RouteDef> = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| store_err("load routes", &e))?
            .map(|r| {
                r.map_err(|e| store_err("load routes row", &e))
                    .and_then(|s| decode_route(&s))
            })
            .collect::<Result<_>>()?;

        Ok((connections, routes))
    }

    /// True when both tables are empty.
    pub fn is_empty(&self) -> Result<bool> {
        let conn = self.lock();
        let conn_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM connections", [], |r| r.get(0))
            .map_err(|e| store_err("is_empty connections", &e))?;
        if conn_count > 0 {
            return Ok(false);
        }
        let route_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM routes", [], |r| r.get(0))
            .map_err(|e| store_err("is_empty routes", &e))?;
        Ok(route_count == 0)
    }

    // ── Connections ───────────────────────────────────────────────────────────

    /// Replace all connections atomically (delete all, then insert).
    pub fn set_connections(&self, defs: &[ConnectionDef]) -> Result<()> {
        let mut conn = self.lock();
        let tx = conn
            .transaction()
            .map_err(|e| store_err("set_connections tx", &e))?;
        tx.execute("DELETE FROM connections", [])
            .map_err(|e| store_err("set_connections delete", &e))?;
        for def in defs {
            let json = encode_connection(def)?;
            tx.execute(
                "INSERT INTO connections (id, def_json) VALUES (?1, ?2)",
                params![def.id, json],
            )
            .map_err(|e| store_err("set_connections insert", &e))?;
        }
        tx.commit()
            .map_err(|e| store_err("set_connections commit", &e))
    }

    /// Upsert one connection by id.
    pub fn upsert_connection(&self, def: &ConnectionDef) -> Result<()> {
        let json = encode_connection(def)?;
        self.lock()
            .execute(
                "INSERT OR REPLACE INTO connections (id, def_json) VALUES (?1, ?2)",
                params![def.id, json],
            )
            .map_err(|e| store_err("upsert_connection", &e))?;
        Ok(())
    }

    /// Delete one connection by id. Ok if not found.
    pub fn delete_connection(&self, id: &str) -> Result<()> {
        self.lock()
            .execute("DELETE FROM connections WHERE id = ?1", params![id])
            .map_err(|e| store_err("delete_connection", &e))?;
        Ok(())
    }

    // ── Routes ────────────────────────────────────────────────────────────────

    /// Replace all routes atomically (delete all, then insert).
    pub fn set_routes(&self, defs: &[RouteDef]) -> Result<()> {
        let mut conn = self.lock();
        let tx = conn
            .transaction()
            .map_err(|e| store_err("set_routes tx", &e))?;
        tx.execute("DELETE FROM routes", [])
            .map_err(|e| store_err("set_routes delete", &e))?;
        for def in defs {
            let json = encode_route(def)?;
            tx.execute(
                "INSERT INTO routes (id, def_json) VALUES (?1, ?2)",
                params![def.id, json],
            )
            .map_err(|e| store_err("set_routes insert", &e))?;
        }
        tx.commit().map_err(|e| store_err("set_routes commit", &e))
    }

    /// Upsert one route by id.
    pub fn upsert_route(&self, def: &RouteDef) -> Result<()> {
        let json = encode_route(def)?;
        self.lock()
            .execute(
                "INSERT OR REPLACE INTO routes (id, def_json) VALUES (?1, ?2)",
                params![def.id, json],
            )
            .map_err(|e| store_err("upsert_route", &e))?;
        Ok(())
    }

    /// Delete one route by id. Ok if not found.
    pub fn delete_route(&self, id: &str) -> Result<()> {
        self.lock()
            .execute("DELETE FROM routes WHERE id = ?1", params![id])
            .map_err(|e| store_err("delete_route", &e))?;
        Ok(())
    }

    fn lock(&self) -> parking_lot::MutexGuard<'_, Connection> {
        self.conn.lock()
    }
}

// ── Encode/decode helpers ──────────────────────────────────────────────────────

fn encode_connection(def: &ConnectionDef) -> Result<String> {
    serde_json::to_string(def)
        .map_err(|e| VkdgError::Internal(format!("gateway store: encode connection: {e}")))
}

fn decode_connection(s: &str) -> Result<ConnectionDef> {
    serde_json::from_str(s)
        .map_err(|e| VkdgError::Internal(format!("gateway store: decode connection: {e}")))
}

fn encode_route(def: &RouteDef) -> Result<String> {
    serde_json::to_string(def)
        .map_err(|e| VkdgError::Internal(format!("gateway store: encode route: {e}")))
}

fn decode_route(s: &str) -> Result<RouteDef> {
    serde_json::from_str(s)
        .map_err(|e| VkdgError::Internal(format!("gateway store: decode route: {e}")))
}

fn store_err(op: &str, e: &rusqlite::Error) -> VkdgError {
    VkdgError::Internal(format!("gateway store {op}: {e}"))
}

fn io_err(op: &str, path: &Path, e: &std::io::Error) -> VkdgError {
    VkdgError::Internal(format!("gateway store {op} {}: {e}", path.display()))
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
    // Create the file with 0600 if it doesn't exist yet.
    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)
        .map_err(|e| io_err("create file", path, &e))?;
    // Force permissions even if the file already existed.
    let perms = std::fs::Permissions::from_mode(0o600);
    std::fs::set_permissions(path, perms).map_err(|e| io_err("set permissions", path, &e))
}

#[cfg(not(unix))]
fn create_private_file(_path: &Path) -> Result<()> {
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::AuthDef;

    fn sample_connection(id: &str) -> ConnectionDef {
        ConnectionDef {
            id: id.to_string(),
            provider: "openai".to_string(),
            base_url: None,
            endpoint: None,
            auth: AuthDef::ApiKey {
                env_var: "OPENAI_API_KEY".to_string(),
            },
            models: vec!["gpt-4o".to_string()],
            max_concurrent: Some(4),
            weight: Some(1),
            tags: vec!["primary".to_string()],
        }
    }

    fn sample_route(id: &str) -> RouteDef {
        RouteDef {
            id: id.to_string(),
            match_models: vec!["gpt-4o".to_string()],
            strategy: "round_robin".to_string(),
            targets: vec!["conn-a".to_string()],
            hooks: Default::default(),
        }
    }

    #[test]
    fn store_and_load_roundtrip() {
        let store = GatewayStore::in_memory().unwrap();
        let conn_def = sample_connection("conn-a");
        let route_def = sample_route("route-1");

        store.upsert_connection(&conn_def).unwrap();
        store.upsert_route(&route_def).unwrap();

        let (conns, routes) = store.load().unwrap();

        assert_eq!(conns.len(), 1);
        assert_eq!(conns[0].id, "conn-a");
        assert_eq!(conns[0].provider, "openai");
        assert_eq!(conns[0].models, vec!["gpt-4o"]);
        assert_eq!(conns[0].max_concurrent, Some(4));

        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].id, "route-1");
        assert_eq!(routes[0].strategy, "round_robin");
        assert_eq!(routes[0].targets, vec!["conn-a"]);
    }

    #[test]
    fn delete_removes_entry() {
        let store = GatewayStore::in_memory().unwrap();
        store.upsert_connection(&sample_connection("a")).unwrap();
        store.upsert_connection(&sample_connection("b")).unwrap();

        store.delete_connection("a").unwrap();

        let (conns, _) = store.load().unwrap();
        assert_eq!(conns.len(), 1);
        assert_eq!(conns[0].id, "b");
    }

    #[test]
    fn is_empty_on_fresh_store() {
        let store = GatewayStore::in_memory().unwrap();
        assert!(store.is_empty().unwrap());

        store.upsert_connection(&sample_connection("x")).unwrap();
        assert!(!store.is_empty().unwrap());
    }

    #[test]
    fn delete_nonexistent_is_ok() {
        let store = GatewayStore::in_memory().unwrap();
        // Should not error even when the row doesn't exist.
        store.delete_connection("ghost").unwrap();
        store.delete_route("ghost").unwrap();
    }

    #[test]
    fn set_connections_replaces_all() {
        let store = GatewayStore::in_memory().unwrap();
        store.upsert_connection(&sample_connection("old")).unwrap();

        let new_batch = vec![sample_connection("new-1"), sample_connection("new-2")];
        store.set_connections(&new_batch).unwrap();

        let (conns, _) = store.load().unwrap();
        assert_eq!(conns.len(), 2);
        assert!(conns.iter().all(|c| c.id.starts_with("new-")));
    }

    #[test]
    fn set_routes_replaces_all() {
        let store = GatewayStore::in_memory().unwrap();
        store.upsert_route(&sample_route("old")).unwrap();

        let new_batch = vec![sample_route("r1"), sample_route("r2")];
        store.set_routes(&new_batch).unwrap();

        let (_, routes) = store.load().unwrap();
        assert_eq!(routes.len(), 2);
    }
}
