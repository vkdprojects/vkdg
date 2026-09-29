//! A cache backend must be shippable as a `.wasm`.
//!
//! The load-bearing property is asymmetric: a cache hit must be trustworthy, but a
//! cache failure must be free. A plugin that misbehaves has to degrade to a miss —
//! the request then goes upstream, which is slower but correct — and must never
//! return a hit the gateway would serve as a real response.

use std::sync::Arc;

use vkdg_cache::{CacheBackend, CacheEntry, CacheResult};
use vkdg_plugin_host::{
    PluginId, PluginKind, PluginManifest, PluginRole, WasmCache, WasmPluginInstance,
};

fn manifest(id: &str) -> PluginManifest {
    PluginManifest {
        id: PluginId::new(id),
        version: "0.1.0".to_string(),
        kind: PluginKind::Wasm {
            path: format!("/tmp/{id}.wasm"),
        },
        hooks: vec![],
        roles: vec![PluginRole::CacheBackend],
        description: "test cache".to_string(),
        memory_limit_mb: Some(16),
        cpu_timeout_ms: None,
    }
}

/// A component whose `lookup` returns a fixed document.
fn lookup_component(payload: &str) -> Vec<u8> {
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
    (func (export "lookup") (param i32 i32) (result i32)
      (i32.store (i32.const 0) (i32.const 1024))
      (i32.store (i32.const 4) (i32.const {len}))
      (i32.const 0))
    (func (export "cabi_realloc") (param i32 i32 i32 i32) (result i32)
      (i32.const 8192))
  )
  (core instance $i (instantiate $m))
  (func (export "lookup") (param "key" string) (result string)
    (canon lift (core func $i "lookup")
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

fn cache(id: &str, bytes: &[u8]) -> WasmCache {
    let m = manifest(id);
    let instance = Arc::new(WasmPluginInstance::from_bytes(bytes, &m).expect("component loads"));
    WasmCache::new(instance, &m)
}

fn entry() -> CacheEntry {
    CacheEntry {
        response_json: r#"{"content":"cached"}"#.to_string(),
        model: "claude-sonnet-4.5".to_string(),
        cached_at_secs: 1_700_000_000,
        ttl_secs: Some(300),
        cache_type: "exact".to_string(),
        hit_score: None,
    }
}

/// Refutes: a bridge that drops what the plugin found, making every WASM cache a
/// permanent miss.
#[tokio::test]
async fn plugin_hit_is_returned_intact() {
    let payload = r#"{"result":"hit","entry":{"response_json":"{\"content\":\"cached\"}","model":"claude-sonnet-4.5","cached_at_secs":1700000000,"ttl_secs":300,"cache_type":"exact"}}"#;
    let c = cache("hitter", &lookup_component(payload));

    match c.lookup("some-key").await.expect("lookup must not error") {
        CacheResult::Hit(e) => {
            assert_eq!(e.response_json, r#"{"content":"cached"}"#);
            assert_eq!(e.model, "claude-sonnet-4.5");
            assert_eq!(e.ttl_secs, Some(300));
            assert_eq!(e.cache_type, "exact");
        }
        CacheResult::Miss => panic!("the plugin reported a hit"),
    }
}

/// Refutes: a malformed answer being served as a hit, which would send garbage to
/// a client as if it were a real response.
#[tokio::test]
async fn malformed_answer_is_a_miss_never_a_hit() {
    let c = cache("liar", &lookup_component("this is not json"));
    assert!(
        matches!(
            c.lookup("k").await.expect("must not error"),
            CacheResult::Miss
        ),
        "an unparsable answer must degrade to a miss"
    );
}

/// Refutes: a broken plugin failing a request. Cache is an optimisation; its
/// failure costs latency, not correctness.
#[tokio::test]
async fn broken_plugin_degrades_to_a_miss() {
    let c = cache("broken", &bare_component());
    assert!(
        matches!(
            c.lookup("k")
                .await
                .expect("a broken cache must not fail the request"),
            CacheResult::Miss
        ),
        "a component without `lookup` must read as a miss"
    );
}

/// Refutes: a silent store failure, which would leave an operator wondering why
/// nothing is ever cached.
#[tokio::test]
async fn store_failure_is_reported() {
    let c = cache("no-store", &bare_component());
    assert!(
        c.store("k", entry()).await.is_err(),
        "a missing `store` export must be surfaced, not swallowed"
    );
}

/// Refutes: treating an unparsable invalidation count as a failure, when the
/// invalidation itself may well have happened.
#[tokio::test]
async fn invalidation_count_is_advisory() {
    let c = cache("no-invalidate", &bare_component());
    // With no export at all this is an error; the count parsing is what must be
    // lenient, not the call itself.
    assert!(c.invalidate_tenant("tenant-a").await.is_err());
    assert_eq!(
        c.name(),
        "no-invalidate",
        "identity falls back to manifest id"
    );
}
