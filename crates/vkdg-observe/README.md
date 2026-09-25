# vkdg-observe

Tracing initialization, OTLP export, and `DecisionRecord` sink. Does not implement domain or routing logic.

## Public API

- `init_tracing(config: &ObserveConfig) -> anyhow::Result<()>`: installs global subscriber with `EnvFilter`; when `otlp_endpoint` is configured, adds an OpenTelemetry layer (demotes OTLP errors to warnings, never panics)
- `ObserveConfig`: observability configuration: `log_level`, `log_format` (`text` | `json`), optional `otlp_endpoint`
- `LogFormat`: enum `Text` | `Json`
- `DecisionRecordExporter`: serializes `DecisionRecord` as a structured tracing event; fire-and-forget
- `DecisionRecord`: re-exported from `vkdg-core` for convenience

## Invariants

- `init_tracing` never panics: OTLP errors are warnings
- `DecisionRecordExporter::export` does not block the response loop
- The subscriber is installed globally; calling `init_tracing` twice in tests may cause a conflict: use `tracing-subscriber`'s `try_init` in tests

## Focal test

```bash
cargo test -p vkdg-observe
```

## Used by

`bin/vkdg` (initialization at startup).
