# Contributing to VKDG

Thank you for your interest in contributing! This guide covers everything you need
to make a clean, reviewable pull request.

## Before You Start

For anything beyond a small fix (typo, docs, obvious bug), **open an issue first**.
This avoids wasted effort on PRs that won't be accepted for design reasons.

Small fixes — documentation edits, typo corrections, trivial refactors — can go
directly as a PR.

## Development Setup

```bash
# Clone
git clone https://github.com/vkdprojects/vkdg
cd vkdg

# Run tests (no external keys needed — uses in-process fake upstreams)
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

## PR Checklist

- [ ] `cargo fmt --all` passes
- [ ] `cargo clippy --workspace -- -D warnings` passes
- [ ] `cargo test --workspace` passes
- [ ] New behavior has a test that fails before the change and passes after
- [ ] Public API changes have rustdoc comments
- [ ] CHANGELOG.md updated under `## Unreleased` (optional — release-plz can do this)

## Code Style

- Clippy warnings are errors in CI. Fix them rather than suppressing with `#[allow]`
- No `unwrap()` or `expect()` on external input paths. Return `VkdgError` instead
- Each `#[test]` should document the plausible wrong implementation it defeats,
  using a comment like: `// Plausible wrong impl: ...`
- Follow the skill guides in `.agents/skills/` — they define the coding contracts
  for each subsystem

## Adding a Provider

See [docs/sdk/adding-a-provider.md](docs/sdk/adding-a-provider.md) for a step-by-step
guide. The short version: create `crates/vkdg-provider-<name>/`, implement `ProviderAdapter`,
add tests, done — no changes to the core pipeline required.

## Adding a Plugin

See [docs/sdk/writing-a-plugin.md](docs/sdk/writing-a-plugin.md) for the WASM plugin guide.

## License

By contributing, you agree that your contributions will be licensed under the same
license as the project (see [LICENSE](LICENSE)).
