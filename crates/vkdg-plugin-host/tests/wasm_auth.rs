//! Auth plugins must fail closed.
//!
//! This is the one role where a broken plugin is a security problem rather than a
//! performance one. Every other bridge degrades quietly; here, anything the plugin
//! cannot answer clearly has to become a denial. A bridge that let an unloadable
//! component through would turn a bad install into an open gateway.

use std::sync::Arc;

use vkdg_plugin_host::{
    AuthContext, AuthOutcome, AuthPlugin, PluginId, PluginKind, PluginManifest, PluginRole,
    RateLimitOutcome, WasmAuth, WasmPluginInstance,
};

fn manifest(id: &str) -> PluginManifest {
    PluginManifest {
        id: PluginId::new(id),
        version: "0.1.0".to_string(),
        kind: PluginKind::Wasm {
            path: format!("/tmp/{id}.wasm"),
        },
        hooks: vec![],
        roles: vec![PluginRole::Auth],
        description: "test auth".to_string(),
        memory_limit_mb: Some(16),
        cpu_timeout_ms: None,
    }
}

/// A component exporting one string→string function returning a fixed document.
fn component(export_name: &str, payload: &str) -> Vec<u8> {
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
    (func (export "f") (param i32 i32) (result i32)
      (i32.store (i32.const 0) (i32.const 1024))
      (i32.store (i32.const 4) (i32.const {len}))
      (i32.const 0))
    (func (export "cabi_realloc") (param i32 i32 i32 i32) (result i32)
      (i32.const 8192))
  )
  (core instance $i (instantiate $m))
  (func (export "{export_name}") (param "input" string) (result string)
    (canon lift (core func $i "f")
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

fn auth(id: &str, bytes: &[u8]) -> WasmAuth {
    let m = manifest(id);
    let instance = Arc::new(WasmPluginInstance::from_bytes(bytes, &m).expect("component loads"));
    WasmAuth::new(instance, &m)
}

fn context() -> AuthContext {
    AuthContext {
        tenant_id: "tenant-a".to_string(),
        key_id: "key-1".to_string(),
        role: "member".to_string(),
        scopes: vec!["data:inference".to_string()],
        claims: Default::default(),
    }
}

/// Refutes: a bridge that drops the plugin's verdict, so authentication never
/// actually succeeds.
#[test]
fn plugin_approval_is_honoured() {
    let payload = r#"{"result":"allowed","context":{"tenant_id":"acme","key_id":"k-7","role":"admin","scopes":["data:inference","data:embeddings"]}}"#;
    let a = auth("approver", &component("authenticate", payload));

    match a.authenticate("{}") {
        AuthOutcome::Allowed(ctx) => {
            assert_eq!(ctx.tenant_id, "acme");
            assert_eq!(ctx.key_id, "k-7");
            assert_eq!(ctx.role, "admin");
            assert_eq!(ctx.scopes.len(), 2);
        }
        AuthOutcome::Denied(r) => panic!("the plugin approved this request: {r}"),
    }
}

/// Refutes: the most dangerous failure mode — an unloadable auth plugin letting
/// traffic through because "the plugin did not say no".
#[test]
fn broken_plugin_denies_rather_than_allowing() {
    let a = auth("broken", &bare_component());
    match a.authenticate("{}") {
        AuthOutcome::Denied(reason) => assert!(
            reason.contains("broken"),
            "denial must name the plugin: {reason}"
        ),
        AuthOutcome::Allowed(_) => {
            panic!("a plugin that cannot run must never authenticate a request")
        }
    }
}

/// Refutes: an unreadable verdict being read as approval.
#[test]
fn unreadable_verdict_denies() {
    let a = auth("garbled", &component("authenticate", "not json at all"));
    assert!(
        matches!(a.authenticate("{}"), AuthOutcome::Denied(_)),
        "an answer the gateway cannot parse is not an approval"
    );
}

/// Refutes: an explicit denial being reported without its reason, which leaves an
/// operator with nothing to debug.
#[test]
fn explicit_denial_keeps_its_reason() {
    let payload = r#"{"result":"denied","reason":"key revoked"}"#;
    let a = auth("denier", &component("authenticate", payload));
    match a.authenticate("{}") {
        AuthOutcome::Denied(reason) => assert_eq!(reason, "key revoked"),
        AuthOutcome::Allowed(_) => panic!("the plugin denied this request"),
    }
}

/// Refutes: a rate limiter that fails open, which would defeat the limit it exists
/// to enforce.
#[test]
fn rate_limiter_that_cannot_answer_denies() {
    let a = auth("no-limiter", &bare_component());
    assert!(
        matches!(
            a.check_rate_limit(&context(), "{}"),
            RateLimitOutcome::Denied(_)
        ),
        "a limiter that cannot run must not allow traffic"
    );
}

/// Refutes: losing the retry hint, so a client is throttled with no idea when to
/// come back.
#[test]
fn retry_hint_survives_the_boundary() {
    let limited = r#"{"result":"limited","retry_after_secs":30}"#;
    let a = auth("limiter", &component("check-rate-limit", limited));
    match a.check_rate_limit(&context(), "{}") {
        RateLimitOutcome::Limited { retry_after_secs } => {
            assert_eq!(retry_after_secs, Some(30));
        }
        other => panic!("expected a limit with a hint, got {other:?}"),
    }

    let allowed = r#"{"result":"allowed"}"#;
    let a = auth("allower", &component("check-rate-limit", allowed));
    assert!(matches!(
        a.check_rate_limit(&context(), "{}"),
        RateLimitOutcome::Allowed
    ));
}

/// Refutes: usage recording being able to affect a response that was already sent.
#[test]
fn usage_recording_never_fails_the_caller() {
    let a = auth("no-usage", &bare_component());
    // No export, no panic, no error: the response has already gone out.
    a.record_usage(&context(), 100, 200);
}
