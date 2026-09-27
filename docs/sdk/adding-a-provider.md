# Adding a Provider to VKDG

There are three paths. Pick the one that matches your situation.

---

## Tier 1 — Config only (no code, no restart)

For any endpoint that speaks OpenAI or Anthropic wire format.

```yaml
# vkdg.yaml
connections:
  - id: my-local-llama
    provider: openai-compat
    base_url: http://localhost:11434
    auth:
      type: api_key
      env_var: MY_KEY          # or omit for keyless endpoints
    models: ["llama3.3:70b"]
    max_concurrent: 4
    weight: 1
```

That's it. No code. No binary rebuild. The `openai-compat` and `anthropic-compat` provider types
are handled by the built-in adapters; you only supply the URL and credentials.

**When to use this:**
- Local Ollama, vLLM, LM Studio, llama.cpp
- Any hosted API that exposes `/v1/chat/completions` (OpenAI) or `/v1/messages` (Anthropic)
- Self-hosted Open WebUI, LocalAI, etc.
- Internal corporate LLM endpoints

**Not for:** providers with custom wire formats, OAuth flows, or non-standard auth.

---

## Tier 2 — WASM plugin (custom protocol, no fork)

For a provider with a custom wire format — when you need to control the exact bytes sent and
received. Write a `.wasm` file; VKDG loads it at startup from the `plugins/` directory.

**You do not need to clone the VKDG repo. You do not rebuild the gateway binary.**

### What to implement

The contract is in [`wit/provider.wit`](../../wit/provider.wit):

```wit
interface provider-plugin {
    // Build the HTTP request your provider expects.
    // Returns: (url, body-bytes, is-streaming)
    // You own the URL, body, and streaming flag.
    // VKDG owns TLS, retries, auth injection, and the HTTP client.
    translate-request: func(
        req: request,
        connection-config-json: string,
        token: string,
    ) -> result<tuple<string, list<u8>, bool>, plugin-error>;

    // Convert a complete (non-streaming) response body to VKDG format.
    translate-response: func(
        body: list<u8>,
        request-id: string,
    ) -> result<response, plugin-error>;

    // Convert one SSE chunk. Return None to skip (heartbeat/comment).
    // Return Err(not-applicable) once if you don't support streaming.
    translate-chunk: func(
        chunk: list<u8>,
        request-id: string,
    ) -> result<option<response>, plugin-error>;

    // Model IDs or glob patterns this plugin handles.
    model-patterns: func() -> list<string>;

    name: func() -> string;
}
```

### Minimal Rust implementation

```rust
// Cargo.toml
// [dependencies]
// vkdg-provider-sdk = "0.1"   (when published — uses wit-bindgen internally)
// wit-bindgen = "0.35"

wit_bindgen::generate!({ world: "provider", path: "wit/" });

struct MyProvider;

impl Guest for MyProvider {
    fn name() -> String {
        "my-provider".into()
    }

    fn model_patterns() -> Vec<String> {
        vec!["my-model-*".into()]
    }

    fn translate_request(
        req: Request,
        config_json: String,
        token: String,
    ) -> Result<(String, Vec<u8>, bool), PluginError> {
        let config: serde_json::Value = serde_json::from_str(&config_json).unwrap_default();
        let model = config["models"][0].as_str().unwrap_or("default");

        // Build your provider's wire format
        let body = serde_json::json!({
            "model": model,
            "messages": req.messages,  // already in VKDG internal format
        });

        let url = "https://api.my-provider.example/v1/generate".into();
        let body_bytes = serde_json::to_vec(&body).unwrap_or_default();
        let is_streaming = req.stream;

        Ok((url, body_bytes, is_streaming))
    }

    fn translate_response(body: Vec<u8>, _request_id: String) -> Result<Response, PluginError> {
        // Parse your provider's response and convert to VKDG Response format
        let value: serde_json::Value = serde_json::from_slice(&body)
            .map_err(|e| PluginError::Internal(e.to_string()))?;

        Ok(Response {
            content: value["output"].as_str().unwrap_or("").into(),
            // ... other fields
        })
    }

    fn translate_chunk(chunk: Vec<u8>, _request_id: String) -> Result<Option<Response>, PluginError> {
        // Parse one SSE chunk from your provider
        // Return Ok(None) to skip (heartbeat, comment, empty)
        // Return Err(PluginError::NotApplicable) once if you don't support streaming
        todo!()
    }
}

export!(MyProvider);
```

Compile to WASM:
```bash
cargo build --target wasm32-wasip2 --release
# produces target/wasm32-wasip2/release/my_provider.wasm
```

### Install

```
~/.config/vkdg/plugins/
  my-provider/
    provider.wasm
    manifest.toml
```

```toml
# manifest.toml
name = "my-provider"
kind = "provider"
version = "1.0.0"
```

Then add a connection pointing at it:
```yaml
connections:
  - id: my-provider-prod
    provider: my-provider       # matches name() from the plugin
    auth:
      type: api_key
      env_var: MY_PROVIDER_KEY
    models: ["my-model-*"]
    max_concurrent: 8
    weight: 1
```

VKDG loads it on startup (or hot-reloads if `watch_plugins: true`). No binary involved.

**When to use this:**
- Provider has a custom protocol (not OpenAI/Anthropic format)
- You need to control exact request/response transformation
- You want to ship a closed-source provider adapter
- You work with a provider whose API changes frequently and you own the update cycle

**Languages:** anything that compiles to `wasm32-wasip2` — Rust, C, Go (TinyGo), Python (Componentize-py), JavaScript (jco).

---

## Tier 3 — Contribute to `plugins/providers/` (first-party, compiled in)

For providers that should ship with VKDG and be maintained by the project.

These live in `plugins/providers/<name>/` and are compiled into the binary.
The full ecosystem of built-in providers (Anthropic, OpenAI, Gemini, Groq, etc.) lives here.

### When to use this path

- The provider is popular enough to belong in the default distribution
- You want it auto-registered (no plugin drop required for users)
- You're contributing back to the project

### For OpenAI-compatible providers

9 lines total. Most providers fall here.

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
- Admin API (loopback, session required): `GET /admin/v1/providers/{id}/login-methods`, `POST /admin/v1/oauth/{provider}/start` (`{method?, params}`), `POST /admin/v1/oauth/{provider}/poll` (`{login_id, code?}`; `code` is for PKCE), `POST /admin/v1/oauth/{provider}/import`, `GET /admin/v1/accounts`, `DELETE /admin/v1/accounts/{id}`. Responses never include tokens or plugin login state.
- At request time, `CredentialManager` loads the account and calls `refresh_token` through the registry once the token is within 5 minutes of expiry, with one refresh per account at a time. It persists the result and then passes `Credential { token, extra }` to `prepare()`.

See `plugins/providers/claude-code/` and `plugins/providers/kimi-coding/` for OAuth examples.

---

## Decision guide

```
Need a custom endpoint (OpenAI/Anthropic format)?
  → Tier 1: YAML config, done.

Need custom protocol / full control over bytes?
  → Tier 2: WASM plugin, no fork.

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
