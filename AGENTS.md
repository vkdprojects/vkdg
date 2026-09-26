# VKDG: agent entry guide

Product: AI gateway in Rust. Receives calls via native VKDG API and compatible protocols; selects connection/provider by capability and policy; transforms only when necessary; streams responses and manages media jobs. Not an agent tool runtime.

## Before editing

1. Read `VKDG-architecture-v0.md` and `VKDG-frontend-day0.md` as reference proposals; the SvelteKit panel in a separate process is a day-0 requirement. Then check the code and ADRs that are actually present. Code and approved contracts take precedence over an old document hypothesis.
2. For any behavioral change, read `.agents/skills/vkdg-test-first/SKILL.md` before writing code. Also select the domain skill below; read its full `SKILL.md`.
3. Find the test/fixture and the module that owns the behavior. Never create another execution path without checking the existing one.
4. Do not assume that crates, `xtask`, commands, or CI described in the architecture already exist; inspect `Cargo.toml` and the repository before executing.

| Task | Skill |
| --- | --- |
| Implement or fix any behavior; define RED/GREEN/REFACTOR and boundaries | `.agents/skills/vkdg-test-first/SKILL.md` (always combined with the domain skill) |
| Implement, fix, refactor, review Rust code or organize modules | `.agents/skills/vkdg-change/SKILL.md` |
| Create/change native API, compatible protocol, converter, SSE, or capability matrix | `.agents/skills/vkdg-protocol/SKILL.md` |
| Design/implement plugin, ABI, hooks, SDK, and install lifecycle | `.agents/skills/vkdg-plugin/SKILL.md` |
| Change concurrency, OAuth, storage, routing, resilience, debug, or performance | `.agents/skills/vkdg-operate/SKILL.md` |
| Rust DX and code quality — module organization, constructors, errors, naming, test patterns | `.agents/skills/vkdg-dx/SKILL.md` |
| Write docs, READMEs, community files, release notes, or any user-facing copy | `.agents/skills/vkdg-copy/SKILL.md` |
| Develop SvelteKit panel, admin API, web authentication, or BFF ↔ Rust contract | `VKDG-frontend-day0.md` and `.agents/skills/vkdg-test-first/SKILL.md` |

Combine skills when a change crosses boundaries. For behavioral changes: record the observable contract, run the RED test for the correct reason, implement in the owning module, reach GREEN, refactor, and save evidence of the commands. Verify with a fake upstream and report limits. Prefer small, complete changes; avoid placeholders that promise compatibility not yet proven.

Initial decisions already made: Claude Code/Messages is the first real client; Codex/Responses comes next. Start HTTP with Hyper/Tower (Axum can serve the routes); compare Pingora only if real profiles justify it. These choices do not authorize shortcuts in conformance or benchmarks.

## Commands

```bash
# check
cargo check --workspace
cargo test --workspace

# run
cargo run -p vkdg -- serve --listen 0.0.0.0:8080

# local smoke (no real credential)
cargo test -p conformance smoke -- --nocapture

# diagnostics
cargo run -p vkdg -- doctor
cargo run -p vkdg -- config check <file.yaml>
```

## Crate map

| Crate | Responsibility |
| --- | --- |
| vkdg-core | Fundamental types, state machine, DecisionRecord, errors. No HTTP/DB/WASM. |
| vkdg-operations | Operation contracts by family: Conversation, Embedding, Image, Audio, Video |
| vkdg-http | HTTP server, frontdoor, admission, SSE parser, upstream client, pipeline runner |
| vkdg-routing | Routing strategies, connection eligibility |
| vkdg-connections | Account catalog, RAII ConnectionGuard, CredentialManager |
| vkdg-ingress-anthropic | Decode/encode Anthropic Messages protocol |
| vkdg-observe | Tracing, OTLP, DecisionRecord exporter |
| vkdg-plugin-host | Plugin registry, manifest (Wasmtime in Phase D) |
| vkdg-storage | Storage ports: JobStore, InMemoryJobStore |
| vkdg-cli | Doctor and config check commands |
| bin/vkdg | CLI entrypoint |

## Tests

| Package | Type | Focus |
| --- | --- | --- |
| tests/conformance | In-process integration | Admission, routing, ingress, state machine, SSE, capabilities, end-to-end pipeline contracts |
| crates/vkdg-http | Unit + integration | SSE parser, pipeline, admission |
| crates/vkdg-core | Unit | PipelineCtx state machine |

## Spec

`spec/scenarios/*.yaml`: observable contracts per phase. Each file maps to tests in `tests/conformance/`.
