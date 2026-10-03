//! Plugin host: manifest registry and hook dispatch.
//!
//! Phase D adds Wasmtime-based WASM component execution.  The WIT contract is
//! defined in `wit/provider.wit`.  Full bindgen codegen (`wasmtime::component::bindgen!`)
//! is deferred to Phase E; for now we validate that a component binary is
//! well-formed and reserve the call surface.

pub mod store;
pub use store::{BrokenPlugin, InstalledPlugin, PluginListing, PluginStore, StoreError};

pub mod registry_index;
pub use registry_index::{IndexEntry, IndexError, RegistryIndex, DEFAULT_REGISTRY};

pub mod registry_manifest;
pub use registry_manifest::{ManifestError, PluginManifestKind, RegistryManifest, SourceKind};

pub mod wasm_auth;
pub use wasm_auth::{AuthContext, AuthOutcome, AuthPlugin, RateLimitOutcome, WasmAuth};

pub mod wasm_cache;
pub use wasm_cache::WasmCache;

pub mod wasm_router;
pub use wasm_router::WasmRouter;

pub mod wasm_compressor;
pub use wasm_compressor::WasmCompressor;

pub mod wasm_provider;
pub use wasm_provider::WasmProviderAdapter;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use wasmtime::component::{Component, Linker};
use wasmtime::{Config, Engine, Store};

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

/// Default ceiling on a plugin's linear memory.
const DEFAULT_MEMORY_LIMIT_MB: u32 = 256;

/// Default execution budget per call, in Wasmtime fuel units.
///
/// Fuel bounds work, not wall-clock, so a plugin stuck in a loop is cut off
/// deterministically instead of hanging a request. Roughly a few hundred million
/// instructions: far more than a translate needs, far less than forever.
const DEFAULT_FUEL: u64 = 200_000_000;

/// Per-instance limits, enforced by the host rather than trusted to the plugin.
struct PluginLimits {
    memory_bytes: usize,
    fuel: u64,
}

/// Store state for one call.
///
/// A plugin gets a WASI context that grants nothing: no preopened directories, no
/// environment, no network, stdio to /dev/null. The context exists only because a
/// guest built for `wasm32-wasip2` links Rust's `libstd`, which imports WASI even
/// when the plugin never touches I/O. Refusing the imports outright would make
/// every Rust plugin fail to instantiate; granting them empty means an attempt at
/// I/O fails at the syscall instead. The gateway still owns all real I/O.
struct PluginState {
    limiter: MemoryLimiter,
    wasi: wasmtime_wasi::WasiCtx,
    table: wasmtime::component::ResourceTable,
}

impl wasmtime_wasi::WasiView for PluginState {
    fn ctx(&mut self) -> &mut wasmtime_wasi::WasiCtx {
        &mut self.wasi
    }

    fn table(&mut self) -> &mut wasmtime::component::ResourceTable {
        &mut self.table
    }
}

/// Refuses growth past the manifest's memory ceiling.
struct MemoryLimiter {
    max_bytes: usize,
}

impl wasmtime::ResourceLimiter for MemoryLimiter {
    fn memory_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        Ok(desired <= self.max_bytes)
    }

    fn table_growing(
        &mut self,
        _current: usize,
        _desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        Ok(true)
    }
}

/// A compiled WASM component, ready to call.
///
/// The component is compiled once at install time; each call gets a fresh
/// [`Store`] so one request cannot observe or corrupt another's state.
pub struct WasmPluginInstance {
    engine: Engine,
    component: Component,
    linker: Linker<PluginState>,
    limits: PluginLimits,
}

impl WasmPluginInstance {
    /// Compile a `.wasm` component from disk under the manifest's limits.
    pub fn load(path: &str, manifest: &PluginManifest) -> Result<Self, String> {
        let bytes =
            std::fs::read(path).map_err(|e| format!("failed to read wasm at {path:?}: {e}"))?;
        Self::from_bytes(&bytes, manifest)
            .map_err(|e| format!("invalid wasm component at {path:?}: {e}"))
    }

    /// Compile a component from memory. Used by tests and by future registry
    /// installs that stream bytes rather than write a file first.
    pub fn from_bytes(bytes: &[u8], manifest: &PluginManifest) -> Result<Self, String> {
        let mut config = Config::new();
        // Fuel is what makes a runaway plugin terminate rather than hang.
        config.consume_fuel(true);
        config.wasm_component_model(true);
        let engine = Engine::new(&config).map_err(|e| e.to_string())?;
        let component = Component::from_binary(&engine, bytes).map_err(|e| e.to_string())?;
        // Satisfy the WASI imports a `wasm32-wasip2` guest links via libstd. The
        // context granted below is empty, so these resolve but do nothing useful.
        let mut linker = Linker::new(&engine);
        wasmtime_wasi::add_to_linker_sync(&mut linker)
            .map_err(|e| format!("failed to wire WASI stubs: {e}"))?;

        let memory_mb = manifest.memory_limit_mb.unwrap_or(DEFAULT_MEMORY_LIMIT_MB);
        Ok(Self {
            engine,
            component,
            linker,
            limits: PluginLimits {
                memory_bytes: memory_mb as usize * 1024 * 1024,
                fuel: DEFAULT_FUEL,
            },
        })
    }

    /// A store for one call, with memory and fuel limits applied.
    fn store(&self) -> Result<Store<PluginState>, String> {
        let mut store = Store::new(
            &self.engine,
            PluginState {
                limiter: MemoryLimiter {
                    max_bytes: self.limits.memory_bytes,
                },
                // Nothing is granted: no preopened dirs, no env, no network, and
                // stdio goes nowhere. A plugin that tries I/O fails at the call.
                wasi: wasmtime_wasi::WasiCtxBuilder::new().build(),
                table: wasmtime::component::ResourceTable::new(),
            },
        );
        store.limiter(|state| &mut state.limiter);
        store
            .set_fuel(self.limits.fuel)
            .map_err(|e| format!("failed to set fuel: {e}"))?;
        Ok(store)
    }

    /// Instantiate the component, proving its imports are satisfiable.
    ///
    /// A component that needs host imports we do not provide fails here rather
    /// than mid-request.
    pub fn instantiate(&self) -> Result<(), String> {
        let mut store = self.store()?;
        self.linker
            .instantiate(&mut store, &self.component)
            .map_err(|e| format!("instantiation failed: {e}"))?;
        Ok(())
    }

    /// Call an exported function that takes a string and returns a string.
    ///
    /// Most of the provider contract has this shape once inputs are JSON-encoded,
    /// so a single typed call covers `prepare`, `decode-response` and friends
    /// without one bespoke binding per function.
    pub fn call_string_fn(&self, export: &str, input: &str) -> Result<String, String> {
        let mut store = self.store()?;
        let instance = self
            .linker
            .instantiate(&mut store, &self.component)
            .map_err(|e| format!("instantiation failed: {e}"))?;
        let func = instance
            .get_typed_func::<(String,), (String,)>(&mut store, export)
            .map_err(|e| format!("export {export:?} not found or wrong type: {e}"))?;
        let (out,) = func
            .call(&mut store, (input.to_owned(),))
            .map_err(|e| format!("call to {export:?} failed: {e}"))?;
        func.post_return(&mut store)
            .map_err(|e| format!("post_return failed: {e}"))?;
        Ok(out)
    }

    /// Fuel budget applied to each call.
    pub fn fuel_budget(&self) -> u64 {
        self.limits.fuel
    }

    /// Memory ceiling in bytes.
    pub fn memory_limit_bytes(&self) -> usize {
        self.limits.memory_bytes
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
        Self {
            role,
            plugins,
            fail_open,
        }
    }
}
// ── PluginHost ────────────────────────────────────────────────────────────────

/// Owns the plugin registry and loaded WASM instances.
///
/// `NativeRust` plugins are registered directly via `registry.register`.
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
            PluginKind::NativeRust => return Err("manifest kind must be Wasm".to_string()),
        }
        // Load and validate first — no state mutation until this succeeds.
        let instance = WasmPluginInstance::load(wasm_path, &manifest)?;
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
        self.registry
            .manifests
            .values()
            .filter(|m| m.roles.contains(role))
            .collect()
    }

    /// Build the default plugin chain for a role from all registered plugins.
    /// Ordered by insertion order (FIFO).  Operators can override via config.
    pub fn default_chain(&self, role: PluginRole) -> PluginChain {
        let plugins: Vec<PluginId> = self
            .plugins_for_role(&role)
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
        let manifest = role_manifest("missing", vec![PluginRole::Provider]);
        let result = WasmPluginInstance::load("/nonexistent/path/plugin.wasm", &manifest);
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
            kind: PluginKind::Wasm {
                path: "/nonexistent.wasm".to_string(),
            },
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
            host.registry
                .plugins_for_hook(&HookKind::OnFinish)
                .is_empty(),
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
        host.registry
            .register(role_manifest("cache-a", vec![PluginRole::CacheBackend]));
        host.registry
            .register(role_manifest("router-a", vec![PluginRole::Router]));
        host.registry.register(role_manifest(
            "multi",
            vec![PluginRole::CacheBackend, PluginRole::Auth],
        ));

        let cache_plugins = host.plugins_for_role(&PluginRole::CacheBackend);
        assert_eq!(cache_plugins.len(), 2, "expected cache-a and multi");
        let ids: Vec<&str> = cache_plugins.iter().map(|m| m.id.0.as_str()).collect();
        assert!(ids.contains(&"cache-a"));
        assert!(ids.contains(&"multi"));
        assert!(
            !ids.contains(&"router-a"),
            "router-a must not appear for CacheBackend role"
        );

        let router_plugins = host.plugins_for_role(&PluginRole::Router);
        assert_eq!(router_plugins.len(), 1);
        assert_eq!(router_plugins[0].id.0, "router-a");
    }

    // Plausible wrong impl: default_chain includes plugins from every role
    // instead of filtering to the requested one.
    #[test]
    fn default_chain_only_includes_matching_role() {
        let mut host = PluginHost::new(PluginRegistry::new());
        host.registry
            .register(role_manifest("cache-1", vec![PluginRole::CacheBackend]));
        host.registry
            .register(role_manifest("cache-2", vec![PluginRole::CacheBackend]));
        host.registry
            .register(role_manifest("auth-1", vec![PluginRole::Auth]));

        let chain = host.default_chain(PluginRole::CacheBackend);
        assert_eq!(
            chain.plugins.len(),
            2,
            "chain must contain exactly the two cache plugins"
        );
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
        for role in [
            PluginRole::Router,
            PluginRole::Compressor,
            PluginRole::Provider,
            PluginRole::CacheBackend,
        ] {
            let chain = PluginChain::new(role.clone(), vec![]);
            assert!(chain.fail_open, "{role:?} chain must be fail-open");
        }
    }
}
