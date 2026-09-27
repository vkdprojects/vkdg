# Changelog

All notable changes to VKDG are documented here.
Format: [keepachangelog.com](https://keepachangelog.com/en/1.1.0/).
Versioning: [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Security
- **Breaking:** `/v1/messages`, `/v1/chat/completions` and `/v1/images/generations` now require a client API key (`x-api-key` or `Authorization: Bearer`), on every bind address including loopback: a loopback listener behind a reverse proxy is still public. Requests without a valid key get 401 in the client's wire format. `VKDG_DATA_AUTH=off` opts out explicitly and logs a warning at startup
- API keys are persisted in `keys.db` (next to `accounts.db`, `0600`, override with `VKDG_KEYS_DB`). Only a SHA-256 hash and a display prefix (`vkdg_1a2b3c4d`) are stored. A key revoked from another process stops working within 5 s
- The admin API's plaintext in-memory key store is gone: `/admin/v1/keys` manages the same keys `/v1/*` checks

- Client IP comes from the TCP socket. `X-Forwarded-For` is believed only from peers in `VKDG_TRUSTED_PROXIES` (walked right to left); before, any client could claim any address and pass an IP allowlist
- IP allowlist/blocklist entries match by bit mask for IPv4 and IPv6 (`10.16.0.0/12` no longer admits `10.32.0.1`). **Breaking:** the dotted-prefix form (`192.168.1.`) is gone; use CIDR. An unknown client IP now fails a non-empty allowlist instead of skipping it

- **Breaking:** `vkdg serve --config <file>` exits with the validation error when the file is invalid. It used to log a warning and serve an env-derived gateway (different routes, possibly an `ANTHROPIC_API_KEY` passthrough) instead of the one the operator asked for

### Added
- Per-key limits: expiry (`expires_at`), allowed model patterns (`allowed_models`, same `*`/`?` syntax as routes) and allowed client ranges (`allowed_ips`, CIDR, v4/v6). Expired keys get 401; a model or address outside the key's list gets 403. Set them with `vkdg keys create --model 'claude-*' --ip 10.0.0.0/8 --expires-in-days 30` or `POST /admin/v1/keys`. Existing `keys.db` files gain the columns on open
- `vkdg keys create|list|revoke`. `create` prints only the raw key on stdout, so `$(vkdg keys create ci)` works
- `/admin/v1/keys` takes `scopes` (`data_inference`, `data_image`) instead of `role`, and returns `prefix`, `status`, `last_used_at`, `revoked_at`. The console Keys page shows them and keeps revoked keys listed

### Fixed
- `limits.ip_allowlist` / `limits.ip_blocklist` were parsed and never enforced. They now apply to every `/v1/*` request, follow hot reload, and are validated at load: an invalid entry names its position (`limits.ip_allowlist[1]`) and a reload with it keeps the current config. `VKDG_TRUSTED_PROXIES` is validated at startup the same way
- IPv4 clients of a dual-stack listener (`::ffff:a.b.c.d`) are matched as IPv4 by IP lists, per-key lists and `VKDG_TRUSTED_PROXIES`; before, they slipped past blocklists and failed allowlists
- A corrupt policy column in `keys.db` (`expires_at`, `allowed_models`, `allowed_ips`) makes the key unusable instead of unrestricted
- **Behavior change:** route `match_models`, connection `models` and combo patterns share one matcher: `*` anywhere and `?` (one character) now work in routes and connections too, so a literal `?` in a model pattern now matches any single character. The combo matcher no longer backtracks exponentially on patterns like `*a*a*a*b`
- Config hot reload reaches the data plane. Before, a valid edit to the config file bumped the admin revision but requests kept routing with the startup routes and connections. Routes and connection configs now swap on each valid reload; in-flight counters and health survive it, so capacity accounting never resets. One save triggers one reload (notify events are debounced)
- A refresh token rejected by the upstream (401, `invalid_grant`, `InvalidGrantException`, `ExpiredTokenException`) now parks the account instead of being retried on every request. New typed error `VkdgError::CredentialRevoked` / `ProviderError::CredentialRevoked`; transient failures (5xx, throttling) still retry
- Refresh failures log the upstream status and message (`401: Bad credentials`) instead of a bare `authorization_failed`
- `/admin/v1/accounts` reports `status: active | needs_login` and `revoked_reason`; the state is stored in `accounts.db` (column added automatically on existing stores)

## [0.1.0-rc1]: 2026-09-26

### Added

**Provider accounts and OAuth login (generic, plugin-driven)**
- `ProviderAdapter::oauth()` hook; `OAuthProvider` gains `login_methods`, device-code (`start_device_login`/`poll_device_login`), PKCE (`start_pkce_login`/`finish_pkce_login`) and `import_token`, all opt-in
- `AccountStore` (SQLite, `0600`) and `auth: { type: account, account: <id> }`; `CredentialManager` refreshes 5 min ahead via the plugin, one refresh per account, and persists the result
- **Breaking (SDK):** `ProviderAdapter::prepare` takes `&Credential { token, extra }` instead of `token: &str`
- CLI: `vkdg login <provider> [--method] [--opt k=v] [--list-methods]`, `vkdg accounts list|remove`
- Admin API: `GET /admin/v1/providers/{id}/login-methods`, `POST /admin/v1/oauth/{provider}/{start,poll,import}`, `GET/DELETE /admin/v1/accounts`

**Plugin ABI made real (2026-09-27)**
- **Fixed:** `wit/` never parsed — `stream` is a WIT keyword, `float32` was renamed `f32`, and the world is `cache-backend`. No plugin could have been built against the published contract; a test now parses the package in CI
- `wit/provider.wit` grows from 5 to 15 functions so a community plugin can match the first-party Kiro adapter: interactive login, per-account credential data, plugin-chosen URL, raw-bytes stream decode, `finish-stream`, runtime model discovery. Verified by compiling Kiro as a `wit-bindgen` guest against the WIT alone
- WASM components now execute rather than only validate: per-call fuel, memory ceiling from the manifest, a fresh `Store` per call, and no host imports beyond WASI resolved against a context that grants nothing
- All 5 roles run as `.wasm`, each with the failure policy its job demands — provider fails the request, compressor is skipped, router falls back to gateway order, cache degrades to a miss, auth denies
- `RegistryManifest` and `RegistryIndex`: checksum mandatory for remote wasm, kebab-case names, exactly one install source; `PluginStore` verifies and compiles before writing
- `vkdg plugin install|list|remove|search` and `/admin/v1/plugins` now work, plus a console plugins screen

**Streaming (2026-09-27)**
- **Fixed:** `ConversationStreamDecoder::finish()` was never called, so a provider that sends no stop event produced a stream with no terminal events
- **Fixed:** the hand-rolled dialect encoders emitted an invalid Anthropic stream (`message_start` as `data: {}`, every block at index 0) and silently dropped tool calls. `StreamEncoder` in `vkdg-operations` is now the single encoding path
- `ConversationEvent` gains `ReasoningDelta`, `ToolCallEnd` and cache token counts

**Kiro (2026-09-27)**
- **Fixed:** the EventStream parser had been fitted to a synthetic fixture — `total_len` excluded the prelude, string headers ignored their `u16` length, and neither CRC was checked. A real Amazon Q stream would desync
- **Fixed:** `reasoningContentEvent` reads `text`, not `content`; `contextUsageEvent` is snake_case, so reading camelCase silently zeroed usage
- Endpoint follows the credential: OAuth accounts use `runtime.{region}.kiro.dev`, `ksk_` API keys use `q.{region}.amazonaws.com` and must omit `profileArn` (AWS answers 403). Kiro's own docs mark `q.*` legacy and slated for deprecation
- Five login methods (builder-id, idc, social, import, api-key) with three refresh paths; prompt caching via `cachePoint`; `x-amzn-codewhisperer-optout` on every request

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

Providers:
- Kiro / Amazon Q provider (`kiro`): API key auth via `KIRO_API_KEY` or device code OAuth (AWS SSO OIDC), fixed `q.us-east-1.amazonaws.com` endpoint
- Kiro exposed end to end: `vkdg setup` wizard, console connection and setup pages, registry manifest, `config.example.yaml`
- Kiro model families: `claude-*`, `gpt-5.6-*`, `minimax-*`, `deepseek-*`, `glm-*`, `qwen3-*`, `auto` (20 model ids)

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
