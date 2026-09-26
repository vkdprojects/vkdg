---
name: vkdg-dx
description: "Rust DX and code quality standards for VKDG in 2026 — workspace layout, constructors, errors, async, tests, naming, and gateway patterns."
---

# VKDG DX Standards — Rust 2026

Reference: matklad (rust-analyzer), Microsoft Pragmatic Rust Guidelines, Tower, SNAFU.

## Workspace layout

- Virtual manifest at root, no `src/` there.
- All crates are siblings under `crates/`. Never nest a crate inside another's `src/`.
- Directory name == crate name. `workspace.dependencies` for all versions.
- Internal crates use `version = "0.0.0"` — signals non-publishable, no semver pressure.
- Split at semantic ownership boundaries, not file size. ~1 crate per 5–10 kloc is healthy.

## lib.rs is a map, not an implementation file

```rust
// crates/vkdg-routing/src/lib.rs
mod scorer;
mod strategies;
pub(crate) mod hints;

pub use scorer::{ScoringWeights, CandidateSignals, rank_candidates};
pub use strategies::{Router, RouteConfig, RoutingHints, StrategyKind};
```

Users write `use vkdg_routing::Router`, not `use vkdg_routing::strategies::Router`.
The module hierarchy is an implementation detail.

## Visibility

| Modifier | When |
|---|---|
| `pub` | Part of the crate's public API |
| `pub(crate)` | Used by multiple modules in this crate; test helpers |
| `pub(super)` | Rare — reconsider the module boundary if you need this often |
| private | Default. Promote only when a compiler error says you must |

## Constructors

- **`new(required_fields)`**: simple types with few required fields.
- **`minimal() + with_*()`**: types with many optional fields. `minimal()` sets all optionals to `None`/`false`.
- **Builder**: public APIs with invariants across fields, or forward-compatibility needs.
- **Default + struct update syntax**: when callers override 1–2 fields.
- **TypeState**: public APIs where construction order must be enforced at compile time. Internal code: runtime check + clear error message.

`PipelineState::minimal()` is the right pattern for internal gateway state. Never copy all 18 fields manually — that's what `minimal()` is for.

## Errors

**Library crates:** `thiserror` typed enums. Never `anyhow` in public return types.
Every error variant should be a distinct semantic case callers might match.

**Binary / app code:** `anyhow` with `.context()`. Fast to write, carries context, good backtraces.

**Never:**
- `Box<dyn Error>` in library public APIs
- `#[error("{self:?}")]` — that's just Debug forwarded to Display
- Error types where `Display` reads like Rust internals

**Adding context:**
```rust
// thiserror: context in the variant name and fields
#[error("upstream connect failed for provider {provider}")]
UpstreamConnect { provider: String, #[source] source: io::Error }

// anyhow: layer at call site
op().with_context(|| format!("processing request {id}"))?
```

## Async

- Mark a function `async` only if it actually `.await`s something.
- `Arc<Mutex<T>>` in async context: use `tokio::sync::Mutex`, not `std::sync::Mutex`.
- Config/routing table with many readers, rare writes: `ArcSwap<T>` for lock-free reads. `tokio::sync::watch` for broadcasting config updates to subscribers.
- `async fn` in traits: stable since 1.75 for static dispatch (generics). Not dyn-compatible yet. Use `async_trait` crate when you need `dyn MyTrait`.

## Strings in structs

Always `String` in structs. Use `&str` or `impl AsRef<str>` in function parameters.
Never put `&str` in a struct — the lifetime makes it unusable in `Arc`, `HashMap`, async code.

## Tests

**Integration tests:** one binary under `tests/it/` (not many `tests/*.rs` files — each is a separate linker invocation).

**Unit tests:** `mod tests;` pointing to an external file, not inline in the module:
```rust
// feature_a.rs
#[cfg(test)]
mod tests; // points to feature_a/tests.rs
```

**Shared test helpers:** live in the single integration binary as a `mod common`, or in a dedicated `crates/test_utils/` with `publish = false`.

**Disable doc tests for internal crates:**
```toml
[lib]
doctest = false
```

**What's worth writing:**
- A test that would have caught a real bug
- Public API tests (they're also usage documentation)
- Edge cases the type system can't prevent
- A regression test every time you fix a bug

**Not worth writing:**
- Tests that verify wiring/forwarding
- Tests that restate the implementation
- Tests that assert `result.is_ok()` without checking the value
- Tests of mocks rather than real behavior

**Check helper pattern:**
```rust
#[track_caller]
fn check(input: &str, expected: &str) {
    assert_eq!(transform(input), expected);
}
```
Wrap the API under test in one `check()` function. Refactors don't break the test structure.

## Naming

- Newtypes for IDs and units: `RequestId(Uuid)` prevents mixing `request_id` with `connection_id`.
- Type aliases only for documentation, not safety: `type Result<T> = std::result::Result<T, VkdgError>`.
- Module names: noun (what it is), not verb: `parser`, `router`, `executor`.
- No `_async` suffix on async functions — the type system makes it clear.
- Re-export from `lib.rs` everything users need to name. Don't expose internal module paths.

## Gateway-specific patterns

**Request context threading (Tower/axum):**
```rust
// Middleware inserts
req.extensions_mut().insert(AuthContext { tenant_id });
// Handler reads
let auth = req.extensions().get::<AuthContext>().expect("auth middleware must run first");
```

**Pipeline state:** `PipelineState` should be `Clone` via `Arc<Inner>` so cloning is an atomic increment, not a deep copy.

**Config hot-reload:** Use `ArcSwap<Config>` — readers pay one atomic load per access, writers swap the pointer without blocking any reader.

**Plugin system:**
- Trait objects (`Arc<dyn Plugin>`) for first-party plugins compiled into the binary.
- WASM (Wasmtime + WIT) for user-supplied plugins needing sandboxing and cross-language support. The gateway owns HTTP, I/O, and credentials; plugins only transform data.

## Red flags

- `async fn` that contains no `.await` — remove `async`
- `unwrap()` outside of tests or startup code with a clear invariant comment
- `format!("{:?}", value)` in Display impls
- A struct literal repeated verbatim in 3+ places — add a constructor
- `mod.rs` files — use `module_name.rs` (Rust 2018+ convention)
- `pub use module::*` at the crate root — explicit re-exports only
- `clone()` on large structures to avoid lifetime reasoning — rethink ownership
