# vkdg-http

Gateway HTTP server: frontdoor, admission guard, SSE parser, upstream HTTP client, and pipeline runner. Does not implement routing business logic or input protocol decoding.

## Public API

- `ServerConfig`: server configuration (address, body limits, maximum concurrency)
- `AdmissionGuard`: RAII semaphore for concurrency control; releases automatically on drop
- `FrontDoor`: accepts connections, assigns `request_id`, applies header and body limits
- `PipelineState`: shared state of the complete pipeline (catalog, router, credentials, http client); optional in `AppState` for tests that only need admission
- `AppState`: state injected into Axum handlers; cloneable, thread-safe
- `build_router(state: AppState) -> Router`: builds the `axum::Router` with routes `/v1/messages`, `/health`, `/info`
- `serve(config: ServerConfig, router: Router)`: starts the server and waits for a shutdown signal
- `sse` module: incremental SSE parser; `upstream` module: `HttpClient` that forwards to providers

## Invariants

- The `AdmissionGuard` semaphore is always released, even on panic (`Drop` implemented)
- `PipelineState` is optional; `AppState` without a pipeline returns 501 on `/v1/messages`
- The `/v1/messages` handler checks admission before any payload decoding

## Focal test

```bash
cargo test -p vkdg-http
```

## Used by

`bin/vkdg` (full wiring), `vkdg-ingress-anthropic` (imports `AppState`), `tests/conformance`.
