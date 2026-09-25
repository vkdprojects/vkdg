# vkdg-jobs

Async job manager for long-running media operations (video generation, batch inference). Manages job lifecycle state transitions and persistence via `SqliteJobStore`.

## Public API

- `JobManager`: creates, updates, and reconciles jobs; validates state transitions
- `SqliteJobStore`: SQLite WAL-backed implementation of the `JobStore` trait from `vkdg-storage`
- `JobTransition`: valid state machine edges; `JobManager` rejects invalid transitions
- `reconcile(store: &dyn JobStore)`: scans for stale `Running` jobs and marks them `Failed` after a timeout

## Invariants

- State transitions are validated; moving from `Succeeded` or `Failed` to any other state returns an error
- `SqliteJobStore` uses WAL mode for concurrent reads without blocking writes
- `reconcile` is idempotent; safe to call from a background task on a schedule

## Focal test

```bash
cargo test -p vkdg-jobs
```

## Used by

`bin/vkdg` (wired into `PipelineState` in Phase D+), `tests/conformance` (async job scenarios).
