//! SQLite-backed JobStore implementation using WAL mode.
//!
//! Uses a single `Arc<Mutex<Connection>>` writer; WAL allows concurrent readers
//! while this single writer serializes all mutations safely.

use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use uuid::Uuid;
use vkdg_core::ConnectionId;
use vkdg_storage::{JobRecord, JobState, JobStore};

// ── SqliteJobStore ────────────────────────────────────────────────────────────

/// SQLite-backed job store with WAL mode.
///
/// `Arc<Mutex<Connection>>` allows the connection to be moved into
/// `spawn_blocking` closures while sharing ownership across async tasks.
pub struct SqliteJobStore {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteJobStore {
    /// Open (or create) a SQLite database at `path` and apply WAL DDL.
    pub fn open(path: &str) -> Result<Self> {
        let conn = Connection::open(path).with_context(|| format!("sqlite open: {path}"))?;
        Self::init(conn)
    }

    /// Open an in-memory store — useful in tests and for Phase D smoke runs.
    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().context("sqlite open in-memory")?;
        Self::init(conn)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             CREATE TABLE IF NOT EXISTS jobs (
                 job_id          TEXT PRIMARY KEY,
                 owner_client_id TEXT NOT NULL,
                 connection_id   TEXT NOT NULL,
                 upstream_job_id TEXT,
                 state           TEXT NOT NULL,
                 created_at      TEXT NOT NULL,
                 idempotency_key TEXT,
                 updated_at      TEXT NOT NULL,
                 webhook_url     TEXT
             );
             CREATE UNIQUE INDEX IF NOT EXISTS idx_idempotency
                 ON jobs(idempotency_key)
                 WHERE idempotency_key IS NOT NULL;",
        )
        .context("sqlite init DDL")?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }
}

// ── JobStore impl ─────────────────────────────────────────────────────────────

#[async_trait]
impl JobStore for SqliteJobStore {
    async fn create(&self, record: JobRecord) -> Result<()> {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let guard = conn.lock().map_err(|e| anyhow!("lock poisoned: {e}"))?;
            let state_json = serde_json::to_string(&record.state).context("serialize JobState")?;
            guard
                .execute(
                    "INSERT INTO jobs
                         (job_id, owner_client_id, connection_id, upstream_job_id,
                          state, created_at, idempotency_key, updated_at, webhook_url)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![
                        record.job_id.to_string(),
                        record.owner_client_id,
                        record.connection_id.0,
                        record.upstream_job_id,
                        state_json,
                        record.created_at.to_rfc3339(),
                        record.idempotency_key,
                        Utc::now().to_rfc3339(),
                        record.webhook_url,
                    ],
                )
                .map_err(|e| {
                    // Expose unique-constraint violations clearly so callers
                    // can distinguish idempotency collisions from other errors.
                    if let rusqlite::Error::SqliteFailure(ref fe, _) = e {
                        if fe.code == rusqlite::ErrorCode::ConstraintViolation {
                            return anyhow!("idempotency key already exists");
                        }
                    }
                    anyhow!("sqlite insert: {e}")
                })?;
            Ok::<_, anyhow::Error>(())
        })
        .await
        .map_err(|e| anyhow!("spawn_blocking join: {e}"))?
    }

    async fn get(&self, job_id: Uuid) -> Result<Option<JobRecord>> {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let guard = conn.lock().map_err(|e| anyhow!("lock poisoned: {e}"))?;
            let result = guard.query_row(
                "SELECT job_id, owner_client_id, connection_id, upstream_job_id,
                        state, created_at, idempotency_key, webhook_url
                   FROM jobs WHERE job_id = ?1",
                params![job_id.to_string()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, Option<String>>(6)?,
                        row.get::<_, Option<String>>(7)?,
                    ))
                },
            );

            match result {
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(anyhow!("sqlite get: {e}")),
                Ok((id_str, owner, conn_id, upstream, state_json, created_str, idem, webhook)) => {
                    let job_id = Uuid::parse_str(&id_str)
                        .with_context(|| format!("parse job_id uuid: {id_str}"))?;
                    let state: JobState = serde_json::from_str(&state_json)
                        .with_context(|| format!("deserialize state: {state_json}"))?;
                    let created_at = DateTime::parse_from_rfc3339(&created_str)
                        .with_context(|| format!("parse created_at: {created_str}"))?
                        .with_timezone(&Utc);
                    Ok(Some(JobRecord {
                        job_id,
                        owner_client_id: owner,
                        connection_id: ConnectionId(conn_id),
                        upstream_job_id: upstream,
                        state,
                        created_at,
                        idempotency_key: idem,
                        webhook_url: webhook,
                    }))
                }
            }
        })
        .await
        .map_err(|e| anyhow!("spawn_blocking join: {e}"))?
    }

    async fn update_state(&self, job_id: Uuid, state: JobState) -> Result<()> {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let guard = conn.lock().map_err(|e| anyhow!("lock poisoned: {e}"))?;
            let state_json = serde_json::to_string(&state).context("serialize JobState")?;
            let rows = guard
                .execute(
                    "UPDATE jobs SET state = ?1, updated_at = ?2 WHERE job_id = ?3",
                    params![state_json, Utc::now().to_rfc3339(), job_id.to_string()],
                )
                .map_err(|e| anyhow!("sqlite update: {e}"))?;
            if rows == 0 {
                Err(anyhow!("job {} not found", job_id))
            } else {
                Ok(())
            }
        })
        .await
        .map_err(|e| anyhow!("spawn_blocking join: {e}"))?
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use vkdg_storage::JobStore;

    fn sample_record(idempotency_key: Option<&str>) -> JobRecord {
        JobRecord {
            job_id: Uuid::new_v4(),
            owner_client_id: "client-a".to_string(),
            connection_id: ConnectionId("conn-1".to_string()),
            upstream_job_id: None,
            state: JobState::Queued,
            created_at: Utc::now(),
            idempotency_key: idempotency_key.map(str::to_string),
            webhook_url: None,
        }
    }

    /// Plausible defect: create() silently overwrites existing job_id.
    #[tokio::test]
    async fn create_and_get_job() {
        let store = SqliteJobStore::in_memory().unwrap();
        let record = sample_record(None);
        let id = record.job_id;
        store.create(record).await.unwrap();
        let fetched = store.get(id).await.unwrap().expect("record must exist");
        assert_eq!(fetched.job_id, id);
        assert_eq!(fetched.owner_client_id, "client-a");
        assert!(matches!(fetched.state, JobState::Queued));
    }

    /// Plausible defect: duplicate idempotency key silently creates a second job.
    #[tokio::test]
    async fn idempotent_create() {
        let store = SqliteJobStore::in_memory().unwrap();
        let mut r1 = sample_record(Some("key-42"));
        let mut r2 = sample_record(Some("key-42"));
        r1.job_id = Uuid::new_v4();
        r2.job_id = Uuid::new_v4();
        store.create(r1).await.unwrap();
        let err = store.create(r2).await;
        assert!(
            err.is_err(),
            "second insert with same idempotency_key must fail"
        );
        let msg = err.unwrap_err().to_string();
        assert!(
            msg.contains("idempotency"),
            "error should mention idempotency, got: {msg}"
        );
    }

    /// Plausible defect: state update overwrites record with wrong job_id.
    #[tokio::test]
    async fn update_state_persists() {
        let store = SqliteJobStore::in_memory().unwrap();
        let record = sample_record(None);
        let id = record.job_id;
        store.create(record).await.unwrap();
        store.update_state(id, JobState::Running).await.unwrap();
        let fetched = store.get(id).await.unwrap().unwrap();
        assert!(matches!(fetched.state, JobState::Running));
    }

    /// Plausible defect: get() panics or errors on missing row instead of Ok(None).
    #[tokio::test]
    async fn job_not_found() {
        let store = SqliteJobStore::in_memory().unwrap();
        let result = store.get(Uuid::new_v4()).await.unwrap();
        assert!(result.is_none());
    }

    /// Plausible defect: in_memory() tries to access filesystem and fails in CI.
    #[tokio::test]
    async fn in_memory_store_no_filesystem() {
        // Must succeed without touching any path on disk.
        let store = SqliteJobStore::in_memory().unwrap();
        let record = sample_record(None);
        let id = record.job_id;
        store.create(record).await.unwrap();
        assert!(store.get(id).await.unwrap().is_some());
    }
}
