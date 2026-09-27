# Changelog

All notable changes to VKDG are documented here.
Format: [keepachangelog.com](https://keepachangelog.com/en/1.1.0/).
Versioning: [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Security
- Console sign-in: after the first sign-in with the bootstrap token, the console asks for a password (at least 12 characters), stored as an argon2id hash in `admin.password` next to `accounts.db` (`0600`; override with `VKDG_ADMIN_PASSWORD_FILE`). From then on the token is refused, even after a restart, and the password signs in any number of times. Before, the token worked once, so signing out locked the admin out until a restart. Recover with `vkdg admin set-password` on the host. Failed sign-ins are throttled per client address (5 per 15 min, resolved through `VKDG_TRUSTED_PROXIES` like the data plane) and globally (100 per 15 min), with 429 and `retry-after`. Sessions expire after 12 h unused; a restart still signs everyone out
- **Breaking:** `/v1/messages`, `/v1/chat/completions` and `/v1/images/generations` now require a client API key (`x-api-key` or `Authorization: Bearer`), on every bind address including loopback: a loopback listener behind a reverse proxy is still public. Requests without a valid key get 401 in the client's wire format. `VKDG_DATA_AUTH=off` opts out explicitly and logs a warning at startup
- API keys are persisted in `keys.db` (next to `accounts.db`, `0600`, override with `VKDG_KEYS_DB`). Only a SHA-256 hash and a display prefix (`vkdg_1a2b3c4d`) are stored. A key revoked from another process stops working within 5 s
- The admin API's plaintext in-memory key store is gone: `/admin/v1/keys` manages the same keys `/v1/*` checks

- Client IP comes from the TCP socket. `X-Forwarded-For` is believed only from peers in `VKDG_TRUSTED_PROXIES` (walked right to left); before, any client could claim any address and pass an IP allowlist
- IP allowlist/blocklist entries match by bit mask for IPv4 and IPv6 (`10.16.0.0/12` no longer admits `10.32.0.1`). **Breaking:** the dotted-prefix form (`192.168.1.`) is gone; use CIDR. An unknown client IP now fails a non-empty allowlist instead of skipping it

- **Breaking:** `vkdg serve --config <file>` exits with the validation error when the file is invalid. It used to log a warning and serve an env-derived gateway (different routes, possibly an `ANTHROPIC_API_KEY` passthrough) instead of the one the operator asked for

### Added
- Browser end-to-end tests (`just e2e`, Playwright 1.62, `apps/console/e2e/`) run the real binary with the console embedded and a fresh data directory. They cover sign-in, the full API key lifecycle checked on `/v1/*` after each step, invalid limits, connecting two accounts on one provider, deleting one, and the provider list
- `GET /admin/v1/providers/oauth` lists the providers on this gateway that support interactive login, OAuth plugins included; the console's Connect-account dialog uses it instead of a hardcoded list
- Test-only `fake-oauth` provider for end-to-end tests of the login flows, registered only with `VKDG_E2E_FAKE_OAUTH=1` in debug builds (ignored with an error in release builds). Its device code approves on the first poll; a refresh token containing `revoked` behaves like a revoked login
- Console: Connections show live state as text (healthy, degraded, circuit open, cooling down), in-flight vs capacity and cooldown details, refreshed every 5 s while visible. Requests page refreshes every 3 s (pausable), filters by status and opens a detail drawer with the routing decision. Keys page can edit, regenerate (new secret shown once), disable and enable keys
- Request history and `DecisionRecord` carry the real routing decision: the route that matched (`auto` when none did), attempts, and the candidates left out with why. They used to record no decision and the connection id as the route
- Request history is persisted in `requests.db` next to `accounts.db` (newest 10 000 kept) and records the client key id. It used to live in memory, capped at 1 000, and vanished on every restart. Metadata only: no prompts, responses or credentials
- Key lifecycle: `PATCH /admin/v1/keys/{id}` edits name, scopes, expiry, allowed models and IPs, budget and rpm (absent field = unchanged, `null` clears a limit); `POST .../regenerate` issues a new secret with the same id, policy and usage and kills the old one at once; `POST .../disable` and `.../enable` switch a key off reversibly (status `disabled`), unlike revoke. Policy changes apply to the next request, cached lookups included
- `GET /v1/models` lists the concrete model ids this gateway routes (from routes and connections, globs skipped), filtered by the calling key's `allowed_models`, in OpenAI shape or Anthropic shape when `anthropic-version` is sent. Requires a key like the rest of `/v1/*`
- Route auth hooks: `hooks: { auth: [plugin-name] }` on a route runs installed WASM auth plugins after the client's vkdg key is accepted, before any dispatch path (fusion and prompt chains included). A plugin can only narrow access: a denial, an error, or a missing plugin returns 403. Plugins receive key id, tenant, client address, model and route, never headers or keys. A route naming a plugin that is not installed stops startup and makes a reload keep the current config
- **Breaking:** the unused `PluginHooks` fields `pre_auth`, `rate_limit`, `pre_dispatch`, `on_event` and `on_finish` are removed; they were never reachable from config nor executed
- Console: Accounts page (provider, label, expiry, `needs_login` with the upstream reason, delete, reconnect) and a Connect-account dialog for device-code and PKCE logins (code with copy and countdown, polling at the provider's interval, paste-back for PKCE, retry on expiry). Keys page: expiry, allowed models and IPs, monthly token budget and requests per minute on create; status, restrictions and this month's usage against the budget in the list
- `GET /admin/v1/connections` reports each connection's live state from the data plane (`healthy`, `degraded`, `circuit_open`, `cooldown` with `cooldown_until` and `failure_count`), in-flight requests and `max_concurrent`. It used to report `healthy` and 0 for everything
- `GET /admin/v1/keys` returns `monthly_token_limit`, `requests_per_minute` and `usage_this_month`
- Installed WASM provider plugins are loaded at startup and registered next to the built-ins. A plugin may only take a free provider id: one claiming `anthropic`, `kiro` or any other registered id is refused with a warning, because an adapter receives the credentials of every connection that names it. A plugin that fails to load, including one with an unreadable `manifest.yaml`, is logged by directory name and skipped; the other plugins and the gateway still start
- `GET /admin/v1/plugins` and `vkdg plugin list` show installed plugins even when another plugin's `manifest.yaml` is unreadable; the broken directory is listed under `broken` with its error instead of failing the whole listing
- The plugins directory defaults to `plugins/` next to `accounts.db` (the data volume), not `~/.config` or a path inside the container image
- Per-key usage, budgets and rate limits. Tokens are counted from the response the client receives (SSE or JSON, Anthropic or OpenAI dialect), at end of stream or on client disconnect, and stored per key per month in `keys.db` with an atomic upsert. `monthly_token_limit` refuses a key over budget before routing (429, `insufficient_quota`); `requests_per_minute` is an in-memory per-key window (429, `rate_limit_exceeded`). Both 429s carry `retry-after` (seconds until the window or the month resets) and a `rate_limit_error` type. The budget is a soft cap: it is checked before each request, so streams already in flight can go past it by what they consume. CLI `--monthly-tokens`/`--rpm`, admin `monthly_token_limit`/`requests_per_minute`
- Per-key limits: expiry (`expires_at`), allowed model patterns (`allowed_models`, same `*`/`?` syntax as routes) and allowed client ranges (`allowed_ips`, CIDR, v4/v6). Expired keys get 401; a model or address outside the key's list gets 403. Set them with `vkdg keys create --model 'claude-*' --ip 10.0.0.0/8 --expires-in-days 30` or `POST /admin/v1/keys`. Existing `keys.db` files gain the columns on open
- `vkdg keys create|list|revoke`. `create` prints only the raw key on stdout, so `$(vkdg keys create ci)` works
- `/admin/v1/keys` takes `scopes` (`data_inference`, `data_image`) instead of `role`, and returns `prefix`, `status`, `last_used_at`, `revoked_at`. The console Keys page shows them and keeps revoked keys listed

### Fixed
- Reconnect on an account (`account_id` on `/oauth/{provider}/start` and `/import`) replaces its tokens and clears `needs_login` in place, keeping the id, and evicts the cached credential. It used to create a second account, so connections referencing the revoked one stayed broken
- `limits.ip_allowlist` / `limits.ip_blocklist` were parsed and never enforced. They now apply to every `/v1/*` request, follow hot reload, and are validated at load: an invalid entry names its position (`limits.ip_allowlist[1]`) and a reload with it keeps the current config. `VKDG_TRUSTED_PROXIES` is validated at startup the same way
- Streams re-encoded for the client (Kiro and any decoded provider) now send the full final usage (`input_tokens`, cache fields, `output_tokens`) in `message_delta`. Before, only `output_tokens` reached the wire, so clients and per-key budgets saw 0 input tokens for every Kiro request
- IPv4 clients of a dual-stack listener (`::ffff:a.b.c.d`) are matched as IPv4 by IP lists, per-key lists and `VKDG_TRUSTED_PROXIES`; before, they slipped past blocklists and failed allowlists
- A corrupt policy column in `keys.db` (`expires_at`, `allowed_models`, `allowed_ips`) makes the key unusable instead of unrestricted
- **Behavior change:** route `match_models`, connection `models` and combo patterns share one matcher: `*` anywhere and `?` (one character) now work in routes and connections too, so a literal `?` in a model pattern now matches any single character. The combo matcher no longer backtracks exponentially on patterns like `*a*a*a*b`
- **Breaking:** route strategies `lowest_latency` and `power_of_two_choices` are implemented (p50 latency per connection, measured by the gateway). `weighted` and `last_known_good` were accepted and silently routed round-robin; config now rejects them
- A route that matched but whose targets were all excluded or cooling down fell through to "any connection serving the model", sending its traffic outside its `targets`. Only a model no route matches is auto-routed now; a matched route with no usable target returns 502
- Config hot reload reaches the data plane. Before, a valid edit to the config file bumped the admin revision but requests kept routing with the startup routes and connections. Routes and connection configs now swap on each valid reload; in-flight counters and health survive it, so capacity accounting never resets. One save triggers one reload (notify events are debounced). The watcher follows the config's directory, so atomic saves (editors, `sed -i`, ConfigMap symlink swaps) keep reloading on Linux; a watcher that cannot start logs an error instead of panicking
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
