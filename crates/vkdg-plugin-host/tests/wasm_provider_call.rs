//! End-to-end proof that a WASM plugin can actually serve a request.
//!
//! The previous tests showed a plugin *registers* like a compiled provider. This
//! one calls across the boundary and checks the value that comes back is used: the
//! plugin picks the upstream URL, the headers and the body, and the host turns that
//! into a `PreparedRequest` the pipeline can send.
//!
//! The component is hand-written WAT rather than a compiled guest so the test has
//! no toolchain prerequisites; it returns a fixed JSON document from a known
//! address in its own linear memory, which is enough to exercise the full
//! host-to-guest string ABI.

use std::sync::Arc;

use vkdg_connections::{AuthKind, ConnectionConfig, Credential, ProviderKind};
use vkdg_core::ConnectionId;
use vkdg_operations::{
    CapabilitySet, ConversationRequest, Message, MessageContent, Operation, Role,
};
use vkdg_plugin_host::{
    PluginId, PluginKind, PluginManifest, PluginRole, WasmPluginInstance, WasmProviderAdapter,
};
use vkdg_provider_sdk::ProviderAdapter;

/// JSON the plugin returns from `prepare`, matching `prepared-request` in
/// `wit/provider.wit`. Kept on one line so its byte length is easy to encode.
const PREPARED_JSON: &str = r#"{"url":"https://plugin.example/v1/chat","headers":[["x-plugin","1"]],"body":[123,125],"is_streaming":true}"#;

/// A component exporting `prepare: func(string) -> string`.
///
/// The guest writes `PREPARED_JSON` into memory at offset 1024 during
/// instantiation, then returns a (ptr, len) pair pointing at it. That is exactly
/// what the canonical ABI expects for a returned string.
fn prepare_component() -> Vec<u8> {
    let json = PREPARED_JSON;
    let len = json.len();
    // Escape nothing: the JSON contains `"` which WAT strings accept as \22.
    let escaped: String = json
        .chars()
        .map(|c| match c {
            '"' => "\\22".to_string(),
            '\\' => "\\5c".to_string(),
            c => c.to_string(),
        })
        .collect();

    let wat = format!(
        r#"
(component
  (core module $m
    (memory (export "mem") 1)
    ;; The returned string lives at 1024; the (ptr, len) pair goes at 0.
    (data (i32.const 1024) "{escaped}")
    (func (export "prepare") (param i32 i32) (result i32)
      (i32.store (i32.const 0) (i32.const 1024))
      (i32.store (i32.const 4) (i32.const {len}))
      (i32.const 0))
    (func (export "cabi_realloc") (param i32 i32 i32 i32) (result i32)
      ;; Scratch space for host-provided arguments, well clear of the data above.
      (i32.const 4096))
  )
  (core instance $i (instantiate $m))
  (func (export "prepare") (param "input" string) (result string)
    (canon lift (core func $i "prepare")
      (memory (core memory $i "mem"))
      (realloc (core func $i "cabi_realloc"))
      string-encoding=utf8))
)
"#
    );
    wat::parse_str(&wat).expect("component assembles")
}

fn manifest(id: &str) -> PluginManifest {
    PluginManifest {
        id: PluginId::new(id),
        version: "0.1.0".to_string(),
        kind: PluginKind::Wasm {
            path: format!("/tmp/{id}.wasm"),
        },
        hooks: vec![],
        roles: vec![PluginRole::Provider],
        description: "prepare-capable test plugin".to_string(),
        memory_limit_mb: Some(16),
        cpu_timeout_ms: None,
    }
}

fn connection() -> ConnectionConfig {
    ConnectionConfig {
        id: ConnectionId("wasm-conn".into()),
        provider: ProviderKind::Plugin {
            id: "wasm-echo".into(),
        },
        auth: AuthKind::ApiKey {
            env_var: "UNUSED".into(),
        },
        models: vec!["custom-*".into()],
        max_concurrent: 4,
        weight: 1,
        tags: vec![],
        capabilities: CapabilitySet::default(),
    }
}

fn operation() -> Operation {
    Operation::Conversation(ConversationRequest {
        messages: vec![Message {
            role: Role::User,
            content: MessageContent::Text("hello".into()),
        }],
        tools: vec![],
        max_tokens: Some(16),
        temperature: None,
        stream: true,
        system: None,
        required_capabilities: CapabilitySet::default(),
    })
}

/// Refutes: a host that registers plugins but never runs them, which is what the
/// previous implementation did (every call returned "not yet wired").
#[test]
fn plugin_decides_the_upstream_request() {
    let m = manifest("wasm-echo");
    let instance = Arc::new(
        WasmPluginInstance::from_bytes(&prepare_component(), &m).expect("component loads"),
    );
    let adapter = WasmProviderAdapter::new(instance, &m).expect("adapter builds");

    let prepared = match adapter.prepare(&operation(), &connection(), &Credential::bearer("tok")) {
        Ok(p) => p,
        Err(e) => panic!("plugin must answer prepare: {e}"),
    };

    assert_eq!(
        prepared.url, "https://plugin.example/v1/chat",
        "the plugin chooses the upstream URL, not the host"
    );
    assert_eq!(
        prepared
            .headers
            .get("x-plugin")
            .map(|v| v.to_str().unwrap()),
        Some("1"),
        "plugin headers must reach the upstream request"
    );
    assert_eq!(
        prepared.body.as_ref(),
        b"{}",
        "plugin body bytes must pass through unmodified"
    );
    assert!(
        prepared.is_streaming,
        "the plugin decides whether the call streams"
    );
}
