# Changelog

All notable changes to VKDG are documented here.
Format: [keepachangelog.com](https://keepachangelog.com/en/1.1.0/).
Versioning: [Semantic Versioning](https://semver.org/).

## [0.1.0-rc1] — 2026-09-26

### Added

**Phase A — Executable specification**
- `vkdg-core`: fundamental types, attempt state machine (12 states), `DecisionRecord`, `VkdgError`, `Capability`/`CapabilitySet`
- `vkdg-operations`: contracts for `conversation.generate`, `image.generate/edit`, `video.generate/remix`, embedding, audio
- `spec/scenarios/`: 13 executable contract scenarios with RED/GREEN verified

**Phase B — Data vertical**
- `vkdg-http`: HTTP server Hyper/Tower/Axum, frontdoor, admission semaphore (503 before routing), incremental SSE parser (fragmentation at any byte, partial UTF-8, `[DONE]`)
- `vkdg-connections`: `ConnectionCatalog`, RAII `ConnectionGuard`, `CredentialManager`, `eligible_for_operation` fail-closed
- `vkdg-ingress-anthropic`: decode Anthropic Messages → Operation, encode response/stream
- `vkdg-provider-anthropic`: `AnthropicAdapter` implementing `ProviderAdapter`
- Complete pipeline: admission → routing → connection reserve → credential → upstream → passthrough
- Backpressure via `Body::from_stream` lazy; `DecisionRecord` emitted per attempt
- `vkdg-observe`: tracing + OTLP + `DecisionRecordExporter`

**Phase C — Interoperability**
- `vkdg-ingress-openai`: decode Chat Completions → Operation, encode SSE OpenAI wire
- `vkdg-provider-openai`: `OpenAIAdapter` with upstream SSE parsing (parallel tool calls included)
- Fallback 429: retry with second candidate before `committed`; `DecisionRecord` per attempt
- Routes: `/v1/messages` (Anthropic), `/v1/chat/completions` (OpenAI), `/v1/images/generations`

**Phase D — Modalities and extensions**
- `vkdg-config`: versioned `ConfigSnapshot`, hot-reload via `notify`, cross-reference validation; real `vkdg config check`
- `image.generate`: `ImageGenerateRequest/Response`, OpenAI Images API provider
- `vkdg-jobs`: `JobManager` + `SqliteJobStore` (SQLite WAL) + validated state transitions + reconciliation
- `video.generate`: `VideoGenerateRequest/RemixRequest` + async job 202 Accepted + simulated provider
- `vkdg-artifacts`: `InMemoryArtifactStore` with per-tenant ACL, TTL, purge GC
- `vkdg-plugin-host`: Wasmtime 27 + Component Model + `WIT` in `wit/provider.wit`; `install_wasm`/`uninstall` lifecycle
- `vkdg-policy-compress`: context truncation with system message preservation + loss metrics
- `ExternalServiceClient`: circuit breaker (threshold/cooldown) + deadline propagation
- `ProviderAdapter` trait: pipeline decoupled from Anthropic; adding a provider = new crate

### Architecture
- 17 crates with isolated responsibilities
- `CapabilitySet` in `vkdg-core` (no inverted coupling)
- `vkdg-http` split into domain modules: admission, frontdoor, server, app_state, pipeline, provider, sse, upstream, external_service
- `DecisionRecord` without secrets; `TokenState::Debug` omits token

### Tests
- 145 tests, 0 failing, 0 clippy warnings
- Each test documents a plausible defect it defeats
- SSE parser: fragmentation at any byte, partial UTF-8, multiple events
- Pipeline: admission before routing, RAII releases on all exit paths, fallback only before committed
- Config: cross-reference validation, hot-reload rejects invalid config

### Known gaps (Phase E targets)
- WASM `call_prepare` is a stub — full Component Model bindgen in Phase E
- `ModelSummarize` compression is a stub — requires metered internal call
- `video.generate` polling/webhook not implemented — returns 202 Accepted
- OAuth2 credential refresh is a stub — singleflight + encrypted vault in Phase E
- Admin API (`/admin/v1/`) and SvelteKit console pending — `VKDG-frontend-day0.md`
- `vkdg request explain <id>` and `vkdg replay` pending

### Breaking changes
- None (first release)

[0.1.0-rc1]: https://github.com/codeatlasdev/vkdg/releases/tag/v0.1.0-rc1
