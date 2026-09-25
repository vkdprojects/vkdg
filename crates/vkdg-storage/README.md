# vkdg-storage

Storage port for async jobs and in-memory implementation. No DB dependencies — SQLite is an optional Phase B feature, not yet present in this crate.

## Public API

- `JobStore` trait — `create`, `get`, `update_state`; `async_trait`, `Send + Sync`
- `JobRecord` — persisted state of a job: `job_id`, `owner_client_id`, `connection_id`, `upstream_job_id`, `state`, `created_at`, `idempotency_key`
- `JobState` — `Queued | Running | Succeeded | Failed { reason } | Cancelled | Expired`
- `InMemoryJobStore` — thread-safe implementation via `Arc<RwLock<HashMap>>`; state is lost on restart

## Invariants

- `InMemoryJobStore::update_state` returns an error if the `job_id` does not exist — never silently creates
- The `JobStore` trait is the only stable contract; concrete implementations are deployment details
- No business logic here — only CRUD and state

## Focal test

```bash
cargo test -p vkdg-storage
```

## Used by

`bin/vkdg` (wiring of `PipelineState` in Phase C+), `tests/conformance` (async jobs in Phase D).
