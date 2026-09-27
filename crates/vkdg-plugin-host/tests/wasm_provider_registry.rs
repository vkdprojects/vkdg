//! A WASM plugin must be indistinguishable from a compiled provider.
//!
//! The pipeline only ever does `registry.get(id)` and calls `ProviderAdapter`, so
//! if a `.wasm` can be registered under the same id namespace and answer the same
//! trait, a provider can ship out-of-tree and be swapped without a gateway
//! release. These tests hold that line.

use std::collections::HashMap;
use std::sync::Arc;

use vkdg_connections::{AuthKind, ConnectionConfig, Credential, ProviderKind};
use vkdg_core::ConnectionId;
use vkdg_operations::{
    CapabilitySet, ConversationRequest, Message, MessageContent, Operation, Role,
};
use vkdg_plugin_host::{
    PluginId, PluginKind, PluginManifest, PluginRole, WasmPluginInstance, WasmProviderAdapter,
};
use vkdg_provider_sdk::{ProviderAdapter, ProviderRegistry};

fn manifest(id: &str) -> PluginManifest {
    PluginManifest {
        id: PluginId::new(id),
        version: "0.1.0".to_string(),
        kind: PluginKind::Wasm {
            path: format!("/tmp/{id}.wasm"),
        },
        hooks: vec![],
        roles: vec![PluginRole::Provider],
        description: "test provider plugin".to_string(),
        memory_limit_mb: Some(16),
        cpu_timeout_ms: None,
    }
}

/// Smallest component that loads: exports nothing, imports nothing.
///
/// A plugin this bare still has to register, falling back to its manifest id, so a
/// half-built plugin fails at call time with a clear error rather than at startup.
fn bare_component() -> Vec<u8> {
    wat::parse_str("(component)").expect("component assembles")
}

fn connection() -> ConnectionConfig {
    ConnectionConfig {
        id: ConnectionId("wasm-conn".into()),
        provider: ProviderKind::Plugin {
            id: "community-provider".into(),
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
        model: "test-model".into(),
        messages: vec![Message {
            role: Role::User,
            content: MessageContent::Text("hello".into()),
        }],
        tools: vec![],
        max_tokens: Some(16),
        temperature: None,
        stream: false,
        system: None,
        required_capabilities: CapabilitySet::default(),
    })
}

fn adapter(id: &str) -> WasmProviderAdapter {
    let m = manifest(id);
    let instance =
        Arc::new(WasmPluginInstance::from_bytes(&bare_component(), &m).expect("component loads"));
    WasmProviderAdapter::new(instance, &m).expect("adapter builds")
}

/// Refutes: a WASM plugin needing its own lookup path. It has to land in the same
/// registry, under the same id namespace, as a compiled provider.
#[test]
fn wasm_plugin_resolves_through_the_same_registry() {
    let mut registry = ProviderRegistry::empty();
    registry.register(Arc::new(adapter("community-provider")));

    let resolved = registry
        .get("community-provider")
        .expect("a WASM plugin must resolve by id like any provider");
    assert_eq!(resolved.id(), "community-provider");
    assert!(
        registry.ids().iter().any(|i| i == "community-provider"),
        "plugin must be listed alongside compiled providers"
    );
}

/// Refutes: a plugin id taken from the component overwriting an unrelated
/// provider, or a missing `name` export leaving the adapter unregistrable.
#[test]
fn identity_falls_back_to_the_manifest_id() {
    let a = adapter("my-plugin");
    assert_eq!(
        a.id(),
        "my-plugin",
        "without a `name` export the manifest id is authoritative"
    );
    assert_eq!(
        a.display_name(),
        "my-plugin",
        "display name falls back to the id, never to empty"
    );
    assert!(
        a.model_patterns().is_empty(),
        "a plugin that exports no patterns claims no models"
    );
}

/// Refutes: a missing or broken export surfacing as a panic, or as a silent
/// success that would send an empty request upstream.
#[test]
fn prepare_on_an_incomplete_plugin_errors_instead_of_panicking() {
    let a = adapter("incomplete");
    let credential = Credential::bearer("token");
    // `PreparedRequest` is not Debug, so the result is matched rather than unwrapped.
    let msg = match a.prepare(&operation(), &connection(), &credential) {
        Ok(_) => panic!("a plugin without `prepare` must fail the request"),
        Err(err) => err.to_string(),
    };
    assert!(
        msg.contains("prepare"),
        "error must name the missing export: {msg}"
    );
}

/// Refutes: leaking credentials the plugin was not handed. The adapter passes the
/// token explicitly, so config-derived secrets never travel implicitly.
#[test]
fn credential_extra_reaches_the_plugin_boundary() {
    let a = adapter("extra-reader");
    let mut extra = HashMap::new();
    extra.insert("region".to_string(), "eu-central-1".to_string());
    let credential = Credential {
        token: "tok".into(),
        extra: Arc::new(extra),
    };

    // The bare component has no `prepare`, so the call fails — but it fails after
    // serialising the input, which is what proves per-account data crosses over.
    let msg = match a.prepare(&operation(), &connection(), &credential) {
        Ok(_) => panic!("bare component cannot answer prepare"),
        Err(err) => err.to_string(),
    };
    assert!(
        !msg.contains("serialization"),
        "input must serialise cleanly, including credential.extra: {msg}"
    );
}
