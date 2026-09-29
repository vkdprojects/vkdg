//! Combos persisted as one JSON file next to the other data files.
//!
//! Combos are edited from the admin API, so they live in the data directory,
//! not in the operator's config file. The file is replaced atomically (write
//! to a temp file, then rename) so a crash never leaves half a table.

use std::path::{Path, PathBuf};

use crate::plan::Combo;

pub struct ComboStore {
    path: PathBuf,
}

impl ComboStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Combos on disk; empty when the file does not exist yet. A file that
    /// does not parse is an error: starting with no combos would silently
    /// reroute their traffic.
    pub fn load(&self) -> Result<Vec<Combo>, String> {
        match std::fs::read(&self.path) {
            Ok(bytes) => {
                serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", self.path.display()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(format!("{}: {e}", self.path.display())),
        }
    }

    pub fn save(&self, combos: &[Combo]) -> Result<(), String> {
        let err = |e: std::io::Error| format!("{}: {e}", self.path.display());
        if let Some(dir) = self.path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).map_err(err)?;
        }
        let json = serde_json::to_vec_pretty(combos).map_err(|e| e.to_string())?;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, json).map_err(err)?;
        std::fs::rename(&tmp, &self.path).map_err(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vkdg_routing::StrategyKind;

    fn combo(id: &str) -> Combo {
        Combo {
            id: id.into(),
            match_patterns: vec!["code:*".into()],
            strategy: StrategyKind::RoundRobin,
            targets: vec![vkdg_core::ConnectionId("c1".into())],
            compression: None,
            cache: None,
            budget: None,
            mode_pack: None,
            model: None,
        }
    }

    #[test]
    fn saved_combos_come_back_and_a_missing_file_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let store = ComboStore::new(dir.path().join("sub/combos.json"));
        assert!(store.load().unwrap().is_empty());
        store.save(&[combo("a"), combo("b")]).unwrap();
        let ids: Vec<String> = store.load().unwrap().into_iter().map(|c| c.id).collect();
        assert_eq!(ids, ["a", "b"]);
    }

    // Plausible wrong impl: a corrupt file reads as "no combos" and the
    // gateway starts routing their traffic somewhere else.
    #[test]
    fn a_corrupt_file_is_an_error_not_an_empty_table() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("combos.json");
        std::fs::write(&path, b"{not json").unwrap();
        assert!(ComboStore::new(path).load().is_err());
    }
}
