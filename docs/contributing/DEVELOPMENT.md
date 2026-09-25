# Development Guide

## Prerequisites

- **Rust stable** (toolchain managed via `rustup`)
- No API keys needed to develop, the fake upstream covers almost the entire test suite

```bash
rustup update stable
```

## Day-to-day commands

```bash
# Compile the entire workspace
cargo check --workspace

# Run all tests
cargo test --workspace

# Run tests for a specific crate
cargo test -p vkdg-core
cargo test -p vkdg-http
cargo test -p conformance

# Start the gateway locally (no credential → 501 on /v1/messages)
cargo run -p vkdg -- serve

# Start with a real Anthropic key
ANTHROPIC_API_KEY=sk-ant-... cargo run -p vkdg -- serve

# Environment diagnostics
cargo run -p vkdg -- doctor

# Validate a config file
cargo run -p vkdg -- config check <file.yaml>
```

## Smoke tests without credentials

The `tests/conformance` crate includes in-process smoke tests that use `FakeUpstream`:

```bash
cargo test -p conformance smoke -- --nocapture
```

These tests start a fake upstream in memory, make no real network calls, and require no environment variables.

## How to add a provider (end to end)

1. **Declare the `CapabilitySet`** for the new provider in `ConnectionConfig` (the `capabilities` field in `vkdg-connections`).

2. **Write the ingress adapter** (if it is a new protocol) as a new crate `vkdg-ingress-<protocol>`:
   - `decode_request(body: Bytes) -> Result<(String, Operation), VkdgError>`
   - `encode_event(event: &ConversationEvent) -> String`
   - `vkdg_error_to_<protocol>_response(err: VkdgError) -> Response`

3. **Add a provider adapter** (Phase C): crate `vkdg-provider-<name>` implementing the `ProviderAdapter` trait (to be stabilized in Phase C).

4. **Write scenarios** in `spec/scenarios/` before implementing, the YAML file documents the expected behavior.

5. **Add conformance tests** in `tests/conformance/` covering at minimum: valid decode, invalid decode, streaming, upstream error, capability mismatch.

6. **Register in `ConnectionCatalog`** via `ConnectionConfig { provider: ProviderKind::Custom { base_url }, ... }`.

## Conventions

- **Test-first**: write the RED test before any implementation. See `.agents/skills/vkdg-test-first/SKILL.md`.
- **Architecture decisions**: record in `docs/adr/ADR-NNN-<slug>.md` before implementing design changes.
- **No `unwrap` on external paths**: use `?` and `VkdgError` in any code that touches user input, network, or files.
- **No automatic `Debug` on types with secrets**: implement `Debug` manually (see `TokenState` in `vkdg-connections`).
- **`DecisionRecord` never carries message payload or access token**.
- **New crates**: add to `[workspace.members]` in the root `Cargo.toml` and use `[workspace.dependencies]` for shared versions.

## Phase backlog

| Phase | Status | Exit criterion |
|---|---|---|
| A: Executable specification | ✅ Done | Conversation contracts, states, failure corpus, decisions in ADR |
| B: Data vertical | ✅ Done | Anthropic endpoint, streaming, cancellation, fake upstream, 72 tests |
| C: Interoperability | ✅ Done | Second provider, second ingress protocol, per-event translation, fallback combo |
| D: Modalities and extensions | ✅ Done | Image, video job, WASM plugin, atomic config |
| E: Scale / release | 🔲 Pending | Benchmarks, SDK documentation, release candidate |
