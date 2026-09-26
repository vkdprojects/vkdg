# Changelog

All notable changes to VKDG are documented here.
Format: [keepachangelog.com](https://keepachangelog.com/en/1.1.0/).
Versioning: [Semantic Versioning](https://semver.org/).

## [0.1.0-rc1]: 2026-09-26

### Added

**Session 2026-09-26 (plugin-first architecture and OmniRoute parity)**

Plugin system:
- WIT interfaces for all 5 plugin types (router, compressor, provider, cache, auth)
- PluginRole + PluginChain with fail-closed semantics for auth
- Cache backends: SQLite exact-match (zero-infra default), Redis/Valkey/DragonflyDB
- SHA-256 cache key derivation (model + messages, excludes temperature)

Routing:
- Auto-scorer with 5 mode packs (ship-fast, cost-saver, quality-first, offline-friendly, balanced)
- ScoredStrategy executing with live quota and latency signals via RoutingHints
- RoutingHints: quota_headroom from QuotaTracker, latency_p50_ms from LatencyTracker (EWMA)
- Session stickiness: SessionRegistry TTL-based pin, preferred connection on multi-turn

Compression:
- CavemanCompressor: 26 regex rules targeting preamble and filler (~30% savings)
- RtkCompressor: content-class detection (stack trace, JSON, file list, diff, command) with class-specific filters
- StackedCompressor: RTK->Caveman pipeline (78-95% savings on tool outputs)
- Compression threshold from combo CompressionPolicy; skip via X-VKDG-Compression: none

Combos:
- Named routing plans with own strategy, compression, cache, and budget policies
- ComboResolver: exact ID match -> glob pattern -> bare model routing
- Active in pipeline: combo targets override route result, combo compression threshold respected

Credentials and OAuth:
- OAuth2 client_credentials flow with per-connection singleflight and conditional generation write
- Cooldown: exponential backoff (2^(n-1)s, cap 300s, jitter) wired to 429/5xx upstream errors

Pipeline additions:
- IP allowlist/blocklist: extract_client_ip (X-Forwarded-For, X-Real-IP) + IpPolicy step 0
- Think-tag filtering: SseParser strips <think>...</think> by default (opt-in via X-VKDG-Think-Tags: include)
- Override headers: X-VKDG-Mode, X-VKDG-Compression, X-VKDG-Cache, X-VKDG-Think-Tags
- DedupTable RAII: first caller proceeds, subsequent callers register + complete (anti-thundering-herd)
- Global system prompt injection from config
- Memory injection: MemoryStore retrieval prepended to system context
- Eval scoring: EvalScorer records latency/quality score per response

New crates:
- vkdg-cache: CacheBackend trait, SqliteExactCache, RedisExactCache (feature-gated)
- vkdg-combos: Combo, ComboResolver, CompressionPolicy, CachePolicy, BudgetPolicy
- vkdg-memory: MemoryStore, extract_facts, inject_memories
- vkdg-eval: EvalScorer, EvalResult, LatencyMetrics

CLI additions:
- vkdg request explain <id>: fetch and display DecisionRecord from admin API
- vkdg config explain --model <name>: simulate routing without consuming quota
- vkdg replay <fixture>: send recorded request to gateway and optionally assert response
- vkdg doctor: expanded to 6 checks (rustc, RUST_LOG, data plane, admin API, API key, admin URL)

Jobs:
- Webhook dispatch on job state transitions (fire-and-forget, 10s timeout)
- webhook_url field on JobRecord and SQLite store

MCP:
- GET /mcp: minimal tool discovery (route_preview, gateway_health)

Tests:
- 19 BFF tests (msw mock server): getSystem, login, listConnections, previewRoute, etc.
- 6 new spec/scenarios for new contracts
- 250 total Rust tests, 0 failing, 0 clippy warnings

**Phase A: Executable specification**
- `vkdg-core`: fundamental types, attempt state machine (12 states), `DecisionRecord`, `VkdgError`, `Capability`/`CapabilitySet`
- `vkdg-operations`: contracts for `conversation.generate`, `image.generate/edit`, `video.generate/remix`, embedding, audio
- `spec/scenarios/`: 13 executable contract scenarios with RED/GREEN verified

**Phase B: Data vertical**
- `vkdg-http`: HTTP server Hyper/Tower/Axum, frontdoor, admission semaphore (503 before routing), incremental SSE parser (fragmentation at any byte, partial UTF-8, `[DONE]`)
- `vkdg-connections`: `ConnectionCatalog`, RAII `ConnectionGuard`, `CredentialManager`, `eligible_for_operation` fail-closed
- `vkdg-ingress-anthropic`: decode Anthropic Messages → Operation, encode response/stream
- `vkdg-provider-anthropic`: `AnthropicAdapter` implementing `ProviderAdapter`
- Complete pipeline: admission → routing → connection reserve → credential → upstream → passthrough
- Backpressure via `Body::from_stream` lazy; `DecisionRecord` emitted per attempt
- `vkdg-observe`: tracing + OTLP + `DecisionRecordExporter`

**Phase C: Interoperability**
- `vkdg-ingress-openai`: decode Chat Completions → Operation, encode SSE OpenAI wire
- `vkdg-provider-openai`: `OpenAIAdapter` with upstream SSE parsing (parallel tool calls included)
- Fallback 429: retry with second candidate before `committed`; `DecisionRecord` per attempt
- Routes: `/v1/messages` (Anthropic), `/v1/chat/completions` (OpenAI), `/v1/images/generations`

**Phase D: Modalities and extensions**
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
- 21 crates with isolated responsibilities
- `CapabilitySet` in `vkdg-core` (no inverted coupling)
- `vkdg-http` split into domain modules: admission, frontdoor, server, app_state, pipeline, provider, sse, upstream, external_service
- `DecisionRecord` without secrets; `TokenState::Debug` omits token

### Tests
- 250 tests, 0 failing, 0 clippy warnings
- Each test documents a plausible defect it defeats
- SSE parser: fragmentation at any byte, partial UTF-8, multiple events
- Pipeline: admission before routing, RAII releases on all exit paths, fallback only before committed
- Config: cross-reference validation, hot-reload rejects invalid config

### Known gaps (Phase E targets)
- WASM `call_prepare` is a stub: full Component Model bindgen in Phase E
- `ModelSummarize` compression is a stub: requires metered internal call
- `video.generate` polling/webhook not implemented: returns 202 Accepted
- OAuth2 credential refresh is a stub: singleflight + encrypted vault in Phase E
- Admin API (`/admin/v1/`) and SvelteKit console pending: `VKDG-frontend-day0.md`

### Breaking changes
- None (first release)

[0.1.0-rc1]: https://github.com/vkdprojects/vkdg/releases/tag/v0.1.0-rc1
