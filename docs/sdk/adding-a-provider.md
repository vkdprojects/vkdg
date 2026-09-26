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

Implement `ProviderAdapter` (and optionally `OAuthProvider`) from `vkdg-provider-sdk`:

```rust
use vkdg_provider_sdk::{OAuthConfig, OAuthFlow, OAuthProvider, ProviderAdapter,
                         PreparedRequest, ProviderError, TokenPair};
use vkdg_connections::ConnectionConfig;
use vkdg_operations::Operation;
use std::collections::HashMap;
use futures::future::BoxFuture;

pub struct MyProvider;

impl ProviderAdapter for MyProvider {
    fn id(&self) -> &str { "my-provider" }
    fn display_name(&self) -> &str { "My Provider" }

    fn prepare(
        &self,
        operation: &Operation,
        config: &ConnectionConfig,
        token: &str,
    ) -> Result<PreparedRequest, ProviderError> {
        // Build your upstream request
        // See plugins/providers/anthropic/src/lib.rs for a complete example
        todo!()
    }
}

impl OAuthProvider for MyProvider {
    fn oauth_config(&self) -> OAuthConfig {
        OAuthConfig {
            flow: OAuthFlow::AuthorizationCodePkce,
            authorize_url: Some("https://my-provider.example/oauth/authorize".into()),
            token_url: "https://my-provider.example/oauth/token".into(),
            client_id: std::env::var("MY_PROVIDER_CLIENT_ID").unwrap_or_default(),
            scopes: vec!["inference".into()],
            redirect_uri: None,
            extra_auth_params: HashMap::new(),
        }
    }

    fn refresh_token<'a>(
        &'a self,
        refresh_token: &'a str,
        _extra: &'a HashMap<String, String>,
    ) -> BoxFuture<'a, Result<TokenPair, ProviderError>> {
        Box::pin(async move {
            // POST to token_url with grant_type=refresh_token
            todo!()
        })
    }
}
```

See `plugins/providers/claude-code/` and `plugins/providers/kiro/` for complete OAuth examples.

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
