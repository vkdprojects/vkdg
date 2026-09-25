# Writing a VKDG Plugin

Plugins extend gateway behavior at well-defined hook points without modifying core code. This guide covers WASM plugins (the primary extension mechanism from Phase D onward).

## 1. Plugin taxonomy

| Kind | Description |
|---|---|
| `ingress` | Transforms or validates the inbound request before it reaches the pipeline |
| `provider` | Implements the `ProviderAdapter` WIT contract to add a new upstream provider |
| `policy` | Implements admission, rate-limiting, or auth decisions |
| `observer` | Reads pipeline events for logging, metrics, or tracing; no side effects on the request |

Each plugin declares its kind in its manifest. The host uses this to enforce fail behavior (see §7).

## 2. WIT interface

The WASM plugin contract is defined in `wit/provider.wit`:

```wit
package vkdg:provider@0.1.0;

interface types {
    type request-body  = list<u8>;
    type response-body = list<u8>;

    record prepared-request {
        url:          string,
        body:         request-body,
        is-streaming: bool,
    }

    variant error {
        capability-unsupported(string),
        upstream-error(u16),
        internal(string),
    }
}

world provider {
    use types.{ prepared-request, error };

    /// Serialised Operation JSON + ConnectionConfig JSON + bearer token.
    export prepare: func(operation-json: string, config-json: string, token: string)
        -> result<prepared-request, error>;

    /// Plugin name for logging and the registry.
    export name: func() -> string;
}
```

Auth headers are **not** included in the `prepared-request` record in the WIT interface — the host injects them separately. Plugins receive the raw bearer `token` string and must embed it in the body or a header field as their protocol requires.

## 3. Manifest

Every plugin must supply a `PluginManifest`. The manifest is registered before the component binary is installed:

```rust
use vkdg_plugin_host::{HookKind, PluginId, PluginKind, PluginManifest};

let manifest = PluginManifest {
    id: PluginId("my-provider-v1".into()),
    version: "0.1.0".into(),
    kind: PluginKind::Wasm {
        path: "./plugins/my-provider.wasm".into(),
    },
    hooks: vec![HookKind::PreDispatch],
    description: "Routes requests to MyProvider API".into(),
};
```

Required fields:

| Field | Type | Description |
|---|---|---|
| `id` | `PluginId(String)` | Unique identifier; used for lookup and uninstall |
| `version` | `String` | Semver string; for logging only |
| `kind` | `PluginKind::Wasm { path }` | Path to the `.wasm` component file |
| `hooks` | `Vec<HookKind>` | Hook points this plugin registers for (see §5) |
| `description` | `String` | Human-readable description |

## 4. Implement in Rust targeting WASM

Minimum project setup:

```toml
# Cargo.toml
[package]
name = "my-provider-plugin"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
serde_json = "1"
wit-bindgen = "0.36"
```

Implement the exported WIT functions. With `wit-bindgen` generating bindings from `wit/provider.wit`:

```rust
// src/lib.rs
wit_bindgen::generate!({
    world: "provider",
    path: "../../wit/provider.wit",
});

use exports::vkdg::provider::types::{Error, PreparedRequest};

struct MyPlugin;

impl exports::vkdg::provider::provider::Guest for MyPlugin {
    fn name() -> String {
        "my-provider".to_string()
    }

    fn prepare(
        operation_json: String,
        config_json: String,
        token: String,
    ) -> Result<PreparedRequest, Error> {
        // Parse the operation from JSON
        let op: serde_json::Value = serde_json::from_str(&operation_json)
            .map_err(|e| Error::Internal(e.to_string()))?;

        // Build the request
        let body = serde_json::json!({
            "model": "my-model",
            "prompt": op["messages"],
        });

        Ok(PreparedRequest {
            url: "https://api.myprovider.example/v1/complete".to_string(),
            body: serde_json::to_vec(&body).unwrap_or_default(),
            is_streaming: false,
        })
    }
}

export!(MyPlugin);
```

Build to a WASM component:

```sh
cargo build --target wasm32-wasip2 --release
# output: target/wasm32-wasip2/release/my_provider_plugin.wasm
```

## 5. Available hooks

Hooks are dispatched in pipeline order:

| Hook | When it fires | Typical use |
|---|---|---|
| `PreAuth` | Before credential resolution | Request validation, header normalization |
| `Auth` | During credential resolution | Custom auth schemes |
| `RateLimit` | After auth, before routing | Token bucket, quota enforcement |
| `PreDispatch` | After routing, before upstream send | Request mutation, logging |
| `OnEvent` | Each SSE event received from upstream | Stream filtering, event counting |
| `OnComplete` | After a successful non-streaming response | Response mutation, cost tracking |
| `OnFinish` | After every request (success or failure) | Audit logging, metrics flush |

A plugin declares which hooks it handles in `manifest.hooks`. Hooks not listed are never called for that plugin.

## 6. Install and rollback

```rust
use vkdg_plugin_host::{HookKind, PluginHost, PluginId, PluginKind, PluginManifest, PluginRegistry};

let registry = PluginRegistry::new();
let mut host = PluginHost::new(registry);

let manifest = PluginManifest {
    id: PluginId("my-provider-v1".into()),
    version: "0.1.0".into(),
    kind: PluginKind::Wasm { path: "./my-plugin.wasm".into() },
    hooks: vec![HookKind::PreDispatch],
    description: "My provider plugin".into(),
};

// Validates the binary *before* touching the registry.
// If the file is missing or the component is malformed, the registry
// is left unchanged — no partial state to clean up.
host.install_wasm(manifest, "./my-plugin.wasm")?;

// Later: unload cleanly
let removed = host.uninstall(&PluginId("my-provider-v1".into()));
assert!(removed);
```

`install_wasm` uses Wasmtime to validate the component binary before any registry mutation. A corrupt or incompatible `.wasm` file returns an error and leaves the host state unchanged.

## 7. Fail behavior

| Plugin kind | On failure |
|---|---|
| `Auth` / `policy` | **Fail closed** — request is rejected with an error response |
| `observer` / enrichment | **Fail open** — error is logged; request continues |

The host enforces this distinction based on `PluginKind`. Never rely on an observer plugin to enforce security decisions.

## 8. Host function limits

WASM components run in an isolated sandbox. Plugins **cannot**:

- Make outbound HTTP requests directly (no WASI `http` capability granted by default)
- Access the filesystem
- Read environment variables

The host exposes a set of host functions for common needs:

| Host function | Purpose |
|---|---|
| `log(level, message)` | Emit a structured log line |
| `cache_get(key)` | Read a value from the gateway's shared cache |
| `cache_set(key, value, ttl_secs)` | Write a value to the shared cache |
| `metrics_incr(name, labels)` | Increment a counter metric |

These are defined as WIT imports and are injected by the host at instantiation time.

## 9. Testing

`WasmPluginInstance::load` validates the component binary at the Wasmtime level. Use it in your own tests to verify the component compiles and passes validation before deploying:

```rust
#[test]
fn wasm_component_loads() {
    // Build your plugin first: cargo build --target wasm32-wasip2 --release
    let path = "target/wasm32-wasip2/release/my_provider_plugin.wasm";
    let instance = WasmPluginInstance::load(path)
        .expect("component should be valid");
    // load() validates — if it returns Ok, the binary is a well-formed component.
    drop(instance);
}

#[test]
fn install_and_uninstall_lifecycle() {
    let mut host = PluginHost::new(PluginRegistry::new());
    let manifest = PluginManifest {
        id: PluginId("test-plugin".into()),
        version: "0.1.0".into(),
        kind: PluginKind::Wasm { path: "test.wasm".into() },
        hooks: vec![HookKind::OnFinish],
        description: "test".into(),
    };

    // install_wasm returns Err if binary is missing/invalid — test rollback
    let result = host.install_wasm(manifest.clone(), "/nonexistent/path.wasm");
    assert!(result.is_err(), "bad path should fail");
    // Registry must be unchanged after the failed install
    assert!(host.registry.plugins_for_hook(&HookKind::OnFinish).is_empty());
}
```

## 10. Current limitations (Phase E)

`WasmPluginInstance::call_prepare()` is currently a **stub** — it re-validates the component binary but returns `Err("wasm call not yet wired (Phase E)")`. Full `wasmtime::component::bindgen!` codegen and typed dispatch are landing in Phase E.

What works today:
- `install_wasm` — validates and registers any well-formed WASM component
- `uninstall` — drops the instance and removes from registry
- `plugins_for_hook` — manifest-based hook dispatch
- `WasmPluginInstance::load` — binary validation

What requires Phase E:
- Actually calling `prepare()` or `name()` inside a loaded component
- Passing `operation_json` / `config_json` across the WASM boundary
