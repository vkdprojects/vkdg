---
name: vkdg-dx
description: "Rust DX and code quality standards for VKDG in 2026 — workspace layout, constructors, errors, async, tests, naming, toolchain config, and gateway patterns."
---

# VKDG DX Standards — Rust 2026

Reference: matklad (rust-analyzer), Microsoft Pragmatic Rust Guidelines, Tower, SNAFU, ruff, ripgrep.

---

## Toolchain configuration (already set up in the repo)

### rustfmt (.rustfmt.toml)
```toml
edition = "2021"
max_width = 100
use_small_heuristics = "Full"
```
Stable only — no nightly options. Run `cargo fmt --all` once after changes, then `cargo fmt --check` in CI.

### Clippy (clippy.toml + [workspace.lints])
```toml
# clippy.toml — thresholds
msrv = "1.80"
too-many-arguments-threshold = 8
too-many-lines-threshold = 120
cognitive-complexity-threshold = 30
```

```toml
# Cargo.toml [workspace.lints.clippy]
pedantic = "warn"    # not deny — some false positives
nursery = "warn"
suspicious = "deny"  # usually real bugs
performance = "deny" # concrete pessimizations

# Gateway-specific allows:
module_name_repetitions = "allow"
must_use_candidate = "allow"
missing_errors_doc = "allow"
items_after_statements = "allow"
```

Every member crate has `[lints] workspace = true`.

CI runs `cargo clippy --workspace -- -D warnings`. That is the gate.

### cargo-deny (deny.toml)
- `[graph] targets` scoped to platforms you ship
- `unsound = "all"`, `unmaintained = "workspace"`
- Explicit ban on openssl (VKDG uses rustls)
- License allowlist with `confidence-threshold = 0.93`

### cargo-audit (audit.yml)
Runs daily via `rustsec/audit-check`. Separate from cargo-deny. Both serve different purposes:
- cargo-deny: policy (licenses, bans, sources, version pins)
- cargo-audit: early warning on live CVEs

### Lefthook (lefthook.yml)
```
pre-commit (parallel, fast):
  typos {staged_files}           # ~50ms
  rustfmt --check {staged_files} # ~200ms per file

pre-push (serial, slower):
  cargo clippy -- -D warnings    # 5-30s warm cache
  cargo machete                  # ~2-3s, unused deps
```

Install: `brew install lefthook && lefthook install`
Skip: `LEFTHOOK=0 git push`

### typos (_typos.toml)
Uses a known-bad-word list, not a dictionary. Very low false positive rate. Add project identifiers to `extend-identifiers` if they fire incorrectly.

---

## Workspace layout

- Virtual manifest at root, no `src/` there.
- All crates siblings under `crates/`. Never nest a crate inside another's `src/`.
- Directory name == crate name. `workspace.dependencies` for all versions.
- Internal crates: `version = "0.0.0"`, `publish = false` (implicit for internal).
- `doctest = false` in `[lib]` for every internal crate — zero doctests means zero extra link steps.
- Split at semantic ownership boundaries, not file size. ~1 crate per 5–10 kloc.

## lib.rs is a map, not an implementation file

```rust
// lib.rs
mod catalog;
mod connection;
mod credentials;

pub use catalog::ConnectionCatalog;
pub use connection::{ConnectionConfig, ProviderKind};
pub use credentials::CredentialManager;
```

Users import from the crate root. Module paths are implementation details.

## Visibility

`pub` → part of the crate's public API.
`pub(crate)` → used by multiple modules in this crate; test helpers.
private → everything else. Promote only when the compiler forces you to.

---

## Constructors

- **`new(required_fields)`**: simple types with ≤3 required fields.
- **`minimal() + with_*()`**: types with many optional fields. `minimal()` sets all optionals to `None`/`false`. Eliminates 18-field struct literals repeated in every test helper.
- **Builder**: public APIs with cross-field invariants or forward-compatibility needs.
- **Default + struct update syntax**: when callers override 1–2 fields.
- **TypeState**: public APIs where wrong construction order must be a compile error. Internal code: runtime check + clear error message.

`PipelineState::minimal()` is the right pattern — never copy all 18 fields manually across test helpers.

---

## Errors

**Library crates (`pub` API):** `thiserror` typed enums. Never `anyhow`.

**Binary / app code:** `anyhow` with `.context()`.

**Never:**
- `Box<dyn Error>` in public return types
- `#[error("{self:?}")]` — that's Debug forwarded to Display

Add `VkdgError::http_status() -> u16` to avoid repeating the status mapping in every ingress crate.

---

## Async

- Mark `async` only if the function actually `.await`s something. The admission guard `acquire()` was async with no await — that's confusing API.
- `Arc<Mutex>` in async: use `tokio::sync::Mutex`, not `std::sync::Mutex`.
- Shared config (many readers, rare writes): `ArcSwap<Config>` for lock-free reads, `tokio::sync::watch` for broadcasting updates.
- `async fn` in traits: AFIT stable since 1.75 for static dispatch. Not dyn-compatible. Use `async_trait` crate for `dyn Trait`.

---

## Tests

**Integration tests:** one binary under `tests/it/` or `tests/conformance/`. Multiple `tests/*.rs` files = multiple linker invocations.

**Unit tests:** inline `mod tests {}` is fine for small modules. For files >200 lines of tests, use `mod tests;` (external file) to avoid recompiling the whole crate when only tests change.

**`doctest = false`** in all internal crates. Zero doctests = zero extra binaries.

**Test helpers:**
- Shared helpers in the same integration binary as `mod common`
- `make_pipeline()` should use `PipelineState::minimal()` — not 18 fields inline
- `make_ctx_streaming(model, api_type)` avoids repeating the 13-field RequestEnvelope literal

**What's worth writing:**
- A test that would have caught a real bug
- Public API tests (usage documentation)
- Edge cases the type system can't prevent
- A regression test every time you fix a bug

**Not worth writing:**
- Tests that only verify wiring/forwarding
- Tests that assert `result.is_ok()` without checking the value
- Tests of a mock's own behavior

---

## Naming

- Newtypes for IDs: `RequestId(Uuid)`, `ConnectionId(String)` — mixing them up is a compile error.
- Module names: noun (what it is) not verb: `pipeline`, `router`, `session`.
- No `_async` suffix — the type system makes it clear.
- `provider_str()` local helpers → `ProviderKind::as_str()` on the type.

---

## Anti-patterns to catch before committing

- `async fn` with no `.await` — remove `async`
- `unwrap()` in production paths without an invariant comment
- Struct literal with 10+ fields repeated in 3+ places — add a `minimal()` constructor
- `format!("{:?}", value)` in Display impls — that's Debug, not human-readable
- A file >300 lines mixing unrelated responsibilities — split by concern
- `pub use module::*` at crate root — explicit re-exports only
- Step labels like `// Step 4.65:` — signals the function grew by insertion; refactor the numbering
- `let _ = some_future()` — use `drop(some_future())` or `tokio::spawn(some_future())` per the `let_underscore_future` lint

---

## Gateway-specific patterns

**Request context threading:**
```rust
req.extensions_mut().insert(AuthContext { tenant_id });
let auth = req.extensions().get::<AuthContext>().expect("auth runs first");
```

**Pipeline state:** hold `Option<Arc<T>>` for optional components (cache, compressor, session). Mandatory components are non-optional fields.

**Plugin system:**
- Trait objects (`Arc<dyn Plugin>`) for first-party plugins.
- WASM (Wasmtime + WIT) for user-supplied plugins needing sandboxing and cross-language support.

**Config hot-reload:** `ArcSwap<ConfigSnapshot>` for lock-free reads. `tokio::sync::watch` channel to broadcast new snapshots. Old snapshot stays alive until all in-flight requests drain.
