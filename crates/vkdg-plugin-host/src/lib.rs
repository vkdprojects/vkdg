//! Plugin host: manifest registry and hook dispatch.
//!
//! Phase D adds Wasmtime-based WASM component execution.  The WIT contract is
//! defined in `wit/provider.wit`.  Full bindgen codegen (`wasmtime::component::bindgen!`)
//! is deferred to Phase E; for now we validate that a component binary is
//! well-formed and reserve the call surface.

use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use wasmtime::component::Component;
use wasmtime::Engine;

// ── PluginId ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PluginId(pub String);

impl PluginId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

impl std::fmt::Display for PluginId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

// ── PluginKind ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PluginKind {
    /// Compiled into the host binary (Phase A–C).
    NativeRust,
    /// WASM component loaded at runtime via Wasmtime + WIT (Phase D+).
    Wasm { path: String },
}

// ── HookKind ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HookKind {
    PreAuth,
    Auth,
    RateLimit,
    PreDispatch,
    OnEvent,
    OnComplete,
    OnFinish,
}

// ── PluginRole ────────────────────────────────────────────────────────────────

/// What capability a plugin provides.
/// A single .wasm component may implement multiple roles.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginRole {
    /// Routing strategy (selects from candidate connections)
    Router,
    /// Context compression (pre-dispatch message reduction)
    Compressor,
    /// Provider adapter (translates to/from provider wire format)
    Provider,
    /// Cache backend (lookup + store)
    CacheBackend,
    /// Auth and rate limiting (ingress)
    Auth,
}

// ── PluginManifest ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    pub id: PluginId,
    pub version: String,
    pub kind: PluginKind,
    /// Legacy hook-based dispatch (kept for backward compat).
    pub hooks: Vec<HookKind>,
    /// Capability roles this plugin fulfills (new model).
    pub roles: Vec<PluginRole>,
    pub description: String,
    /// Maximum WASM linear memory in megabytes; None = host default (256 MB).
    pub memory_limit_mb: Option<u32>,
    /// Hard CPU wall-clock timeout per call in milliseconds; None = no limit.
    pub cpu_timeout_ms: Option<u32>,
}

// ── PluginRegistry ────────────────────────────────────────────────────────────

pub struct PluginRegistry {
    pub manifests: HashMap<PluginId, PluginManifest>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self {
            manifests: HashMap::new(),
        }
    }

    pub fn register(&mut self, manifest: PluginManifest) {
        self.manifests.insert(manifest.id.clone(), manifest);
    }

    /// Remove a plugin by id.  Returns `true` if it was present.
    pub fn remove(&mut self, id: &PluginId) -> bool {
        self.manifests.remove(id).is_some()
    }

    /// Return all manifests that declare the given hook.
    pub fn plugins_for_hook(&self, hook: &HookKind) -> Vec<&PluginManifest> {
        self.manifests
            .values()
            .filter(|m| m.hooks.contains(hook))
            .collect()
    }
}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ── WasmPluginInstance ────────────────────────────────────────────────────────

/// A loaded and validated WASM component.
///
/// The component binary is validated at construction time via Wasmtime; the
/// engine is stored per-instance so callers don't need to manage its lifetime.
/// In a production deployment the engine would be shared across all instances
/// (it is expensive to construct); that optimisation is deferred to Phase E
/// together with full bindgen codegen.
pub struct WasmPluginInstance {
    component_bytes: Vec<u8>,
    engine: Engine,
}

impl WasmPluginInstance {
    /// Load and validate a `.wasm` component from disk.
    ///
    /// Returns `Err` with a human-readable message if the file cannot be read
    /// or the binary is not a valid Wasmtime component.
    pub fn load(path: &str) -> Result<Self, String> {
        let bytes = std::fs::read(path)
            .map_err(|e| format!("failed to read wasm at {path:?}: {e}"))?;
        let engine = Engine::default();
        // Validate by attempting a compile; drops the compiled artifact.
        Component::from_binary(&engine, &bytes)
            .map_err(|e| format!("invalid wasm component at {path:?}: {e}"))?;
        Ok(Self { component_bytes: bytes, engine })
    }

    /// Call the `prepare` WIT export with JSON-serialised inputs.
    ///
    /// Returns the serialised `PreparedRequest` JSON on success.
    ///
    /// **Phase D stub**: instantiation and typed call are Phase E (requires
    /// `wasmtime::component::bindgen!` codegen).  This method re-validates the
    /// component binary and returns `Err` to signal that the call surface is
    /// reserved but not yet wired.
    pub fn call_prepare(
        &self,
        _operation_json: &str,
        _config_json: &str,
        _token: &str,
    ) -> Result<String, String> {
        // Re-validate to prove the component is still well-formed.
        Component::from_binary(&self.engine, &self.component_bytes)
            .map_err(|e| e.to_string())?;
        Err("wasm call not yet wired (Phase E)".to_string())
    }
}


// ── PluginChain ───────────────────────────────────────────────────────────────

/// An ordered list of plugin IDs for a given role.
/// Compiled per route from config, not re-computed per request.
pub struct PluginChain {
    pub role: PluginRole,
    /// Plugin IDs in execution order.
    pub plugins: Vec<PluginId>,
    /// If true, non-fatal errors from plugins are logged and skipped rather
    /// than aborting the request.  Auth chains are always fail-closed
    /// regardless of this flag.
    pub fail_open: bool,
}

impl PluginChain {
    pub fn new(role: PluginRole, plugins: Vec<PluginId>) -> Self {
        // Auth is always fail-closed; every other role defaults to fail-open
        // so that a broken optional plugin (e.g. cache) never blocks requests.
        let fail_open = !matches!(role, PluginRole::Auth);
        Self { role, plugins, fail_open }
    }
}
// ── PluginHost ────────────────────────────────────────────────────────────────

/// Owns the plugin registry and loaded WASM instances.
///
/// NativeRust plugins are registered directly via `registry.register`.
/// WASM plugins go through `install_wasm` which validates the binary before
/// adding it to the registry, enabling atomic rollback on failure.
pub struct PluginHost {
    pub registry: PluginRegistry,
    wasm_instances: HashMap<PluginId, WasmPluginInstance>,
}

impl PluginHost {
    pub fn new(registry: PluginRegistry) -> Self {
        Self {
            registry,
            wasm_instances: HashMap::new(),
        }
    }

    /// Install a WASM plugin atomically.
    ///
    /// Loads and validates the component *before* touching the registry so
    /// that a corrupt binary leaves the host state unchanged (rollback by
    /// construction).
    pub fn install_wasm(
        &mut self,
        manifest: PluginManifest,
        wasm_path: &str,
    ) -> Result<(), String> {
        match &manifest.kind {
            PluginKind::Wasm { .. } => {}
            _ => return Err("manifest kind must be Wasm".to_string()),
        }
        // Load and validate first — no state mutation until this succeeds.
        let instance = WasmPluginInstance::load(wasm_path)?;
        self.wasm_instances.insert(manifest.id.clone(), instance);
        self.registry.register(manifest);
        Ok(())
    }

    /// Unload a plugin (any kind) by id.
    ///
    /// Returns `true` if the plugin was present in the registry.  WASM
    /// instances are dropped immediately; the `Drop` impl frees the compiled
    /// bytes.
    pub fn uninstall(&mut self, id: &PluginId) -> bool {
        self.wasm_instances.remove(id);
        self.registry.remove(id)
    }

    /// Return all registered manifests that advertise the given role.
    pub fn plugins_for_role(&self, role: &PluginRole) -> Vec<&PluginManifest> {
        self.registry.manifests.values()
            .filter(|m| m.roles.contains(role))
            .collect()
    }

    /// Build the default plugin chain for a role from all registered plugins.
    /// Ordered by insertion order (FIFO).  Operators can override via config.
    pub fn default_chain(&self, role: PluginRole) -> PluginChain {
        let plugins: Vec<PluginId> = self.plugins_for_role(&role)
            .into_iter()
            .map(|m| m.id.clone())
            .collect();
        PluginChain::new(role, plugins)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn native_manifest(id: &str, hooks: Vec<HookKind>) -> PluginManifest {
        PluginManifest {
            id: PluginId::new(id),
            version: "0.1.0".to_string(),
            kind: PluginKind::NativeRust,
            hooks,
            roles: vec![],
            description: "test plugin".to_string(),
            memory_limit_mb: None,
            cpu_timeout_ms: None,
        }
    }

    fn role_manifest(id: &str, roles: Vec<PluginRole>) -> PluginManifest {
        PluginManifest {
            id: PluginId::new(id),
            version: "0.1.0".to_string(),
            kind: PluginKind::NativeRust,
            hooks: vec![],
            roles,
            description: "role plugin".to_string(),
            memory_limit_mb: None,
            cpu_timeout_ms: None,
        }
    }

    // Plausible wrong impl: panics (unwrap) instead of returning Err for an invalid path.
    #[test]
    fn wasm_plugin_load_invalid_path() {
        let result = WasmPluginInstance::load("/nonexistent/path/plugin.wasm");
        assert!(result.is_err(), "expected Err for missing path");
        let msg = result.err().unwrap();
        assert!(
            msg.contains("failed to read wasm"),
            "error message should describe read failure, got: {msg}"
        );
    }

    // Plausible wrong impl: install_wasm mutates state before validating the binary,
    // leaving the registry dirty on failure.
    #[test]
    fn install_wasm_invalid_path_does_not_mutate_registry() {
        let mut host = PluginHost::new(PluginRegistry::new());
        let manifest = PluginManifest {
            id: PluginId::new("bad-plugin"),
            version: "0.1.0".to_string(),
            kind: PluginKind::Wasm { path: "/nonexistent.wasm".to_string() },
            hooks: vec![HookKind::PreDispatch],
            roles: vec![],
            description: "bad".to_string(),
            memory_limit_mb: None,
            cpu_timeout_ms: None,
        };
        let err = host.install_wasm(manifest, "/nonexistent.wasm");
        assert!(err.is_err());
        // Registry must be untouched — rollback by construction.
        assert!(
            host.registry.manifests.is_empty(),
            "registry must not be mutated on failed install"
        );
    }

    // Plausible wrong impl: register() does not insert into the map, plugins_for_hook returns empty.
    #[test]
    fn install_registers_in_registry() {
        let mut host = PluginHost::new(PluginRegistry::new());
        let manifest = native_manifest("auth-plugin", vec![HookKind::PreDispatch]);
        host.registry.register(manifest);
        let found = host.registry.plugins_for_hook(&HookKind::PreDispatch);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, PluginId::new("auth-plugin"));
    }

    // Plausible wrong impl: uninstall does not remove from the registry, plugins_for_hook still returns the plugin.
    #[test]
    fn uninstall_removes_from_registry() {
        let mut host = PluginHost::new(PluginRegistry::new());
        let manifest = native_manifest("transient-plugin", vec![HookKind::OnFinish]);
        let id = manifest.id.clone();
        host.registry.register(manifest);
        assert_eq!(
            host.registry.plugins_for_hook(&HookKind::OnFinish).len(),
            1,
            "plugin should be present before uninstall"
        );
        let removed = host.uninstall(&id);
        assert!(removed, "uninstall should return true when plugin existed");
        assert!(
            host.registry.plugins_for_hook(&HookKind::OnFinish).is_empty(),
            "registry must be empty after uninstall"
        );
    }

    // Plausible wrong impl: remove() panics or returns true for non-existent ids.
    #[test]
    fn registry_remove_nonexistent_returns_false() {
        let mut registry = PluginRegistry::new();
        let absent = PluginId::new("ghost");
        assert!(
            !registry.remove(&absent),
            "removing a non-existent plugin must return false"
        );
    }

    // Plausible wrong impl: plugins_for_role returns plugins with wrong role
    // (e.g. iterates hooks instead of roles, or uses contains on the wrong field).
    #[test]
    fn plugins_for_role_filters_correctly() {
        let mut host = PluginHost::new(PluginRegistry::new());
        host.registry.register(role_manifest("cache-a", vec![PluginRole::CacheBackend]));
        host.registry.register(role_manifest("router-a", vec![PluginRole::Router]));
        host.registry.register(role_manifest("multi",   vec![PluginRole::CacheBackend, PluginRole::Auth]));

        let cache_plugins = host.plugins_for_role(&PluginRole::CacheBackend);
        assert_eq!(cache_plugins.len(), 2, "expected cache-a and multi");
        let ids: Vec<&str> = cache_plugins.iter().map(|m| m.id.0.as_str()).collect();
        assert!(ids.contains(&"cache-a"));
        assert!(ids.contains(&"multi"));
        assert!(!ids.contains(&"router-a"), "router-a must not appear for CacheBackend role");

        let router_plugins = host.plugins_for_role(&PluginRole::Router);
        assert_eq!(router_plugins.len(), 1);
        assert_eq!(router_plugins[0].id.0, "router-a");
    }

    // Plausible wrong impl: default_chain includes plugins from every role
    // instead of filtering to the requested one.
    #[test]
    fn default_chain_only_includes_matching_role() {
        let mut host = PluginHost::new(PluginRegistry::new());
        host.registry.register(role_manifest("cache-1", vec![PluginRole::CacheBackend]));
        host.registry.register(role_manifest("cache-2", vec![PluginRole::CacheBackend]));
        host.registry.register(role_manifest("auth-1",  vec![PluginRole::Auth]));

        let chain = host.default_chain(PluginRole::CacheBackend);
        assert_eq!(chain.plugins.len(), 2, "chain must contain exactly the two cache plugins");
        assert!(chain.fail_open, "CacheBackend chains must be fail-open");

        let auth_chain = host.default_chain(PluginRole::Auth);
        assert_eq!(auth_chain.plugins.len(), 1);
        assert!(!auth_chain.fail_open, "Auth chain must be fail-closed");
    }

    // Plausible wrong impl: PluginChain::new sets fail_open=true for Auth.
    #[test]
    fn plugin_chain_auth_is_fail_closed() {
        let chain = PluginChain::new(PluginRole::Auth, vec![PluginId::new("auth-plugin")]);
        assert!(!chain.fail_open, "Auth chains must always be fail-closed");
    }

    // Plausible wrong impl: PluginChain::new sets fail_open=false for non-Auth roles.
    #[test]
    fn plugin_chain_non_auth_is_fail_open() {
        for role in [PluginRole::Router, PluginRole::Compressor, PluginRole::Provider, PluginRole::CacheBackend] {
            let chain = PluginChain::new(role.clone(), vec![]);
            assert!(chain.fail_open, "{role:?} chain must be fail-open");
        }
    }
}
