# Contributing to VKDG

Good contributions share one trait: the author read the relevant skill guide first.
`.agents/skills/` has a file for each subsystem. A PR that ignores those conventions
takes longer to review and usually needs a rewrite.

## Before you start

For anything beyond a small fix, open an issue first. PRs that conflict with
planned design changes get closed, not revised. Small fixes (docs, typos, obvious
bugs) can go straight to a PR.

## Development setup

```bash
# Clone
git clone https://github.com/vkdprojects/vkdg
cd vkdg

# Run tests (no external keys needed; fake upstreams cover almost everything)
cargo test --workspace

# Smoke test with fake provider
cargo test -p conformance smoke -- --nocapture

# Lint
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings

# Check docs compile
cargo doc --workspace --no-deps

# Validate config file
cargo run -p vkdg -- config check <path>
```

## Git hooks (optional but recommended)

Install [lefthook](https://lefthook.dev) to enforce formatting and lint checks locally before committing:

```bash
# macOS
brew install lefthook

# Or via cargo
cargo binstall lefthook

# Activate hooks for this repo
lefthook install
```

Once installed, lefthook runs automatically on:
- **pre-commit**: typos spell check + rustfmt check on staged files
- **pre-push**: `cargo clippy -- -D warnings` + `cargo machete` (unused dep check)

To skip hooks when needed: `LEFTHOOK=0 git push`


The conformance suite starts a `FakeUpstream` in-process. You do not need real
provider credentials to run any of the existing tests. If you are adding a new
provider adapter, the same fake covers the wire protocol; check
`tests/conformance/src/fake_upstream.rs` for `FakeUpstreamBehavior` variants.

## PR checklist

- [ ] `cargo fmt --all` passes
- [ ] `cargo clippy --workspace -- -D warnings` passes
- [ ] `cargo test --workspace` passes
- [ ] New behavior has a test that fails before the change and passes after
- [ ] Public API changes have rustdoc comments
- [ ] CHANGELOG.md updated under `## Unreleased` (release-plz can handle this if you skip it)

## Code style

Clippy warnings are errors in CI. Fix them rather than suppressing with `#[allow]`.

No `unwrap()` or `expect()` on external input paths. Return `VkdgError` instead.
The distinction matters: panics in the hot path bring down all tenant traffic.

Each `#[test]` should document the plausible wrong implementation it defeats:

```rust
// Plausible wrong impl: admits request before checking semaphore
#[test]
fn admission_blocks_before_routing() { ... }
```

Tests that only verify wiring or that a function returns `Ok(())` on a happy path
get asked to demonstrate an actual defect they prevent.

## Adding a provider

See [docs/sdk/adding-a-provider.md](docs/sdk/adding-a-provider.md). The short path:
create `crates/vkdg-provider-<name>/`, implement `ProviderAdapter`, add tests. The
core pipeline does not change.

## Adding a plugin

See [docs/sdk/writing-a-plugin.md](docs/sdk/writing-a-plugin.md) for the WASM/Component
Model guide.

## License

Contributions are licensed under the same terms as the project. See [LICENSE](LICENSE).
