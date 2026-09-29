//! A routing strategy must be shippable as a `.wasm`.
//!
//! Which account serves a request is a business decision — cost, quota, tenancy —
//! that no built-in strategy can anticipate. These tests check a plugin's ordering
//! is honoured, and that the two safety properties hold: a plugin cannot route to a
//! connection the gateway excluded, and a broken plugin degrades to the gateway's
//! own order instead of failing the request.

use std::sync::Arc;

use vkdg_core::{ApiType, ClientId, ConnectionId, RequestEnvelope, RequestId, TenantId};
use vkdg_plugin_host::{
    PluginId, PluginKind, PluginManifest, PluginRole, WasmPluginInstance, WasmRouter,
};
use vkdg_routing::{EligibilityFilter, RoutingHints, Strategy};

fn manifest(id: &str) -> PluginManifest {
    PluginManifest {
        id: PluginId::new(id),
        version: "0.1.0".to_string(),
        kind: PluginKind::Wasm {
            path: format!("/tmp/{id}.wasm"),
        },
        hooks: vec![],
        roles: vec![PluginRole::Router],
        description: "test router".to_string(),
        memory_limit_mb: Some(16),
        cpu_timeout_ms: None,
    }
}

/// A component whose `route` returns a fixed candidate order.
fn router_component(payload: &str) -> Vec<u8> {
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
    (func (export "route") (param i32 i32) (result i32)
      (i32.store (i32.const 0) (i32.const 1024))
      (i32.store (i32.const 4) (i32.const {len}))
      (i32.const 0))
    (func (export "cabi_realloc") (param i32 i32 i32 i32) (result i32)
      (i32.const 8192))
  )
  (core instance $i (instantiate $m))
  (func (export "route") (param "input" string) (result string)
    (canon lift (core func $i "route")
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

fn router(id: &str, bytes: &[u8]) -> WasmRouter {
    let m = manifest(id);
    let instance = Arc::new(WasmPluginInstance::from_bytes(bytes, &m).expect("component loads"));
    WasmRouter::new(instance, &m)
}

fn envelope() -> RequestEnvelope {
    RequestEnvelope {
        request_id: RequestId::new(),
        client_id: ClientId("router-test".into()),
        tenant_id: TenantId("default".into()),
        session_key: None,
        api_type: ApiType::AnthropicMessages,
        model_requested: "claude-sonnet-4.5".into(),
        deadline: None,
        mode_pack_override: None,
        compression_override: None,
        cache_bypass: false,
        include_think_tags: false,
        client_ip: None,
    }
}

fn ids(names: &[&str]) -> Vec<ConnectionId> {
    names.iter().map(|n| ConnectionId((*n).into())).collect()
}

/// Refutes: a bridge that ignores the plugin and always takes the first candidate,
/// which would make every WASM router a no-op.
#[tokio::test]
async fn plugin_order_is_honoured() {
    // The plugin prefers the third candidate.
    let r = router(
        "prefer-c",
        &router_component(r#"{"candidates":["conn-c","conn-a"]}"#),
    );
    let chosen = r
        .select(
            &ids(&["conn-a", "conn-b", "conn-c"]),
            &envelope(),
            &EligibilityFilter::default(),
            &RoutingHints::default(),
        )
        .await
        .expect("a router with candidates must choose one");
    assert_eq!(chosen.0, "conn-c", "the plugin's first preference must win");
}

/// Refutes: a plugin escaping eligibility filtering, which would let it route to a
/// connection the gateway ruled out (rate-limited, unhealthy, wrong tenant).
#[tokio::test]
async fn plugin_cannot_select_an_excluded_connection() {
    let r = router(
        "prefer-excluded",
        &router_component(r#"{"candidates":["conn-bad","conn-good"]}"#),
    );
    let filter = EligibilityFilter {
        excluded_connections: vec![ConnectionId("conn-bad".into())],
        ..Default::default()
    };

    let chosen = r
        .select(
            &ids(&["conn-bad", "conn-good"]),
            &envelope(),
            &filter,
            &RoutingHints::default(),
        )
        .await
        .expect("an eligible candidate remains");
    assert_eq!(
        chosen.0, "conn-good",
        "an excluded connection must not be selectable, even if the plugin asks"
    );
}

/// Refutes: a plugin inventing a connection id the gateway never offered.
#[tokio::test]
async fn unknown_ids_from_the_plugin_are_ignored() {
    let r = router(
        "hallucinating",
        &router_component(r#"{"candidates":["conn-does-not-exist"]}"#),
    );
    let chosen = r
        .select(
            &ids(&["conn-a", "conn-b"]),
            &envelope(),
            &EligibilityFilter::default(),
            &RoutingHints::default(),
        )
        .await
        .expect("must still choose from the real candidates");
    assert_eq!(
        chosen.0, "conn-a",
        "an unusable plugin answer falls back to the gateway's order"
    );
}

/// Refutes: a broken plugin failing the request rather than degrading to default
/// routing.
#[tokio::test]
async fn broken_plugin_falls_back_to_gateway_order() {
    let r = router("broken", &bare_component());
    let chosen = r
        .select(
            &ids(&["conn-a", "conn-b"]),
            &envelope(),
            &EligibilityFilter::default(),
            &RoutingHints::default(),
        )
        .await
        .expect("a broken router must not fail the request");
    assert_eq!(chosen.0, "conn-a");
    assert_eq!(r.name(), "broken", "identity falls back to the manifest id");
}

/// Refutes: reporting a choice when nothing is eligible, which would dispatch to a
/// connection that cannot serve the request.
#[tokio::test]
async fn no_eligible_candidates_is_an_error() {
    let r = router("any", &router_component(r#"{"candidates":["conn-a"]}"#));
    let filter = EligibilityFilter {
        excluded_connections: vec![ConnectionId("conn-a".into())],
        ..Default::default()
    };

    assert!(
        r.select(
            &ids(&["conn-a"]),
            &envelope(),
            &filter,
            &RoutingHints::default()
        )
        .await
        .is_err(),
        "with every candidate excluded there is nothing to choose"
    );
}
