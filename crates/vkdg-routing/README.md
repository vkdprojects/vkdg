# vkdg-routing

Routing strategies and connection eligibility filtering. Does not execute HTTP, access credentials, or modify connection state.

## Public API

- `Router` — resolves a request to a `RouteResult` by applying strategy and eligibility filters
- `RouteConfig` — route configuration: candidate connections, weights, strategy, plugin hooks
- `RouteResult` — resolution result: chosen `ConnectionId` and excluded candidates with reason
- `Strategy` trait — interface for custom strategies (`select` receives eligible candidates and returns the chosen one)
- `RoundRobinStrategy` — selects by atomic round-robin counter
- `FallbackChainStrategy` — tries the first eligible entry in the ordered list
- `EligibilityFilter` — filters connections by `CapabilitySet` and `ExcludedCandidate`s
- `StrategyKind`, `PluginHooks`, `RouteId`, `ConnectionWeight` — configuration types

## Invariants

- `Router::resolve` never returns a connection excluded by `EligibilityFilter`
- Excluded connections always appear in `RouteResult::excluded` with an auditable reason
- `RoundRobinStrategy` uses `AtomicUsize` without a lock — safe for concurrent use

## Focal test

```bash
cargo test -p vkdg-routing
```

## Used by

`vkdg-http` (pipeline runner), `bin/vkdg`, `tests/conformance`.
