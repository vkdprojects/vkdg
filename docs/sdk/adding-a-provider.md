# Adding a Provider Adapter

A provider adapter translates VKDG's internal `Operation` type into the wire format a specific upstream API expects. The pipeline core calls `prepare()` once per request and knows nothing about URLs, auth headers, or JSON schemas — that's the adapter's job.

## 1. The `ProviderAdapter` trait

Defined in `crates/vkdg-http/src/provider.rs`:

```rust
pub struct PreparedRequest {
    pub url: String,
    pub headers: HeaderMap,
    pub body: Bytes,
    pub is_streaming: bool,
}

pub trait ProviderAdapter: Send + Sync {
    fn name(&self) -> &str;

    fn prepare(
        &self,
        operation: &Operation,
        config: &ConnectionConfig,
        token: &str,
    ) -> Result<PreparedRequest, VkdgError>;
}
```

- `name()` — unique string identifier used in logs and error messages.
- `prepare()` — receives the decoded operation, the connection's config (models, capabilities, base URL), and the resolved bearer token. Returns a ready-to-send HTTP request or a `VkdgError`.

## 2. Create the crate

```
crates/vkdg-provider-myprovider/
├── Cargo.toml
└── src/
    └── lib.rs
```

Minimum `Cargo.toml`:

```toml
[package]
name = "vkdg-provider-myprovider"
version = "0.1.0"
edition = "2021"

[dependencies]
bytes = "1"
http = "1"
serde_json = "1"
vkdg-connections = { path = "../../crates/vkdg-connections" }
vkdg-core = { path = "../../crates/vkdg-core" }
vkdg-http = { path = "../../crates/vkdg-http" }
vkdg-operations = { path = "../../crates/vkdg-operations" }
```

Add the new crate to the workspace `Cargo.toml` members list.

## 3. Implement `ProviderAdapter`

```rust
use bytes::Bytes;
use http::HeaderMap;
use serde_json::json;
use vkdg_connections::{ConnectionConfig, ProviderKind};
use vkdg_core::VkdgError;
use vkdg_http::provider::{PreparedRequest, ProviderAdapter};
use vkdg_operations::Operation;

pub struct MyProviderAdapter;

impl ProviderAdapter for MyProviderAdapter {
    fn name(&self) -> &str {
        "myprovider"
    }

    fn prepare(
        &self,
        operation: &Operation,
        config: &ConnectionConfig,
        token: &str,
    ) -> Result<PreparedRequest, VkdgError> {
        let req = match operation {
            Operation::Conversation(r) => r,
            _ => return Err(VkdgError::Internal("unsupported operation".into())),
        };

        // 1. Resolve base URL
        let base = base_url(config);
        let url = format!("{base}/v1/chat");

        // 2. Build auth headers
        let mut headers = HeaderMap::new();
        headers.insert(
            "Authorization",
            format!("Bearer {token}")
                .parse()
                .unwrap_or_else(|_| http::HeaderValue::from_static("invalid")),
        );
        headers.insert(
            http::header::CONTENT_TYPE,
            http::HeaderValue::from_static("application/json"),
        );

        // 3. Serialize the body
        let model = config
            .models
            .first()
            .cloned()
            .unwrap_or_else(|| "myprovider-default".into());

        let body_value = json!({
            "model": model,
            "messages": req.messages.iter().map(|m| json!({
                "role": format!("{:?}", m.role).to_lowercase(),
                "content": "..."   // map MessageContent to your wire format
            })).collect::<Vec<_>>(),
            "stream": req.stream,
        });

        let body = Bytes::from(serde_json::to_vec(&body_value).unwrap_or_default());

        // 4. Set streaming flag
        Ok(PreparedRequest {
            url,
            headers,
            body,
            is_streaming: req.stream,
        })
    }
}

fn base_url(config: &ConnectionConfig) -> String {
    match &config.provider {
        ProviderKind::Custom { base_url } => base_url.clone(),
        _ => "https://api.myprovider.example".into(),
    }
}
```

## 4. Add to `ConnectionConfig`

In the YAML config, point a connection at your provider using the `custom:<url>` form:

```yaml
connections:
  - id: myprovider-prod
    provider: "custom:https://api.myprovider.example"
    auth:
      type: api_key
      env_var: MYPROVIDER_API_KEY
    models:
      - "myprovider-*"
    max_concurrent: 20
    weight: 1
```

`ConfigSnapshot::build` parses `"custom:<url>"` into `ProviderKind::Custom { base_url }`. If your provider warrants a first-class variant, add it to the `ProviderKind` enum in `crates/vkdg-connections/src/lib.rs` and update the `parse_provider` function in `crates/vkdg-config/src/snapshot.rs`.

## 5. Register in `PipelineState`

Wire the adapter where the pipeline state is constructed. Locate `build_pipeline_from_env()` (or your startup code) and swap in your adapter:

```rust
use vkdg_provider_myprovider::MyProviderAdapter;

let pipeline_state = PipelineState {
    provider_adapter: Arc::new(MyProviderAdapter),
    // ... other fields unchanged
};
```

## 6. Required tests

Follow the red-green-refactor discipline from `vkdg-test-first`. Every adapter needs at minimum:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use vkdg_connections::{AuthKind, ConnectionConfig, ProviderKind};
    use vkdg_core::{CapabilitySet, ConnectionId};
    use vkdg_operations::{ConversationRequest, Operation};

    fn test_config() -> ConnectionConfig {
        ConnectionConfig {
            id: ConnectionId("test".into()),
            provider: ProviderKind::Custom {
                base_url: "https://api.example.com".into(),
            },
            auth: AuthKind::ApiKey { env_var: "TEST_KEY".into() },
            models: vec!["myprovider-v1".into()],
            max_concurrent: 4,
            weight: 1,
            tags: vec![],
            capabilities: CapabilitySet::default(),
        }
    }

    fn minimal_op() -> Operation {
        // build a minimal ConversationRequest
        Operation::Conversation(ConversationRequest {
            messages: vec![],
            stream: false,
            ..Default::default()
        })
    }

    #[test]
    fn prepare_url_correct() {
        let result = MyProviderAdapter.prepare(&minimal_op(), &test_config(), "tok").unwrap();
        assert!(result.url.contains("api.example.com"), "url: {}", result.url);
        assert!(result.url.ends_with("/v1/chat"), "url: {}", result.url);
    }

    #[test]
    fn prepare_auth_header() {
        let result = MyProviderAdapter.prepare(&minimal_op(), &test_config(), "secret").unwrap();
        let auth = result.headers.get("Authorization").expect("missing Authorization");
        assert!(auth.to_str().unwrap().contains("secret"));
    }

    #[test]
    fn prepare_body_fields() {
        let result = MyProviderAdapter.prepare(&minimal_op(), &test_config(), "tok").unwrap();
        let body: serde_json::Value = serde_json::from_slice(&result.body).unwrap();
        assert!(body.get("model").is_some(), "body missing 'model'");
        assert!(body.get("messages").is_some(), "body missing 'messages'");
    }
}
```

## 7. Complete example: `echo` provider

A toy provider that returns the last user message as the assistant reply (useful for integration testing):

```rust
pub struct EchoAdapter;

impl ProviderAdapter for EchoAdapter {
    fn name(&self) -> &str { "echo" }

    fn prepare(
        &self,
        operation: &Operation,
        _config: &ConnectionConfig,
        _token: &str,
    ) -> Result<PreparedRequest, VkdgError> {
        let req = match operation {
            Operation::Conversation(r) => r,
            _ => return Err(VkdgError::Internal("echo: unsupported".into())),
        };

        // Encode the last user message so the fake upstream can mirror it back.
        let last = req.messages.last().map(|m| format!("{:?}", m.content)).unwrap_or_default();
        let body = Bytes::from(serde_json::to_vec(&serde_json::json!({ "echo": last })).unwrap());

        let mut headers = HeaderMap::new();
        headers.insert(http::header::CONTENT_TYPE, http::HeaderValue::from_static("application/json"));

        Ok(PreparedRequest {
            url: "http://localhost:0/echo".into(),
            headers,
            body,
            is_streaming: false,
        })
    }
}
```

## 8. Reference implementations

- **Anthropic**: `crates/vkdg-provider-anthropic/src/lib.rs` — `x-api-key` header, `/v1/messages` endpoint, `anthropic-version` header.
- **OpenAI**: `crates/vkdg-provider-openai/src/prepare.rs` — `Authorization: Bearer` header, `/v1/chat/completions`, image and video generation endpoints.
