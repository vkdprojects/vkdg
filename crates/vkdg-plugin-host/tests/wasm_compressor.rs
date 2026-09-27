//! A compression strategy must be shippable as a `.wasm`.
//!
//! Compression is where users most want their own heuristic: which lines of a build
//! log matter is project-specific. These tests check the bridge honours a plugin's
//! output, and — more importantly — that a broken plugin costs nothing, because
//! compression is an optimisation and must never fail a user's request.

use std::sync::Arc;

use vkdg_operations::{CapabilitySet, ConversationRequest, Message, MessageContent, Role};
use vkdg_plugin_host::{
    PluginId, PluginKind, PluginManifest, PluginRole, WasmCompressor, WasmPluginInstance,
};
use vkdg_policy_compress::{CompressionError, Compressor};

/// What the plugin returns from `compress`: two messages collapsed into one.
const COMPRESSED_JSON: &str = r#"{"messages":[{"role":"user","content":"short"}],"report":{"tokens_before":100,"tokens_after":10,"algorithm":"test-squeeze","lossy":true}}"#;

fn manifest(id: &str) -> PluginManifest {
    PluginManifest {
        id: PluginId::new(id),
        version: "0.1.0".to_string(),
        kind: PluginKind::Wasm {
            path: format!("/tmp/{id}.wasm"),
        },
        hooks: vec![],
        roles: vec![PluginRole::Compressor],
        description: "test compressor".to_string(),
        memory_limit_mb: Some(16),
        cpu_timeout_ms: None,
    }
}

/// A component exporting `compress: func(string) -> string` returning a constant.
fn compressor_component(payload: &str) -> Vec<u8> {
    let escaped: String = payload
        .chars()
        .map(|c| match c {
            '"' => "\\22".to_string(),
            '\\' => "\\5c".to_string(),
            c => c.to_string(),
        })
        .collect();
    let len = payload.len();
    let wat = format!(
        r#"
(component
  (core module $m
    (memory (export "mem") 1)
    (data (i32.const 1024) "{escaped}")
    (func (export "compress") (param i32 i32) (result i32)
      (i32.store (i32.const 0) (i32.const 1024))
      (i32.store (i32.const 4) (i32.const {len}))
      (i32.const 0))
    (func (export "cabi_realloc") (param i32 i32 i32 i32) (result i32)
      (i32.const 8192))
  )
  (core instance $i (instantiate $m))
  (func (export "compress") (param "input" string) (result string)
    (canon lift (core func $i "compress")
      (memory (core memory $i "mem"))
      (realloc (core func $i "cabi_realloc"))
      string-encoding=utf8))
)
"#
    );
    wat::parse_str(&wat).expect("component assembles")
}

fn bare_component() -> Vec<u8> {
    wat::parse_str("(component)").expect("component assembles")
}

fn request() -> ConversationRequest {
    ConversationRequest {
        messages: vec![
            Message {
                role: Role::User,
                content: MessageContent::Text("a very long first message".repeat(4)),
            },
            Message {
                role: Role::Assistant,
                content: MessageContent::Text("a very long reply".repeat(4)),
            },
        ],
        tools: vec![],
        max_tokens: Some(256),
        temperature: None,
        stream: false,
        system: Some("you are helpful".into()),
        required_capabilities: CapabilitySet::default(),
    }
}

fn compressor(id: &str, bytes: &[u8]) -> WasmCompressor {
    let m = manifest(id);
    let instance = Arc::new(WasmPluginInstance::from_bytes(bytes, &m).expect("component loads"));
    WasmCompressor::new(instance, &m)
}

/// Refutes: a bridge that ignores what the plugin returned and hands back the
/// original request, which would make every WASM compressor a no-op.
#[test]
fn plugin_output_replaces_the_messages() {
    let c = compressor("squeeze", &compressor_component(COMPRESSED_JSON));
    let (compressed, metrics) = c.compress(request(), 50).expect("plugin answers compress");

    assert_eq!(
        compressed.messages.len(),
        1,
        "the plugin collapsed two messages into one"
    );
    assert!(matches!(
        &compressed.messages[0].content,
        MessageContent::Text(t) if t == "short"
    ));
    // The system prompt was not returned, so it must survive untouched.
    assert_eq!(compressed.system.as_deref(), Some("you are helpful"));

    assert_eq!(metrics.original_message_count, 2);
    assert_eq!(metrics.compressed_message_count, 1);
    assert_eq!(metrics.estimated_tokens_removed, 90);
    assert_eq!(metrics.strategy, "test-squeeze");
    assert!(!metrics.lossless, "the plugin reported a lossy transform");
}

/// Refutes: a broken plugin failing the request. Compression is optional, so a
/// plugin that cannot run must degrade to "not applicable".
#[test]
fn broken_plugin_is_skipped_not_fatal() {
    let c = compressor("broken", &bare_component());
    match c.compress(request(), 50) {
        Err(CompressionError::NotApplicable) => {}
        Ok(_) => panic!("a component without `compress` cannot have compressed anything"),
        Err(e) => panic!("a broken plugin must be skippable, got {e}"),
    }
}

/// Refutes: a plugin that cannot estimate disabling itself by reporting zero,
/// which would stop the gateway from ever invoking compression.
#[test]
fn missing_estimate_falls_back_instead_of_reporting_zero() {
    let c = compressor("no-estimate", &bare_component());
    let estimate = c.estimate_tokens(&request());
    assert!(
        estimate > 0,
        "a fallback estimate must be non-zero for a long request, got {estimate}"
    );
}

/// Refutes: losing the plugin's identity, which the pipeline logs and reports as
/// the compression strategy.
#[test]
fn name_falls_back_to_the_manifest_id() {
    let c = compressor("my-compressor", &bare_component());
    assert_eq!(c.name(), "my-compressor");
}
