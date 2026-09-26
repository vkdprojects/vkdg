//! Pack registry — maps ContentClass to the active FilterPack.
//!
//! Packs are dispatched in insertion order, so the first registered pack
//! for a class wins. `GenericPack` is always registered last so it acts
//! as a true fallback, never competing with a specific pack.

use std::collections::HashSet;
use std::sync::Arc;

use crate::class::ContentClass;
use crate::FilterPack;

/// Registry of FilterPack implementations.
///
/// Lookup is O(n) in registration order — deterministic and predictable.
/// Register specific packs before `GenericPack`; the first match wins.
pub struct PackRegistry {
    packs: Vec<Arc<dyn FilterPack>>,
    disabled: HashSet<String>,
}

impl PackRegistry {
    /// All 14 built-in packs, specific packs before the Generic fallback.
    pub fn with_all_defaults() -> Self {
        use crate::packs;
        let mut r = Self::empty();
        r.register(Arc::new(packs::command::CommandOutputPack));
        r.register(Arc::new(packs::stack_trace::StackTracePack));
        r.register(Arc::new(packs::json::JsonOutputPack));
        r.register(Arc::new(packs::file_list::FileListPack));
        r.register(Arc::new(packs::diff::DiffPack));
        r.register(Arc::new(packs::git::GitStatusPack));
        r.register(Arc::new(packs::git::GitLogPack));
        r.register(Arc::new(packs::typescript::TypeScriptBuildPack));
        r.register(Arc::new(packs::lint::EslintOutputPack));
        r.register(Arc::new(packs::lint::NpmAuditPack));
        r.register(Arc::new(packs::docker::DockerLogPack));
        r.register(Arc::new(packs::tests::TestOutputPack));
        r.register(Arc::new(packs::hex::HexDumpPack));
        r.register(Arc::new(packs::generic::GenericPack)); // fallback — always last
        r
    }

    pub fn empty() -> Self {
        Self {
            packs: Vec::new(),
            disabled: HashSet::new(),
        }
    }

    /// Register a pack at the end of the priority queue.
    /// If a pack with the same id is already registered, it is removed first
    /// so the new one takes effect at the insertion point.
    pub fn register(&mut self, pack: Arc<dyn FilterPack>) {
        self.packs.retain(|p| p.id() != pack.id());
        self.packs.push(pack);
    }

    pub fn disable(&mut self, pack_id: &str) {
        self.disabled.insert(pack_id.to_string());
    }

    pub fn enable(&mut self, pack_id: &str) {
        self.disabled.remove(pack_id);
    }

    /// First enabled pack that handles `class`, in registration order.
    pub fn find_for_class(&self, class: &ContentClass) -> Option<&dyn FilterPack> {
        self.packs
            .iter()
            .filter(|p| !self.disabled.contains(p.id()))
            .find(|p| p.handles() == *class)
            .map(|p| p.as_ref())
    }

    /// All registered packs as `(id, enabled)` in registration order.
    pub fn list_all(&self) -> Vec<(&str, bool)> {
        self.packs
            .iter()
            .map(|p| (p.id(), !self.disabled.contains(p.id())))
            .collect()
    }
}
