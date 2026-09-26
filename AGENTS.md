# VKDG — agent entry guide

Product: AI gateway in Rust. Receives calls via Anthropic Messages, OpenAI Chat Completions, and OpenAI Images; selects a provider connection by capability, quota, health, and policy; streams responses with backpressure; manages async jobs for video/media. Not an agent tool runtime.

## Before editing

1. Read `VKDG-architecture-v0.md` and `VKDG-frontend-day0.md` — architecture reference and SvelteKit console contract. Code and committed ADRs take precedence over document hypotheses.
2. For behavioral changes: read `.agents/skills/vkdg-test-first/SKILL.md` before writing a line of code. Pick the domain skill from the table below and read it too.
3. Find the module and test that own the behavior you are changing. Never create a second execution path without first verifying the first one cannot be extended.
4. Verify that crates, commands, and files you assume exist actually exist. Inspect `Cargo.toml` and the file tree before acting on assumptions.

| Task | Skill |
| --- | --- |
| Any behavioral change: define RED/GREEN/REFACTOR cycle | `.agents/skills/vkdg-test-first/SKILL.md` (always combined with a domain skill) |
| Implement, fix, refactor, or review Rust code; organize modules | `.agents/skills/vkdg-change/SKILL.md` |
| Native API, protocol, SSE, converter, or capability matrix | `.agents/skills/vkdg-protocol/SKILL.md` |
| Plugin ABI, hooks, WIT interface, SDK, or install lifecycle | `.agents/skills/vkdg-plugin/SKILL.md` |
| Concurrency, OAuth, storage, routing, resilience, or performance | `.agents/skills/vkdg-operate/SKILL.md` |
| Rust DX: module layout, constructors, errors, naming, test patterns, toolchain | `.agents/skills/vkdg-dx/SKILL.md` |
| READMEs, docs, changelogs, community files, or any user-facing copy | `.agents/skills/vkdg-copy/SKILL.md` |
| SvelteKit console, admin API, BFF, or web authentication | `VKDG-frontend-day0.md` + `.agents/skills/vkdg-test-first/SKILL.md` |

---

## Commands

```bash
# Build
cargo build --release -p vkdg

# Check all crates
cargo check --workspace

# Full test suite (all targets: lib, bin, tests, benches)
cargo test --workspace --tests

# Clippy — same flags as CI
cargo clippy --workspace --all-targets --locked -- -D warnings

# Unused dependency check
cargo machete

# Spell check
typos .

# Format check
cargo fmt --all --check

# Apply formatting
cargo fmt --all

# Run locally (no credentials = 501 on inference endpoints)
cargo run -p vkdg -- serve

# Run with a real Anthropic key
ANTHROPIC_API_KEY=sk-ant-... cargo run -p vkdg -- serve

# Run with a config file
cargo run -p vkdg -- serve --config vkdg.toml

# Smoke tests — no external credentials needed
cargo test -p conformance smoke -- --nocapture

# Diagnostics
cargo run -p vkdg -- doctor
cargo run -p vkdg -- config check <file.yaml>
cargo run -p vkdg -- config explain --model claude-3-5-haiku-20241022
cargo run -p vkdg -- request explain <request-uuid>
cargo run -p vkdg -- replay spec/replay/health_check.yaml
```

### Git hooks (lefthook)

```bash
# Install once per clone
brew install lefthook && lefthook install

# pre-commit: typos + rustfmt check (on staged files, ~0.3s)
# pre-push:   cargo clippy --all-targets -D warnings + cargo machete (~5-30s warm cache)
```

---

## Crate map

| Crate | Responsibility |
| --- | --- |
| `vkdg-core` | Domain types, 12-state attempt machine, `DecisionRecord`, `VkdgError`, `Capability`. No HTTP/DB/WASM. |
| `vkdg-operations` | Operation contracts per family: Conversation, Image, Audio, Video, Embedding |
| `vkdg-http` | HTTP server, frontdoor, admission semaphore, SSE parser, upstream client, pipeline orchestration |
| `vkdg-routing` | `Router`, routing strategies, `ScoredStrategy`, `RoutingHints`, eligibility filter |
| `vkdg-connections` | `ConnectionCatalog`, `ConnectionGuard` RAII, `CredentialManager` (OAuth2 + API key), `SessionRegistry`, `QuotaTracker`, `LatencyTracker` |
| `vkdg-config` | `ConfigSnapshot` versioned config, hot-reload via `notify`, cross-reference validation |
| `vkdg-ingress-anthropic` | Decode Anthropic Messages wire format → Operation; encode response/stream |
| `vkdg-ingress-openai` | Decode OpenAI Chat Completions and Images wire formats → Operation |
| `vkdg-provider-anthropic` | `AnthropicAdapter` implementing `ProviderAdapter` trait |
| `vkdg-provider-openai` | `OpenAIAdapter` with upstream SSE parse, parallel tool calls |
| `vkdg-observe` | Tracing setup, OTLP export, `DecisionRecordExporter` |
| `vkdg-plugin-host` | WASM plugin registry, manifest, `install_wasm`/`uninstall`, Wasmtime + WIT |
| `vkdg-storage` | `JobStore` port, `InMemoryJobStore` |
| `vkdg-jobs` | `JobManager`, `SqliteJobStore` (WAL), job state transitions, webhook dispatch |
| `vkdg-artifacts` | `InMemoryArtifactStore` with per-tenant ACL, TTL, purge GC |
| `vkdg-cache` | `CacheBackend` trait, `SqliteExactCache` (zero-infra default), Redis backend (feature-gated) |
| `vkdg-combos` | Named routing plans with own strategy, compression, cache, and budget policies |
| `vkdg-policy-compress` | Compression: `TruncateCompressor`, `CavemanCompressor`, `RtkCompressor`, `StackedCompressor` |
| `vkdg-memory` | Conversational memory: `MemoryStore`, `extract_facts`, `inject_memories` |
| `vkdg-eval` | `EvalScorer`: latency and content quality scoring per response |
| `vkdg-admin` | Admin API (`/admin/v1/*`): session, keys, connections, routes, requests |
| `vkdg-cli` | `vkdg doctor`, `config check`, `config explain`, `request explain`, `replay` |
| `bin/vkdg` | CLI entrypoint — wires all crates into the binary |

### Module structure (key splits)

Large modules are split into focused submodules:

```
vkdg-http/src/pipeline/
  mod.rs      — re-exports only
  entry.rs    — run_conversation_pipeline, emit_decision_record
  phases.rs   — resolve_combo_and_session, prepare_operation, post_response_accounting
  inner.rs    — run_pipeline_inner (12 sequential steps)
  helpers.rs  — is_multiturn, has_tool_calls, error_response, sse helpers

vkdg-connections/src/
  lib.rs           — re-exports
  connection.rs    — ProviderKind, ConnectionConfig, ConnectionGuard, Connection
  catalog.rs       — ConnectionCatalog
  credentials.rs   — CredentialManager, OAuth2 flow
  session.rs, quota.rs, latency.rs

vkdg-routing/src/
  lib.rs       — re-exports
  types.rs     — RouteConfig, RoutingHints, EligibilityFilter
  strategy.rs  — Strategy trait, RoundRobinStrategy, FallbackChainStrategy
  scored.rs    — ScoredStrategy
  router.rs    — Router
  scorer.rs    — ScoringWeights, CandidateSignals, rank_candidates

vkdg-ingress-anthropic/src/
  lib.rs     — handle_messages handler
  wire.rs    — Anthropic wire types (private)
  decode.rs  — decode_request
  encode.rs  — encode_event, vkdg_error_to_anthropic_response
```

---

## Tests

| Package | Type | What it covers |
| --- | --- | --- |
| `tests/conformance` | In-process integration | Admission, routing, ingress, state machine, SSE, pipeline end-to-end, fake upstream |
| `crates/vkdg-http` | Unit + integration | SSE parser, pipeline phases, admission, dedup, external service client |
| `crates/vkdg-core` | Unit | `PipelineCtx` state machine, `VkdgError`, `CapabilitySet` |
| `crates/vkdg-connections` | Unit | ConnectionCatalog, CredentialManager, SessionRegistry, QuotaTracker, LatencyTracker |
| `crates/vkdg-routing` | Unit | Router, all strategies, ScoredStrategy with live quota hints |
| `crates/vkdg-cache` | Unit | SqliteExactCache: store/lookup, expiry, tenant isolation |
| `crates/vkdg-combos` | Unit | ComboResolver: exact match, glob patterns |
| `crates/vkdg-admin` | Unit + integration | All `/admin/v1/*` endpoints, session lifecycle |
| `apps/console` | Unit (vitest + msw) | Typed admin client: 19 BFF tests against mock HTTP server |

Run all tests: `cargo test --workspace --tests`

Run smoke tests only (fake upstream, no credentials): `cargo test -p conformance smoke -- --nocapture`

---

## Spec

`spec/scenarios/*.yaml` — observable contracts per phase. Each maps to tests in `tests/conformance/`.

`spec/replay/*.yaml` — fixture files for `vkdg replay`. Send a recorded request to a running gateway and optionally assert status and body.

---

## WIT interfaces

Plugin contracts live in `wit/`:

| File | Interface |
| --- | --- |
| `types.wit` | Shared domain types: request, response, message, candidate, signals |
| `router.wit` | Routing strategy plugin |
| `compressor.wit` | Context compression plugin |
| `provider.wit` | Provider adapter plugin |
| `cache.wit` | Cache backend plugin |
| `auth.wit` | Auth and rate-limit plugin |
| `world.wit` | Multi-role plugin world |

WASM Component Model bindgen (Wasmtime) is stubbed in `vkdg-plugin-host`. Phase E wires it.

---

## Architecture decisions

`docs/adr/` contains:
- `ADR-001` — Hyper/Tower over Pingora
- `ADR-002` — CapabilitySet in vkdg-core
- `ADR-003` — WIT/WASM as plugin ABI

---

## Invariants to preserve

These must hold in every change:

1. `run_pipeline_inner` never retries after `ctx.mark_committed()` — `can_retry()` guards this.
2. `ConnectionGuard` drops exactly once on all exit paths (success, error, cancellation).
3. `DecisionRecord` never contains a token, credential, prompt, or response payload.
4. `TokenState::Debug` omits the access_token field.
5. Cache bypass is enforced in core for multi-turn conversations and requests with tool calls — plugins cannot override this.
6. A 429 on one connection does not affect other connections on the same provider.
7. Config reload only activates after full validation — invalid reload keeps the current snapshot.
