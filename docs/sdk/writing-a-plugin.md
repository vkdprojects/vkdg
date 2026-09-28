# Writing a VKDG Plugin

A VKDG plugin is a WebAssembly component plus a `manifest.yaml`, installed into the gateway's plugin
directory and loaded without rebuilding the binary.

## What the gateway loads today

The WIT files in [`wit/`](../../wit/) describe more roles than the host runs. Two are wired end to end:

| Role | Manifest `kind` | What the gateway calls | Used by |
|---|---|---|---|
| Provider | `provider` | `name`, `display-name`, `model-patterns`, `prepare` | a connection with `provider: <plugin name>` |
| Route auth | `auth` | `name`, `authenticate` | a route with `hooks: { auth: [<plugin name>] }` |

`compressor`, `router` and `cache-backend` manifests install and validate, but nothing calls them yet.
For those, and for providers that need custom response decoding or login, write a Rust plugin under
`plugins/providers/` instead (see [adding-a-provider.md](adding-a-provider.md), Tier 3).

## The call convention

Every export takes one JSON string and returns one JSON string. This keeps the ABI stable while the
gateway's Rust types change, and a JSON round trip is small next to the upstream HTTP call it
precedes.

### Provider

`prepare` receives:

```json
{
  "operation":  { "Conversation": { "model": "my-model-1", "messages": [], "stream": true } },
  "connection": { "id": "c1", "provider": "my-provider", "models": ["my-model-*"],
                  "max_concurrent": 4, "weight": 1, "tags": [] },
  "credential": { "token": "sk-...", "extra": {} }
}
```

and returns the request to send:

```json
{ "url": "https://api.my-provider.example/v1/chat",
  "headers": [["authorization", "Bearer sk-..."]],
  "body": [123, 125],
  "is_streaming": true }
```

`body` is a byte array, so a JSON body goes out as its UTF-8 bytes. The gateway sends the request and
passes the response to the client unchanged, so the upstream must already answer in OpenAI or
Anthropic format.

### Route auth

`authenticate` receives who is calling, never their credentials:

```json
{ "key_id": "k-123", "tenant_id": "default", "client_ip": "203.0.113.7",
  "model": "claude-sonnet-4.5", "route_id": "guarded" }
```

and returns one of:

```json
{ "result": "allowed", "context": { "tenant_id": "default", "key_id": "k-123",
                                    "role": "user", "scopes": [] } }
{ "result": "denied", "reason": "outside business hours" }
```

The reason is logged; the client gets a plain 401.

## Failure behavior

Auth fails closed. A route that names an auth plugin denies the request when the plugin is missing,
fails to load, traps, runs out of fuel, or returns anything but `allowed`. A provider plugin that
fails returns an error for that request only.

The gateway refuses to start, and refuses a config reload, when a route names an auth plugin that is
not installed. The admin API refuses to remove an auth plugin while a route still names it.

## Sandbox limits

A component runs in Wasmtime with a fuel budget and a memory cap. It has no network, filesystem or
environment access: the gateway makes the upstream call, so a plugin never sees another connection's
credentials. WASI is provided only so a `wasm32-wasip2` guest that links Rust's `libstd` can
instantiate.

## Manifest

```yaml
# manifest.yaml
name: my-sso               # kebab-case; the id used in config
version: "1.0.0"
kind: auth                 # or provider
description: "Allow only keys from the SSO tenant"
license: MIT
install:
  wasm: "https://example.com/my-sso.wasm"
  checksum: "sha256:<64 hex>"  # sha256 of plugin.wasm; install refuses a mismatch
```

Unknown fields are rejected. `vkdg plugin validate-manifest manifest.yaml` runs the same check as the
registry bot.

## Install

```
my-sso/
  manifest.yaml
  plugin.wasm
```

```bash
vkdg plugin install ./my-sso/   # verifies the checksum and compiles the component first
vkdg plugin list
vkdg plugin remove my-sso
```

The CLI writes to the plugin directory (`$VKDG_PLUGINS_DIR`, else `plugins/` next to `accounts.db`);
a running gateway reads it at the next restart. Installing or removing through the console, or
`POST /admin/v1/plugins` and `DELETE /admin/v1/plugins/{name}`, applies to the next request without a
restart.

## Building a component

There is no published guest SDK yet. Two working references live in the repo:

- `crates/vkdg-plugin-host/tests/wasm_provider_call.rs`: a provider written in WAT whose `prepare`
  returns a fixed request.
- `crates/vkdg-plugin-host/tests/wasm_auth.rs`: auth components that allow, deny, trap, and exhaust
  their fuel.

From Rust, build with `cargo component build --release` against `wit/`. Any toolchain that emits a
`wasm32-wasip2` component and exports the functions above works.
