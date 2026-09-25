# vkdg-config

Versioned configuration snapshot with hot-reload. Parses YAML config into typed structs, validates cross-references, and publishes immutable `Arc<ConfigSnapshot>` revisions via a `tokio::sync::watch` channel.

## Public API

- `ConfigSnapshot` — immutable, validated configuration revision; holds `connections`, `routes`, `limits`, `observe`, and a monotonic `version`
- `ConfigSnapshot::build(raw: RawConfig) -> Result<Self>` — validates cross-references (duplicate IDs, unknown route targets, unknown strategies) and builds runtime types
- `watch(path: &Path) -> (Arc<ConfigSnapshot>, watch::Receiver<Arc<ConfigSnapshot>>)` — starts a filesystem watcher via `notify`; invalid reloads are rejected and the current snapshot is kept
- `RawConfig`, `ConnectionDef`, `RouteDef`, `LimitsDef`, `ObserveDef` — serde-deserializable config structs

## Invariants

- An invalid reload never replaces the active snapshot — receivers always see a valid `ConfigSnapshot`
- In-flight requests hold a strong `Arc` reference to the snapshot they started with; config changes do not affect them
- `ConfigSnapshot::version` is monotonically increasing; a rejected reload does not advance the counter

## Focal test

```bash
cargo test -p vkdg-config
```

## Used by

`bin/vkdg` (startup and hot-reload), `vkdg-cli` (`config check` command).
