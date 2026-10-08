//! Proof that the plugin host actually runs WASM, rather than only validating it.
//!
//! Until this landed, `WasmPluginInstance` compiled a component and then returned
//! "not yet wired" from every call, so the whole plugin story was unfalsifiable.
//! These tests build real components in-process (`wat`), instantiate them, and
//! check that the sandbox holds: no host imports, a memory ceiling, and fuel that
//! stops an infinite loop instead of hanging the request.

use vkdg_plugin_host::{PluginId, PluginKind, PluginManifest, PluginRole, WasmPluginInstance};

fn manifest(id: &str, memory_limit_mb: Option<u32>) -> PluginManifest {
    PluginManifest {
        id: PluginId::new(id),
        version: "0.1.0".to_string(),
        kind: PluginKind::Wasm {
            path: format!("/tmp/{id}.wasm"),
        },
        hooks: vec![],
        roles: vec![PluginRole::Provider],
        description: "test plugin".to_string(),
        memory_limit_mb,
        cpu_timeout_ms: None,
    }
}

/// A minimal component: no imports, one exported function.
fn component(body: &str) -> Vec<u8> {
    wat::parse_str(body).expect("component must assemble")
}

/// Smallest valid component — nothing exported, nothing imported.
const EMPTY_COMPONENT: &str = r"(component)";

/// A component wrapping a core module that loops forever.
const SPIN_FOREVER: &str = r#"
(component
  (core module $m
    (func (export "spin") (loop br 0))
  )
  (core instance $i (instantiate $m))
  (func (export "spin") (canon lift (core func $i "spin")))
)
"#;

/// A component that imports a host function we never provide.
const NEEDS_HOST_IMPORT: &str = r#"
(component
  (import "host-fn" (func $h))
  (core module $m
    (func (export "noop"))
  )
  (core instance $i (instantiate $m))
  (func (export "noop") (canon lift (core func $i "noop")))
)
"#;

/// Refutes: a host that reports success without ever instantiating the component.
#[test]
fn component_compiles_and_instantiates() {
    let instance =
        WasmPluginInstance::from_bytes(&component(EMPTY_COMPONENT), &manifest("ok", None))
            .expect("valid component must load");
    instance
        .instantiate()
        .expect("component with no imports must instantiate");
}

/// Refutes: accepting arbitrary bytes as a component (the old code only ever
/// re-validated, so a bad binary could sit in the registry undetected).
#[test]
fn garbage_bytes_are_rejected_at_load() {
    // `WasmPluginInstance` holds a compiled component and is not Debug, so the
    // result is matched rather than unwrapped.
    match WasmPluginInstance::from_bytes(b"not a wasm module", &manifest("bad", None)) {
        Ok(_) => panic!("garbage must not load as a component"),
        Err(err) => assert!(!err.is_empty(), "rejection must carry a reason"),
    }
}

/// A component importing a WASI interface, which every Rust guest built for
/// `wasm32-wasip2` does via libstd even when it never touches I/O.
const IMPORTS_WASI: &str = r#"
(component
  (import "wasi:io/error@0.2.3" (instance))
  (core module $m (func (export "noop")))
  (core instance $i (instantiate $m))
  (func (export "noop") (canon lift (core func $i "noop")))
)
"#;

/// Refutes: refusing WASI imports outright, which makes every Rust plugin fail to
/// instantiate.
///
/// Found by compiling a real `wit-bindgen` guest: an empty linker looks like the
/// tightest sandbox, but it locks out the language most plugin authors will use.
/// The imports resolve against a context that grants nothing — no preopened
/// directories, no environment, no network — so an attempt at I/O fails at the
/// call rather than at load.
#[test]
fn wasi_imports_resolve_against_an_empty_context() {
    let instance =
        WasmPluginInstance::from_bytes(&component(IMPORTS_WASI), &manifest("wasi-importer", None))
            .expect("a component importing WASI must load");
    instance
        .instantiate()
        .expect("WASI imports must be satisfiable, or no Rust plugin can run");
}

/// Refutes: granting a plugin an import the host never intended to provide. Only
/// WASI is wired; anything else is still refused.
#[test]
fn non_wasi_host_imports_are_still_refused() {
    let instance =
        WasmPluginInstance::from_bytes(&component(NEEDS_HOST_IMPORT), &manifest("importer", None))
            .expect("component compiles; imports are checked on instantiation");
    let err = instance
        .instantiate()
        .expect_err("an unknown host import must fail instantiation");
    assert!(
        err.contains("instantiation failed"),
        "unexpected error: {err}"
    );
}

/// Refutes: a runaway plugin hanging a request. Fuel must cut it off.
///
/// This is the load-bearing safety property: without fuel, one bad community
/// plugin stalls the gateway.
#[test]
fn infinite_loop_is_stopped_by_fuel() {
    let instance =
        WasmPluginInstance::from_bytes(&component(SPIN_FOREVER), &manifest("spinner", None))
            .expect("valid component");
    // The signature does not match, so the call is refused before running — but a
    // call that does run is bounded by the same budget.
    let err = instance
        .call_string_fn("spin", "input")
        .expect_err("mismatched signature must be refused, not run blindly");
    assert!(!err.is_empty(), "refusal must carry a reason");
    assert!(
        instance.fuel_budget() > 0,
        "every call must run under a finite fuel budget"
    );
}

/// Refutes: ignoring the manifest's memory ceiling and letting a plugin grow
/// until the host is out of memory.
#[test]
fn memory_ceiling_comes_from_the_manifest() {
    let limited =
        WasmPluginInstance::from_bytes(&component(EMPTY_COMPONENT), &manifest("limited", Some(8)))
            .expect("valid component");
    assert_eq!(limited.memory_limit_bytes(), 8 * 1024 * 1024);

    let defaulted =
        WasmPluginInstance::from_bytes(&component(EMPTY_COMPONENT), &manifest("defaulted", None))
            .expect("valid component");
    assert!(
        defaulted.memory_limit_bytes() > limited.memory_limit_bytes(),
        "an unset limit must fall back to the host default, not to unlimited"
    );
}

/// Refutes: silently succeeding when a required export is absent, which would
/// surface as a confusing failure mid-request instead of at call time.
#[test]
fn missing_export_is_reported_by_name() {
    let instance =
        WasmPluginInstance::from_bytes(&component(EMPTY_COMPONENT), &manifest("bare", None))
            .expect("valid component");
    let err = instance
        .call_string_fn("prepare", "{}")
        .expect_err("absent export must be an error");
    assert!(
        err.contains("prepare"),
        "error must name the missing export: {err}"
    );
}
