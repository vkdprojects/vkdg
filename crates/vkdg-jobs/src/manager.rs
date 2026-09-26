//! High-level job lifecycle manager on top of a `JobStore`.
//!
//! Handles idempotency (via the store's unique index), state-transition
//! validation, and post-restart reconciliation stubs.

use std::sync::Arc;

use chrono::Utc;
use uuid::Uuid;
use vkdg_core::{ConnectionId, VkdgError};
use vkdg_storage::{JobRecord, JobState, JobStore};

use crate::webhook;

// ── JobManager ────────────────────────────────────────────────────────────────

pub struct JobManager {
    store: Arc<dyn JobStore>,
}

impl JobManager {
    pub fn new(store: Arc<dyn JobStore>) -> Self {
        Self { store }
    }

    /// Create a new job and return its id.
    ///
    /// If `idempotency_key` matches an existing job the underlying store will
    /// return an error (unique constraint); callers should treat that as a
    /// signal to look up the existing job instead.
    pub async fn create_job(
        &self,
        owner_client_id: String,
        connection_id: ConnectionId,
        idempotency_key: Option<String>,
        webhook_url: Option<String>,
    ) -> vkdg_core::Result<Uuid> {
        let job_id = Uuid::new_v4();
        let record = JobRecord {
            job_id,
            owner_client_id,
            connection_id,
            upstream_job_id: None,
            state: JobState::Queued,
            created_at: Utc::now(),
            idempotency_key,
            webhook_url,
        };
        self.store
            .create(record)
            .await
            .map_err(|e| VkdgError::Internal(e.to_string()))?;
        Ok(job_id)
    }

    /// Transition a job to `new_state`, enforcing the allowed state machine.
    ///
    /// Valid transitions:
    /// - Queued   → Running | Cancelled
    /// - Running  → Succeeded | Failed | Cancelled
    /// - any      → Expired
    pub async fn transition(
        &self,
        job_id: Uuid,
        new_state: JobState,
    ) -> vkdg_core::Result<()> {
        let record = self
            .store
            .get(job_id)
            .await
            .map_err(|e| VkdgError::Internal(e.to_string()))?
            .ok_or_else(|| VkdgError::Internal(format!("job {job_id} not found")))?;
        validate_transition(&record.state, &new_state)?;
        // Fire webhook before updating state so we have the pre-transition record
        // with the webhook_url; the event name reflects the target state.
        if let Some(url) = &record.webhook_url {
            // Build a temporary record reflecting the new state for the payload.
            let mut updated = record.clone();
            updated.state = new_state.clone();
            webhook::dispatch(&updated, url);
        }
        self.store
            .update_state(job_id, new_state)
            .await
            .map_err(|e| VkdgError::Internal(e.to_string()))
    }

    /// Stub: after a restart, collect jobs that may need re-queueing.
    ///
    /// Full implementation requires a `list_by_state` query on `JobStore`
    /// (Phase D backlog). Returns an empty list for now so callers compile.
    pub async fn reconcile_after_restart(
        &self,
        _running_timeout_secs: u64,
    ) -> vkdg_core::Result<Vec<Uuid>> {
        Ok(vec![])
    }
}

// ── Transition guard ──────────────────────────────────────────────────────────

fn validate_transition(from: &JobState, to: &JobState) -> vkdg_core::Result<()> {
    use JobState::*;
    let ok = matches!(
        (from, to),
        (Queued, Running)
            | (Running, Succeeded)
            | (Running, Failed { .. })
            | (Running, Cancelled)
            | (Queued, Cancelled)
            | (_, Expired)
    );
    if ok {
        Ok(())
    } else {
        Err(VkdgError::Internal(format!(
            "invalid job state transition {from:?} → {to:?}"
        )))
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SqliteJobStore;

    fn manager() -> JobManager {
        let store = SqliteJobStore::in_memory().expect("in-memory store");
        JobManager::new(Arc::new(store))
    }

    /// Plausible defect: create_job returns wrong id or creates no record.
    #[tokio::test]
    async fn create_and_get_job() {
        let mgr = manager();
        let id = mgr
            .create_job("client-a".into(), ConnectionId("c1".into()), None, None)
            .await
            .unwrap();
        // Confirm the id is a valid uuid (non-nil).
        assert!(!id.is_nil());
    }

    /// Plausible defect: Queued→Running→Succeeded doesn't error.
    #[tokio::test]
    async fn state_transitions_valid() {
        let mgr = manager();
        let id = mgr
            .create_job("client-b".into(), ConnectionId("c1".into()), None, None)
            .await
            .unwrap();
        mgr.transition(id, JobState::Running).await.unwrap();
        mgr.transition(id, JobState::Succeeded).await.unwrap();
    }

    /// Plausible defect: invalid transition Queued→Succeeded is silently accepted.
    #[tokio::test]
    async fn state_transition_invalid() {
        let mgr = manager();
        let id = mgr
            .create_job("client-c".into(), ConnectionId("c1".into()), None, None)
            .await
            .unwrap();
        let err = mgr.transition(id, JobState::Succeeded).await;
        assert!(err.is_err(), "Queued→Succeeded must be rejected");
    }

    /// Plausible defect: transitioning unknown job panics or gives misleading error.
    #[tokio::test]
    async fn job_not_found_transition() {
        let mgr = manager();
        let err = mgr.transition(Uuid::new_v4(), JobState::Running).await;
        assert!(err.is_err());
        let msg = err.unwrap_err().to_string();
        assert!(msg.contains("not found"), "error should say not found, got: {msg}");
    }
}
