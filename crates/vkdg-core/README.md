# vkdg-core

Fundamental domain types: identities, attempt state machine, `DecisionRecord`, `VkdgError`, `CapabilitySet`. Has no knowledge of HTTP, databases, WASM, or async runtimes.

## Public API

- `RequestId`, `ClientId`, `TenantId`, `ConnectionId`, `SessionKey` — ID newtypes
- `AttemptState` — states of the attempt state machine (`Received → ... → Completed | Failed`)
- `RequestEnvelope` — immutable input envelope; no credentials or response payload
- `PipelineCtx` — mutable context for an in-progress attempt; tracks current state, `committed` flag, and transition history with timestamps
- `DecisionRecord` / `ExcludedCandidate` / `AttemptResult` — auditable record of a routing decision
- `VkdgError` — typed error catalog (`thiserror`); serializable
- `Capability`, `CapabilitySet` — capabilities declared by connection/operation; lives here to avoid circular dependency between `vkdg-connections` and `vkdg-operations`

## Invariants

- No dependency on any crate that knows HTTP, DB, or WASM
- `DecisionRecord` never contains an access token or message payload
- `PipelineCtx::can_retry()` returns `false` after any call to `mark_committed()`, regardless of current state
- `PipelineCtx::new()` starts in state `Received` with one transition already recorded in history

## Focal test

```bash
cargo test -p vkdg-core
```

## Used by

All other workspace crates — it is the only crate with no internal dependencies.
