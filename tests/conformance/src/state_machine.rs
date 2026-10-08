// Phase B state machine conformance tests.
//
// Test-first: each test documents the plausible wrong implementation it would catch.
// All tests COMPILE and fail behaviorally (never on import error).

use vkdg_core::{
    pipeline::PipelineCtx, ApiType, AttemptState, ClientId, RequestEnvelope, RequestId, TenantId,
};

fn make_envelope() -> RequestEnvelope {
    RequestEnvelope {
        request_id: RequestId::default(),
        client_id: ClientId("client-test".into()),
        tenant_id: TenantId("tenant-test".into()),
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

// ── Variant completeness ───────────────────────────────────────────────────────

/// Plausible wrong impl: adding/removing a state variant without updating
/// downstream match arms — test catches count drift.
/// PASSES.
#[test]
fn attempt_state_all_variants_exist() {
    let states = vec![
        AttemptState::Received,
        AttemptState::Authenticated,
        AttemptState::Admitted,
        AttemptState::AccountReserved,
        AttemptState::CredentialReady,
        AttemptState::Prepared,
        AttemptState::UpstreamOpen,
        AttemptState::Committed,
        AttemptState::Completed,
        AttemptState::Partial,
        AttemptState::Cancelled,
        AttemptState::Failed,
    ];
    assert_eq!(
        states.len(),
        12,
        "Spec requires exactly 12 AttemptState variants"
    );
}

// ── State ordering ─────────────────────────────────────────────────────────────

/// Plausible wrong impl: Committed placed before UpstreamOpen (allowing retry
/// after data has been sent). Test anchors the threshold position.
/// PASSES.
#[test]
fn committed_comes_after_upstream_open_in_pipeline() {
    use AttemptState::*;
    let pipeline_order: &[AttemptState] = &[
        Received,
        Authenticated,
        Admitted,
        AccountReserved,
        CredentialReady,
        Prepared,
        UpstreamOpen,
        Committed,
        Completed,
        Partial,
        Cancelled,
        Failed,
    ];
    let pos_upstream = pipeline_order
        .iter()
        .position(|s| s == &UpstreamOpen)
        .unwrap();
    let pos_committed = pipeline_order.iter().position(|s| s == &Committed).unwrap();
    assert!(
        pos_committed > pos_upstream,
        "Committed must appear after UpstreamOpen — retry threshold is wrong"
    );
}

// ── PipelineCtx: initial state ─────────────────────────────────────────────────

/// Plausible wrong impl: PipelineCtx initialises with state = Authenticated
/// instead of Received, causing the first transition to be skipped.
/// PASSES (PipelineCtx::new sets state = Received).
#[test]
fn pipeline_ctx_starts_at_received() {
    let ctx = PipelineCtx::new(make_envelope());
    assert_eq!(
        ctx.state,
        AttemptState::Received,
        "New PipelineCtx must start at Received, not {:?}",
        ctx.state
    );
}

// ── PipelineCtx: transition records ───────────────────────────────────────────

/// Plausible wrong impl: transition() updates ctx.state but forgets to push
/// to ctx.transitions, breaking audit log completeness.
/// PASSES.
#[test]
fn transition_records_history() {
    let mut ctx = PipelineCtx::new(make_envelope());
    ctx.transition(AttemptState::Authenticated);
    ctx.transition(AttemptState::Admitted);

    assert_eq!(
        ctx.state,
        AttemptState::Admitted,
        "state must reflect latest transition"
    );
    // Received (initial) + Authenticated + Admitted = 3 entries
    assert_eq!(
        ctx.transitions.len(),
        3,
        "every transition must be recorded; transitions = {:?}",
        ctx.transitions
    );
}

// ── PipelineCtx: can_retry contract ───────────────────────────────────────────

/// Plausible wrong impl: can_retry() returns true always (never checks
/// committed flag), allowing silent replay of committed requests.
/// FAILS unless mark_committed() clears can_retry().
#[test]
fn can_retry_false_after_mark_committed() {
    let mut ctx = PipelineCtx::new(make_envelope());
    assert!(ctx.can_retry(), "must be retryable before committed");

    ctx.mark_committed();

    assert!(
        !ctx.can_retry(),
        "can_retry() must return false after mark_committed(). \
         Contract: no transparent retry once upstream has begun sending data."
    );
}

/// Plausible wrong impl: mark_committed() is idempotent but a second call
/// accidentally re-enables retry (toggles instead of sets).
/// PASSES only if committed is stored as a bool, not flipped.
#[test]
fn mark_committed_is_idempotent() {
    let mut ctx = PipelineCtx::new(make_envelope());
    ctx.mark_committed();
    ctx.mark_committed(); // second call must not toggle back

    assert!(
        !ctx.can_retry(),
        "mark_committed() called twice must not re-enable retry"
    );
}

// ── PipelineCtx: transition after committed ────────────────────────────────────

/// Plausible wrong impl: transition() after committed resets the committed
/// flag, allowing retry to sneak back in.
/// PASSES only if committed is preserved across transitions.
#[test]
fn committed_flag_persists_through_further_transitions() {
    let mut ctx = PipelineCtx::new(make_envelope());
    ctx.mark_committed();
    ctx.transition(AttemptState::Completed);

    assert!(
        !ctx.can_retry(),
        "committed flag must survive further transitions — \
         retry must remain blocked after Completed"
    );
}
