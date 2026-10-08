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

## Shipping a dev image

```bash
just ship-dev <ssh-host>            # deploy HEAD
just ship-dev <ssh-host> --plan     # show where it would build, and why
just rollback-dev <ssh-host>        # restore the previous image
```

`ship-dev` decides where to compile:

| Your machine | Build runs | Typical time |
|---|---|---|
| Has `cargo-zigbuild`, `zig`, Docker; ≥ 8 CPUs; ≥ 16 GiB RAM | Locally: cross-compile to x86 musl, package with `--target prebuilt`, stream over SSH | ~1–2 min |
| Anything else | On the host: source synced, `deploy/Dockerfile` built there under `nice -n 19` | several minutes, cached afterwards |

If the local build fails it falls back to the host. Force a mode with
`VKDG_BUILD=local|remote`. Either way the host performs the same tag swap,
restart, health check, and cleanup (old images and build cache are pruned so the
disk stays bounded). Uncommitted changes are refused: the image tag
`dev-<sha>` always names a real commit.

Dev deploys compile with the `ship` profile (no LTO, 16 codegen units), which is
several times faster than `release`. Set `VKDG_PROFILE=release` for production
parity.

To enable the fast path once:

```bash
brew install zig                    # or your platform's zig package
cargo install cargo-zigbuild
rustup target add x86_64-unknown-linux-musl
```

### The image

`deploy/Dockerfile` is the only Dockerfile. Stages, cheapest to change last:
console (bun, runs on the builder's native arch) → `cargo-chef` recipe →
dependencies (cached until a `Cargo.toml`/`Cargo.lock` changes) → workspace →
`scratch` runtime with the CA bundle and the binary. The runtime has no shell, so
its `HEALTHCHECK` runs `vkdg healthcheck`. Images carry OCI labels with the
commit (`docker inspect --format '{{index .Config.Labels "org.opencontainers.image.revision"}}'`).

### Publishing to a registry instead

```bash
docker login ghcr.io
just push-dev                       # ghcr.io/vkdprojects/vkdg:dev-<sha> and :dev
just push-dev my-registry.example.com/team
```

Avoid QEMU emulation (`docker build --platform linux/amd64` of the full build on
an ARM host): it translates every `rustc` instruction and turns a 2-minute
compile into an hour. Cross-compiling with zig, as `ship-dev` does, emulates
nothing.

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
