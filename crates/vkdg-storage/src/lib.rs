//! Storage port traits and in-memory implementation.
//!
//! SQLite-backed implementation is optional (Phase B). Only port traits and an
//! in-memory store live here so the rest of the workspace has zero DB deps.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use uuid::Uuid;

use vkdg_core::ConnectionId;

// ── JobState ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum JobState {
    Queued,
    Running,
    Succeeded,
    Failed { reason: String },
    Cancelled,
    Expired,
}

// ── JobRecord ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobRecord {
    pub job_id: Uuid,
    pub owner_client_id: String,
    pub connection_id: ConnectionId,
    pub upstream_job_id: Option<String>,
    pub state: JobState,
    pub created_at: DateTime<Utc>,
    pub idempotency_key: Option<String>,
}

// ── JobStore trait ────────────────────────────────────────────────────────────

#[async_trait]
pub trait JobStore: Send + Sync {
    async fn create(&self, record: JobRecord) -> Result<()>;
    async fn get(&self, job_id: Uuid) -> Result<Option<JobRecord>>;
    async fn update_state(&self, job_id: Uuid, state: JobState) -> Result<()>;
}

// ── InMemoryJobStore ──────────────────────────────────────────────────────────

#[derive(Clone, Default)]
pub struct InMemoryJobStore {
    inner: Arc<RwLock<HashMap<Uuid, JobRecord>>>,
}

impl InMemoryJobStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl JobStore for InMemoryJobStore {
    async fn create(&self, record: JobRecord) -> Result<()> {
        self.inner.write().await.insert(record.job_id, record);
        Ok(())
    }

    async fn get(&self, job_id: Uuid) -> Result<Option<JobRecord>> {
        Ok(self.inner.read().await.get(&job_id).cloned())
    }

    async fn update_state(&self, job_id: Uuid, state: JobState) -> Result<()> {
        let mut map = self.inner.write().await;
        match map.get_mut(&job_id) {
            Some(record) => {
                record.state = state;
                Ok(())
            }
            None => anyhow::bail!("job {} not found", job_id),
        }
    }
}
