# Adding a Provider to VKDG

There are three paths. Pick the one that matches your situation.

---

## Tier 1: config only (no code)

For any endpoint that speaks the OpenAI Chat Completions or Anthropic Messages wire format.

```yaml
# vkdg.yaml
connections:
  - id: my-local-llama
    provider: openai-compat          # or anthropic-compat for a /v1/messages endpoint
    base_url: http://localhost:11434 # host only: the adapter appends /v1/chat/completions
    auth:
      type: api_key
      env_var: MY_KEY                # any value works if the endpoint ignores it
    models: ["llama3.3:70b"]
    max_concurrent: 4
    weight: 1
```

Check it before you start the gateway:

```bash
vkdg config check vkdg.yaml
# OK: config valid (version 0, 1 connections, 0 routes)
```

`config check` refuses what `serve` would refuse: an unknown field (`base_ur:`), a `base_url` on any
provider other than the two compatible kinds, a missing `base_url` on them, and a provider id with no
adapter behind it. When the gateway runs with `vkdg serve --config <file>`, edits to that file apply
on save; an invalid edit is refused and the running config stays.

**When to use this:**
- Local Ollama, vLLM, LM Studio, llama.cpp
- Any hosted API that exposes `/v1/chat/completions` or `/v1/messages`
- Internal corporate endpoints

**Not for:** custom wire formats or OAuth logins. Compatible endpoints get no list price, so their
requests show an unknown cost in the request history, never an invented one.

---

## Tier 2: WASM plugin (custom request, no fork)

For a provider that needs its own URL, headers or request body. You ship a WebAssembly component and a
manifest; the gateway loads it without being rebuilt.

### What the gateway calls today

The full contract is in [`wit/provider.wit`](../../wit/provider.wit). The host currently calls four of
its exports, each taking and returning a JSON string:

| Export | Input | Output |
|---|---|---|
| `name` | `""` | provider id, e.g. `"my-provider"`; falls back to the manifest `name` |
| `display-name` | `""` | label for the console |
| `model-patterns` | `""` | JSON array of globs, e.g. `["my-model-*"]` |
| `prepare` | `{operation, connection, credential}` | `{url, headers, body, is_streaming}` |

`connection` is the connection without secrets (`id`, `provider`, `models`, `max_concurrent`, `weight`,
`tags`). `credential` is `{token, extra}`. `body` is a byte array, so a JSON body goes out as its UTF-8
bytes. The gateway sends the request and streams the response back to the client unchanged.

### Limitations

- The response must already be OpenAI or Anthropic shaped. `decode-response`, `decode-chunk` and
  `finish-stream` are in the WIT, but the host does not call them yet, so a binary or custom stream
  protocol still needs Tier 3.
- No login from WASM: `login-methods`, the device, PKCE and import functions, and `refresh-token` are
  not called yet. Use `auth: { type: api_key }` for a WASM provider.
- There is no published guest SDK. Build the component with `cargo component` against `wit/`, or any
  toolchain that produces a `wasm32-wasip2` component.
  `crates/vkdg-plugin-host/tests/wasm_provider_call.rs` is a working component, written in WAT, that
  returns a fixed `prepare` result.

### Install

A plugin is a directory with two files:

```
my-provider/
  manifest.yaml
  plugin.wasm
```

```yaml
# manifest.yaml
name: my-provider          # kebab-case; also the `provider:` id in config
version: "1.0.0"
kind: provider
description: "My provider"
license: MIT
models: ["my-model-*"]
install:
  wasm: "https://example.com/my-provider.wasm"
  checksum: "sha256:<64 hex>"  # sha256 of plugin.wasm; the install is refused on mismatch
```

```bash
vkdg plugin validate-manifest my-provider/manifest.yaml  # same check the registry bot runs
vkdg plugin install ./my-provider/                        # verifies the checksum, compiles the component
```

`vkdg plugin install` writes to the plugin directory (`$VKDG_PLUGINS_DIR`, else `plugins/` next to
`accounts.db`). A running gateway picks it up on restart. Installing through the console, or
`POST /admin/v1/plugins`, applies to the next request with no restart.

Then point a connection at it:

```yaml
connections:
  - id: my-provider-prod
    provider: my-provider    # the plugin's name
    auth:
      type: api_key
      env_var: MY_PROVIDER_KEY
    models: ["my-model-*"]
```

A plugin cannot take the id of a built-in provider (`anthropic`, `kiro`, ...). It would receive their
credentials, so the gateway refuses it at load.

---

## Tier 3: contribute to `plugins/providers/` (first-party, compiled in)

For providers that should ship with VKDG and be maintained by the project.

These live in `plugins/providers/<name>/` and are compiled into the binary.
The full ecosystem of built-in providers (Anthropic, OpenAI, Gemini, Groq, etc.) lives here.

### When to use this path

- The provider is popular enough to belong in the default distribution
- You want it auto-registered (no plugin drop required for users)
- You're contributing back to the project

### For OpenAI-compatible providers

One function. Most hosted APIs fall here.

```
plugins/providers/my-provider/
  Cargo.toml
  src/lib.rs
```

```toml
# Cargo.toml
[package]
name = "vkdg-provider-my-provider"
version = "0.1.0"
edition = "2021"

[dependencies]
vkdg-provider-sdk = { workspace = true }

[lib]
doctest = false

[lints]
workspace = true
```

```rust
// src/lib.rs
//! VKDG provider: My Provider (OpenAI-compatible endpoint).
use vkdg_provider_sdk::openai_compat::OpenAiCompatAdapter;

pub fn provider() -> OpenAiCompatAdapter {
    OpenAiCompatAdapter::new(
        "my-provider",
        "My Provider",
        "https://api.my-provider.example",
        "my-model-default",
    )
}
```

Add to root `Cargo.toml` workspace members, then register in `bin/vkdg/src/main.rs`:

```rust
registry.register(Arc::new(vkdg_provider_my_provider::provider()));
```

Open a PR. That's it.

### For providers with OAuth or custom protocol

Implement `ProviderAdapter` from `vkdg-provider-sdk`. For OAuth, also implement `OAuthProvider` and return `Some(self)` from `ProviderAdapter::oauth()`. The core then handles login, persistence, refresh, the CLI and the admin API, so it needs no provider-specific code.

```rust
use std::collections::HashMap;
use futures::future::BoxFuture;
use vkdg_connections::ConnectionConfig;
use vkdg_operations::Operation;
use vkdg_provider_sdk::{
    Credential, DeviceAuthorization, DevicePoll, LoginField, LoginMethod, LoginParams,
    LoginResult, LoginState, OAuthConfig, OAuthFlow, OAuthProvider, PreparedRequest,
    ProviderAdapter, ProviderError, TokenPair,
};

pub struct MyProvider;

impl ProviderAdapter for MyProvider {
    fn id(&self) -> &str { "my-provider" }
    fn display_name(&self) -> &str { "My Provider" }

    // Opt in to login + refresh.
    fn oauth(&self) -> Option<&dyn OAuthProvider> { Some(self) }

    fn prepare(
        &self,
        operation: &Operation,
        config: &ConnectionConfig,
        credential: &Credential,
    ) -> Result<PreparedRequest, ProviderError> {
        // credential.token = access token (or API key).
        // credential.extra = per-account data you returned at login/refresh
        //                    (region, profile ARN, device id, …). Empty for API keys.
        todo!("see plugins/providers/anthropic/src/lib.rs")
    }
}

impl OAuthProvider for MyProvider {
    fn oauth_config(&self) -> OAuthConfig { todo!() }

    // Advertise methods; CLI and console render them generically.
    fn login_methods(&self) -> Vec<LoginMethod> {
        vec![LoginMethod {
            id: "device".into(),
            label: "Device code".into(),
            flow: OAuthFlow::DeviceCode,
            fields: vec![LoginField {
                id: "region".into(), label: "Region".into(),
                required: true, secret: false, default: Some("us-east-1".into()),
            }],
        }]
    }

    fn start_device_login<'a>(&'a self, _method: &'a str, params: &'a LoginParams)
        -> BoxFuture<'a, Result<DeviceAuthorization, ProviderError>> {
        // Call the device authorization endpoint. Put device_code etc. in `state`:
        // it stays server-side and comes back on every poll.
        todo!()
    }

    fn poll_device_login<'a>(&'a self, _method: &'a str, state: &'a LoginState)
        -> BoxFuture<'a, Result<DevicePoll, ProviderError>> {
        // Map authorization_pending → Pending, slow_down → SlowDown,
        // success → Done(LoginResult { tokens, label }), denial/expiry → Failed(msg).
        todo!()
    }

    fn refresh_token<'a>(&'a self, refresh_token: &'a str, extra: &'a HashMap<String, String>)
        -> BoxFuture<'a, Result<TokenPair, ProviderError>> {
        // Keys you return in TokenPair::extra overwrite stored ones; others are kept.
        // refresh_token: None keeps the stored refresh token.
        todo!()
    }
}
```

| Hook | Flow | Default |
|---|---|---|
| `login_methods()` | all | empty (refresh only) |
| `start_device_login` / `poll_device_login` | `DeviceCode` (RFC 8628) | `UnsupportedOperation` |
| `start_pkce_login` / `finish_pkce_login` | `AuthorizationCodePkce` | `UnsupportedOperation` |
| `import_token` | `ImportToken` (pasted refresh token / credential blob) | `UnsupportedOperation` |
| `refresh_token` | all | required |

What the core does with it:

- `vkdg login <provider> [--method <id>] [--opt key=value …]` runs the method, saves an account to the account store, and prints the `auth: { type: account, account: <id> }` snippet. `vkdg login <provider> --list-methods` shows the methods and fields. `vkdg accounts list|remove <id>` manages saved accounts.
- Connecting an account in the console (`POST …/poll`, `POST …/import`) also creates the connection that serves it: `id` = the account id, `auth: { type: account, account: <id> }`, `models` = `ProviderAdapter::default_models()` (default `["*"]`: return patterns only your provider serves, or an empty list when it cannot serve requests yet and no connection should exist). An account that already has a connection (reconnect, YAML, hand-made) keeps it untouched. The response carries `connection_id`, or `connection_error` when the account was saved but the connection was not. `DELETE /admin/v1/accounts/{id}` removes the account's connections and drops them from routes. `vkdg login` does not create one: it prints the snippet.
- Admin API (loopback, session required): `GET /admin/v1/providers/{id}/login-methods`, `POST /admin/v1/oauth/{provider}/start` (`{method?, params}`), `POST /admin/v1/oauth/{provider}/poll` (`{login_id, code?}`; `code` is for PKCE), `POST /admin/v1/oauth/{provider}/import`, `GET /admin/v1/accounts`, `DELETE /admin/v1/accounts/{id}`. Responses never include tokens or plugin login state.
- At request time, `CredentialManager` loads the account and calls `refresh_token` through the registry once the token is within 5 minutes of expiry, with one refresh per account at a time. It persists the result and then passes `Credential { token, extra }` to `prepare()`.

See `plugins/providers/claude-code/` and `plugins/providers/kimi-coding/` for OAuth examples.

---

## Decision guide

```
Endpoint speaks OpenAI or Anthropic format?
  → Tier 1: YAML config.

Needs its own URL, headers or body, and answers in OpenAI/Anthropic format?
  → Tier 2: WASM plugin, no fork.

Custom stream protocol, or an OAuth login?
  → Tier 3.

Want it bundled with VKDG for everyone?
  → Tier 3: PR to plugins/providers/.
```

---

## Reference

- WIT interface: [`wit/provider.wit`](../../wit/provider.wit)
- OpenAI-compat shared module: [`crates/vkdg-provider-sdk/src/openai_compat.rs`](../../crates/vkdg-provider-sdk/src/openai_compat.rs)
- `ProviderAdapter` trait: [`crates/vkdg-provider-sdk/src/request.rs`](../../crates/vkdg-provider-sdk/src/request.rs)
- `OAuthProvider` trait: [`crates/vkdg-provider-sdk/src/oauth.rs`](../../crates/vkdg-provider-sdk/src/oauth.rs)
- Built-in examples: `plugins/providers/anthropic/`, `plugins/providers/groq/`, `plugins/providers/claude-code/`
