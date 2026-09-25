# vkdg-artifacts

In-memory artifact store with per-tenant ACL, TTL, and GC purge. Used for short-lived binary results (generated images, video thumbnails) that do not warrant external object storage.

## Public API

- `InMemoryArtifactStore` — thread-safe store via `Arc<RwLock<HashMap>>`; implements `ArtifactStore`
- `ArtifactStore` trait — `store`, `get`, `delete`, `purge_expired`
- `ArtifactMeta` — metadata: `artifact_id`, `owner_tenant`, `content_type`, `size`, `expires_at`
- `ArtifactError` — typed errors: `NotFound`, `Forbidden`, `Expired`, `StoreFull`

## Invariants

- `get` checks `owner_tenant` before returning data; cross-tenant access returns `Forbidden`
- `get` checks TTL and returns `Expired` for stale artifacts without deleting them
- `purge_expired` removes all artifacts past their TTL; safe to call from a background task
- State is not persisted across restarts — `InMemoryArtifactStore` is for ephemeral results only

## Focal test

```bash
cargo test -p vkdg-artifacts
```

## Used by

`bin/vkdg` (wired into `PipelineState` in Phase D+), `tests/conformance` (artifact ACL scenarios).
