//! Pack registry — maps ContentClass to the active FilterPack.
//!
//! `PackRegistry` holds the set of registered packs and tracks which are
//! disabled. Dispatch is O(n) over the number of registered packs; in
//! practice n ≤ 20, so a HashMap by id suffices over a sorted structure.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::class::ContentClass;
use crate::FilterPack;

/// Registry of FilterPack implementations.
///
/// Each pack covers exactly one `ContentClass`. At most one pack per class
/// should be registered at a time; if two packs handle the same class,
/// `find_for_class` returns whichever comes first in the internal map
/// iteration order (non-deterministic).
pub struct PackRegistry {
    packs: HashMap<String, Arc<dyn FilterPack>>,
    disabled: HashSet<String>,
}

impl PackRegistry {
    /// Build a registry with all 14 built-in packs enabled.
    pub fn with_all_defaults() -> Self {
        use crate::packs;
        let mut r = Self {
            packs: HashMap::new(),
            disabled: HashSet::new(),
        };
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
        r.register(Arc::new(packs::generic::GenericPack));
        r
    }

    /// Empty registry — no packs active.
    pub fn empty() -> Self {
        Self {
            packs: HashMap::new(),
            disabled: HashSet::new(),
        }
    }

    /// Register a pack. Replaces any existing pack with the same id.
    pub fn register(&mut self, pack: Arc<dyn FilterPack>) {
        self.packs.insert(pack.id().to_string(), pack);
    }

    /// Disable a pack by id. Disabled packs are skipped by `find_for_class`.
    pub fn disable(&mut self, pack_id: &str) {
        self.disabled.insert(pack_id.to_string());
    }

    /// Re-enable a previously disabled pack.
    pub fn enable(&mut self, pack_id: &str) {
        self.disabled.remove(pack_id);
    }

    /// Find the first enabled pack that handles `class`, if any.
    pub fn find_for_class(&self, class: &ContentClass) -> Option<&dyn FilterPack> {
        self.packs
            .values()
            .filter(|p| !self.disabled.contains(p.id()))
            .find(|p| p.handles() == *class)
            .map(|p| p.as_ref())
    }

    /// List all registered packs as `(id, enabled)` pairs.
    pub fn list_all(&self) -> Vec<(&str, bool)> {
        self.packs
            .keys()
            .map(|id| (id.as_str(), !self.disabled.contains(id)))
            .collect()
    }
}
