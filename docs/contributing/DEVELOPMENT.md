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

## Shipping a dev image from any machine

`deploy/Dockerfile.gateway` compiles Rust inside the build. That is right for a
release, but it costs about 15 minutes on a modest x86 host, and far longer if
Docker has to emulate x86 on an ARM machine.

For a dev loop, compile on your own machine and let the image just package the
binary. `rustc` runs natively — including on Apple Silicon — and emits x86, so
nothing is emulated:

```bash
# Once
brew install zig                       # or your platform's zig package
cargo install cargo-zigbuild
rustup target add x86_64-unknown-linux-musl

# Build, stream over SSH, and restart the remote container
just ship-dev <ssh-host>
just ship-dev <ssh-host> /srv/vkdg     # if compose lives elsewhere

# Undo it
just rollback-dev <ssh-host>
```

Measured on an M4 Pro against a 4-core Broadwell server:

| Step | Compiling inside the image | `just ship-dev` |
|---|---|---|
| Local machine to running container | 14m28s | **35s** |

`ship-dev` streams the image over the SSH connection, so it works against a host
that cannot reach a private registry, and it retags the previous image as
`vkdg-gateway:rollback` before switching.

Images are tagged `dev-<short-sha>`, with `-dirty` appended when the tree has
uncommitted changes — so an image that came from unpushed work says so.

### Publishing to a registry instead

```bash
docker login ghcr.io
just push-dev                          # ghcr.io/vkdprojects/vkdg:dev-<sha> and :dev
just push-dev my-registry.example.com/team
```

### If you cannot install zig

Register a remote x86 machine as a buildx node and build there natively. Slower
than a local cross-compile when your own machine is faster, but it needs nothing
installed locally:

```bash
docker buildx create --name vkdg-x86 --driver docker-container \
    --platform linux/amd64 ssh://<ssh-host>
docker buildx build --builder vkdg-x86 -f deploy/Dockerfile.gateway \
    -t <registry>/vkdg:dev --push .
```

Avoid QEMU emulation (`docker build --platform linux/amd64` on an ARM host with
no remote builder) for anything but a one-off: it translates every `rustc`
instruction, which turns a 2-minute compile into an hour. Docker Desktop's
Rosetta option is faster than QEMU but still translates the compiler; note that
Rosetta itself is for running x86 **macOS** binaries and does not help
cross-compilation.

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
