# vkdg-connections

Upstream connection catalog, RAII `ConnectionGuard`, and `CredentialManager`. Does not implement HTTP or routing strategies.

## Public API

- `ConnectionConfig` — declarative configuration for a connection (provider, auth, capabilities, limits)
- `ProviderKind` — enum of supported providers (`Anthropic`, `OpenAI`, `Custom { base_url }`)
- `AuthKind` — authentication strategy (`ApiKey`, `OAuth2`)
- `Connection` — runtime state of a connection: config + token + concurrency counters
- `ConnectionGuard` — RAII that decrements `active_requests` on drop; acquired via `Connection::acquire()`
- `ConnectionState` — current state (`Active`, `Cooldown`, `Disabled`)
- `ConnectionCatalog` — map of `ConnectionId → Arc<RwLock<Connection>>`; lookup and listing
- `CredentialManager` — manages token refresh; singleflight per connection (Phase C complete)
- `TokenState` — holds the access token; manual `Debug` implementation that omits the sensitive value

## Invariants

- `TokenState` never exposes the token in `Debug` or logs
- `ConnectionGuard::drop` always decrements the active request counter
- `ConnectionCatalog` does not validate eligibility — that is `vkdg-routing`'s responsibility

## Focal test

```bash
cargo test -p vkdg-connections
```

## Used by

`vkdg-http` (via `PipelineState`), `bin/vkdg`, `tests/conformance`.
