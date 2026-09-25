# ADR-002: CapabilitySet in vkdg-core

**Status:** Accepted  
**Date:** 2026-09-25

## Context

`Capability` and `CapabilitySet` lived in `vkdg-operations`. The `vkdg-connections` crate needs `CapabilitySet` in `ConnectionConfig` (each connection declares the capabilities it offers). This created a dependency:

```
vkdg-connections → vkdg-operations
```

However, `vkdg-connections` is infrastructure domain close to the core (account catalog, credentials, RAII guard), while `vkdg-operations` is operation contract domain (Conversation, Embedding, Image...). The correct dependency direction is:

```
vkdg-operations → vkdg-core
vkdg-connections → vkdg-core
```

Having `vkdg-connections → vkdg-operations` inverted this hierarchy, making `vkdg-operations` an unintentional shared infrastructure node.

## Decision

Move `Capability` and `CapabilitySet` to **`vkdg-core`**.  
`vkdg-operations` re-exports both (`pub use vkdg_core::{Capability, CapabilitySet}`) so callers importing from `vkdg-operations` do not break.

## Reasons

- `CapabilitySet` is a pure domain type: `HashSet<Capability>` with set operations. It has no HTTP, DB, or WASM dependencies, so it belongs in core.
- `vkdg-connections` now imports directly from `vkdg-core`, eliminating the inverted coupling.
- The re-export in `vkdg-operations` preserves backward compatibility for existing callers (`use vkdg_operations::CapabilitySet` continues to work).

## Consequences

- `vkdg-connections` no longer depends on `vkdg-operations`.
- The crate dependency graph is acyclic with a clear direction toward the core.
- New crates that need only `CapabilitySet` can import from `vkdg-core` without pulling in the entire operation contracts.
