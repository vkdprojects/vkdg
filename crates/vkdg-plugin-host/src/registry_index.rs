//! The registry index: how `vkdg plugin search` and `install <name>` find a plugin.
//!
//! The registry is a plain git repo of manifests (see `docs/plugins/registry.md`).
//! Walking every file to answer one search would need a request per plugin, so the
//! repo also publishes a single `index.json` listing what it holds. That keeps the
//! client to one fetch, and keeps the registry a repo rather than a service.
//!
//! A tap is any repo with the same layout, so a third party can host plugins
//! without asking anyone's permission.

use serde::{Deserialize, Serialize};

use crate::registry_manifest::PluginManifestKind;

/// The default registry, used when no tap is given.
pub const DEFAULT_REGISTRY: &str = "vkdprojects/vkdg-registry";

/// One plugin as listed in the index.
///
/// This is a summary, not the manifest: enough to search and to locate the real
/// manifest, which is fetched and validated before anything is installed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexEntry {
    pub name: String,
    pub version: String,
    pub kind: PluginManifestKind,
    pub description: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<String>,
    /// Path to the manifest inside the registry repo.
    pub manifest_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
}

impl IndexEntry {
    /// Whether this entry matches a free-text query.
    ///
    /// Matching covers name, description, tags and model globs so that searching
    /// for a model family finds the provider that serves it.
    pub fn matches(&self, query: &str) -> bool {
        let q = query.trim().to_ascii_lowercase();
        if q.is_empty() {
            return true;
        }
        let haystacks = [self.name.as_str(), self.description.as_str()];
        haystacks
            .iter()
            .any(|h| h.to_ascii_lowercase().contains(&q))
            || self
                .tags
                .iter()
                .chain(self.models.iter())
                .any(|t| t.to_ascii_lowercase().contains(&q))
    }
}

/// A registry index document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryIndex {
    /// Index format version, so an older client can refuse a newer layout rather
    /// than silently misread it.
    pub schema: u32,
    /// When the index was generated (RFC 3339), for cache decisions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generated_at: Option<String>,
    pub plugins: Vec<IndexEntry>,
}

/// The index layout this client understands.
pub const INDEX_SCHEMA: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IndexError {
    #[error("invalid index JSON: {0}")]
    Parse(String),
    #[error(
        "index schema {found} is newer than this client supports ({INDEX_SCHEMA}); upgrade vkdg"
    )]
    Schema { found: u32 },
    #[error("duplicate plugin {name:?} in index")]
    Duplicate { name: String },
}

impl RegistryIndex {
    /// Parse and validate an index document.
    pub fn from_json(json: &str) -> Result<Self, IndexError> {
        let index: Self =
            serde_json::from_str(json).map_err(|e| IndexError::Parse(e.to_string()))?;
        if index.schema > INDEX_SCHEMA {
            return Err(IndexError::Schema {
                found: index.schema,
            });
        }
        // A duplicate name would make `install <name>` ambiguous.
        let mut seen: Vec<&str> = Vec::with_capacity(index.plugins.len());
        for entry in &index.plugins {
            if seen.contains(&entry.name.as_str()) {
                return Err(IndexError::Duplicate {
                    name: entry.name.clone(),
                });
            }
            seen.push(&entry.name);
        }
        Ok(index)
    }

    /// Entries matching a query and optional kind, best match first.
    ///
    /// An exact name match sorts ahead of a substring hit, so searching for a
    /// plugin you already know the name of puts it at the top.
    pub fn search(&self, query: &str, kind: Option<PluginManifestKind>) -> Vec<&IndexEntry> {
        let q = query.trim().to_ascii_lowercase();
        let mut hits: Vec<&IndexEntry> = self
            .plugins
            .iter()
            // `is_none_or` postdates the project MSRV.
            .filter(|e| kind.map_or(true, |k| e.kind == k))
            .filter(|e| e.matches(query))
            .collect();
        hits.sort_by_key(|e| {
            let exact = e.name.to_ascii_lowercase() != q;
            (exact, e.name.clone())
        });
        hits
    }

    /// One entry by exact name.
    pub fn get(&self, name: &str) -> Option<&IndexEntry> {
        self.plugins.iter().find(|e| e.name == name)
    }
}

/// URL of the index for a registry or tap.
///
/// `registry` is an `owner/repo` pair; `git_ref` is a branch or tag.
pub fn index_url(registry: &str, git_ref: &str) -> String {
    format!("https://raw.githubusercontent.com/{registry}/{git_ref}/index.json")
}

/// URL of a manifest inside a registry, given its path from the index.
pub fn manifest_url(registry: &str, git_ref: &str, manifest_path: &str) -> String {
    format!("https://raw.githubusercontent.com/{registry}/{git_ref}/{manifest_path}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const INDEX: &str = r#"{
      "schema": 1,
      "generated_at": "2026-09-27T00:00:00Z",
      "plugins": [
        {
          "name": "deepinfra",
          "version": "1.0.0",
          "kind": "provider",
          "description": "DeepInfra inference API",
          "tags": ["openai-compat", "cheap"],
          "models": ["meta-llama/*"],
          "manifest_path": "plugins/providers/deepinfra.yaml"
        },
        {
          "name": "rust-build-errors",
          "version": "0.2.0",
          "kind": "filter-pack",
          "description": "Compresses cargo output",
          "tags": ["rust", "build"],
          "manifest_path": "plugins/filter-packs/rust-build-errors.yaml"
        }
      ]
    }"#;

    /// Refutes: an index format the client cannot read, or one whose kinds do not
    /// line up with the manifest schema.
    #[test]
    fn index_parses_and_kinds_match_the_manifest_schema() {
        let index = RegistryIndex::from_json(INDEX).expect("index parses");
        assert_eq!(index.plugins.len(), 2);
        assert_eq!(
            index.get("deepinfra").unwrap().kind,
            PluginManifestKind::Provider
        );
        assert_eq!(
            index.get("rust-build-errors").unwrap().kind,
            PluginManifestKind::FilterPack
        );
        assert!(index.get("absent").is_none());
    }

    /// Refutes: a newer index being misread by an older client. Refusing is safer
    /// than guessing which fields still mean what.
    #[test]
    fn newer_schema_is_refused_rather_than_guessed() {
        let future = INDEX.replace("\"schema\": 1", "\"schema\": 99");
        assert_eq!(
            RegistryIndex::from_json(&future).err(),
            Some(IndexError::Schema { found: 99 })
        );
    }

    /// Refutes: two plugins sharing a name, which would make install ambiguous.
    #[test]
    fn duplicate_names_are_refused() {
        let dup = INDEX.replace("rust-build-errors", "deepinfra");
        match RegistryIndex::from_json(&dup) {
            Err(IndexError::Duplicate { name }) => assert_eq!(name, "deepinfra"),
            other => panic!("duplicate must be refused, got {other:?}"),
        }
    }

    /// Refutes: search that only matches names, so a user cannot find a provider
    /// by the model family they actually care about.
    #[test]
    fn search_covers_name_description_tags_and_models() {
        let index = RegistryIndex::from_json(INDEX).expect("index parses");

        let by_name = index.search("deepinfra", None);
        assert_eq!(by_name.len(), 1);
        assert_eq!(by_name[0].name, "deepinfra");

        assert_eq!(index.search("meta-llama", None).len(), 1, "model glob");
        assert_eq!(index.search("cargo", None).len(), 1, "description");
        assert_eq!(index.search("rust", None).len(), 1, "tag");
        assert_eq!(index.search("", None).len(), 2, "empty query lists all");
        assert!(index.search("nonexistent", None).is_empty());
    }

    /// Refutes: a kind filter that does not filter.
    #[test]
    fn kind_filter_narrows_results() {
        let index = RegistryIndex::from_json(INDEX).expect("index parses");
        let packs = index.search("", Some(PluginManifestKind::FilterPack));
        assert_eq!(packs.len(), 1);
        assert_eq!(packs[0].name, "rust-build-errors");
        assert!(index
            .search("", Some(PluginManifestKind::Router))
            .is_empty());
    }

    /// Refutes: an exact name buried under substring matches.
    #[test]
    fn exact_name_sorts_first() {
        let json = INDEX.replace("\"name\": \"rust-build-errors\"", "\"name\": \"rust\"");
        let index = RegistryIndex::from_json(&json).expect("index parses");
        let hits = index.search("rust", None);
        assert_eq!(
            hits.first().map(|e| e.name.as_str()),
            Some("rust"),
            "an exact match must lead: {hits:?}"
        );
    }

    /// Refutes: URLs that point at the repo page instead of raw content, which
    /// would download HTML.
    #[test]
    fn urls_target_raw_content() {
        assert_eq!(
            index_url(DEFAULT_REGISTRY, "main"),
            "https://raw.githubusercontent.com/vkdprojects/vkdg-registry/main/index.json"
        );
        assert_eq!(
            manifest_url(DEFAULT_REGISTRY, "main", "plugins/providers/deepinfra.yaml"),
            "https://raw.githubusercontent.com/vkdprojects/vkdg-registry/main/plugins/providers/deepinfra.yaml"
        );
    }
}
