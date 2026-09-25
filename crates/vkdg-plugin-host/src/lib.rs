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

// ── PluginManifest ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    pub id: PluginId,
    pub version: String,
    pub kind: PluginKind,
    pub hooks: Vec<HookKind>,
    pub description: String,
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
            description: "test plugin".to_string(),
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
            description: "bad".to_string(),
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
}
