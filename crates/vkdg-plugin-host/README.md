# vkdg-plugin-host

Plugin manifest registry and hook dispatch. WASM execution via Wasmtime + WIT is Phase D — currently only `NativeRust` plugins are executed; `Wasm` manifests are stored but not called.

## Public API

- `PluginId` — plugin identifier newtype
- `PluginKind` — `NativeRust` | `Wasm { path }`
- `HookKind` — phases at which a plugin may intervene: `OnDecode`, `PreRoute`, `RoutePolicy`, `PreDispatch`, `OnEvent`, `OnComplete`, `OnJobTransition`, `OnFinish`
- `PluginManifest` — plugin declaration: id, kind, subscribed hooks, required capabilities
- `PluginRegistry` — map of `PluginId → PluginManifest`; `register`, `get`, `plugins_for_hook`
- `PluginHost` — owns the registry; will coordinate WASM execution in Phase D

## Invariants

- `Wasm` manifests are accepted in the registry but not executed until Phase D
- Plugins have no direct access to HTTP, credentials, or storage — only to the hook contract
- `PluginRegistry::plugins_for_hook` returns only plugins registered for that `HookKind`

## Focal test

```bash
cargo test -p vkdg-plugin-host
```

## Used by

`vkdg-routing` (route hooks), `bin/vkdg` (host initialization). Phase D: inline WASM execution.
