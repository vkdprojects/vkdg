use chrono::{DateTime, Utc};

use crate::{AttemptState, ConnectionId, RequestEnvelope, RequestId};

// ── Pipeline context ──────────────────────────────────────────────────────────

pub struct PipelineCtx {
    pub request_id: RequestId,
    pub envelope: RequestEnvelope,
    pub state: AttemptState,
    pub connection_id: Option<ConnectionId>,
    pub committed: bool,
    pub transitions: Vec<(AttemptState, DateTime<Utc>)>,
}

impl PipelineCtx {
    pub fn new(envelope: RequestEnvelope) -> Self {
        let request_id = envelope.request_id.clone();
        Self {
            request_id,
            envelope,
            state: AttemptState::Received,
            connection_id: None,
            committed: false,
            transitions: vec![(AttemptState::Received, Utc::now())],
        }
    }

    /// Advance to the next state, recording the timestamp.
    pub fn transition(&mut self, next: AttemptState) {
        self.state = next.clone();
        self.transitions.push((next, Utc::now()));
    }

    /// Mark the request as committed — no transparent retry is allowed after this point.
    /// Must only be called once upstream has begun sending data.
    pub fn mark_committed(&mut self) {
        self.committed = true;
    }

    /// Returns false once committed; the pipeline must not attempt a transparent retry.
    pub fn can_retry(&self) -> bool {
        !self.committed
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ApiType, ClientId, TenantId};

    fn make_envelope() -> RequestEnvelope {
        RequestEnvelope {
            request_id: RequestId::new(),
            client_id: ClientId("c1".into()),
            tenant_id: TenantId("t1".into()),
            session_key: None,
            api_type: ApiType::AnthropicMessages,
            model_requested: "claude-3-5-sonnet-20241022".into(),
            deadline: None,
            mode_pack_override: None,
            compression_override: None,
            cache_bypass: false,
            include_think_tags: false,
            client_ip: None,
        }
    }

    // Plausible wrong impl: mark_committed() never sets committed=true (no-op).
    // This test defeats it: can_retry() must return false after mark_committed().
    #[test]
    fn can_retry_false_after_mark_committed() {
        let mut ctx = PipelineCtx::new(make_envelope());
        assert!(ctx.can_retry(), "before commit: retry must be allowed");
        ctx.mark_committed();
        assert!(
            !ctx.can_retry(),
            "after commit: retry must be forbidden — mark_committed() must set committed=true"
        );
    }

    // Plausible wrong impl: mark_committed() is idempotent but flips back on second call.
    // This test defeats it: calling mark_committed() twice must still yield can_retry()=false.
    #[test]
    fn mark_committed_is_idempotent() {
        let mut ctx = PipelineCtx::new(make_envelope());
        ctx.mark_committed();
        ctx.mark_committed(); // second call must not flip back
        assert!(
            !ctx.can_retry(),
            "double mark_committed must remain committed"
        );
    }

    // Plausible wrong impl: transition() updates state but does not push to transitions vec,
    // so audit log is missing.
    #[test]
    fn transition_records_state_history() {
        let mut ctx = PipelineCtx::new(make_envelope());
        // new() records Received — that's 1 entry.
        assert_eq!(ctx.transitions.len(), 1);
        assert_eq!(ctx.transitions[0].0, AttemptState::Received);

        ctx.transition(AttemptState::Admitted);
        assert_eq!(ctx.state, AttemptState::Admitted);
        assert_eq!(ctx.transitions.len(), 2);
        assert_eq!(ctx.transitions[1].0, AttemptState::Admitted);

        ctx.transition(AttemptState::UpstreamOpen);
        assert_eq!(ctx.transitions.len(), 3);
        assert_eq!(ctx.transitions[2].0, AttemptState::UpstreamOpen);
    }

    // Plausible wrong impl: transition() sets committed instead of state field,
    // so state.current never changes.
    #[test]
    fn transition_updates_current_state() {
        let mut ctx = PipelineCtx::new(make_envelope());
        ctx.transition(AttemptState::Committed);
        assert_eq!(
            ctx.state,
            AttemptState::Committed,
            "transition() must update self.state"
        );
    }

    // Plausible wrong impl: new() does not initialise transitions, leaving the vec empty,
    // so the Received entry is missing from the audit log.
    #[test]
    fn new_starts_at_received_with_one_transition() {
        let ctx = PipelineCtx::new(make_envelope());
        assert_eq!(ctx.state, AttemptState::Received);
        assert!(!ctx.committed);
        assert_eq!(
            ctx.transitions.len(),
            1,
            "Received must be logged at construction"
        );
        assert_eq!(ctx.transitions[0].0, AttemptState::Received);
    }

    // Plausible wrong impl: can_retry() checks state ordinal instead of committed flag,
    // so after transition to Committed state it returns false but after mark_committed()
    // on a non-Committed state it incorrectly returns true.
    #[test]
    fn can_retry_driven_by_committed_flag_not_state() {
        let mut ctx = PipelineCtx::new(make_envelope());
        // State is Received (far from Committed), but mark_committed() was called.
        ctx.mark_committed();
        assert!(
            !ctx.can_retry(),
            "can_retry must be driven by committed flag, not state ordinal"
        );
    }
}
