//! The published plugin manifest: one YAML file per plugin in the registry.
//!
//! This is the format specified in `docs/plugins/registry.md`, implemented so the
//! CLI, the registry bot and the console all validate identically instead of each
//! guessing. A manifest is untrusted input from a third party, so validation is
//! strict: unknown fields are rejected, names are constrained, and an install
//! source must be exactly one of the supported kinds.
//!
//! Checksums are required for remote WASM. Without one, `vkdg plugin install`
//! would execute whatever bytes the URL happened to serve that day.

use serde::{Deserialize, Serialize};

/// What extension point a plugin fills.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PluginManifestKind {
    Provider,
    /// Provider that also handles interactive login and token refresh.
    OauthProvider,
    /// Compressor for one content class (build logs, git output).
    FilterPack,
    /// Full compression strategy.
    Compressor,
    /// Routing strategy.
    Router,
    /// Cache backend.
    CacheBackend,
    /// Auth and rate limiting.
    Auth,
}

/// Where the plugin's code comes from. Exactly one variant per manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallSource {
    /// URL of a `.wasm` component.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wasm: Option<String>,
    /// `sha256:<hex>` of the `.wasm`. Required whenever `wasm` is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checksum: Option<String>,
    /// YAML to merge into the user's config; for providers that need no code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_snippet: Option<String>,
    /// crates.io package compiled into the binary; needs a gateway rebuild.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crate_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crate_version: Option<String>,
}

/// Which kind of source a manifest declares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    Wasm,
    ConfigSnippet,
    Crate,
}

/// A published plugin manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegistryManifest {
    /// Unique, kebab-case. Doubles as the provider id in config.
    pub name: String,
    /// Semver.
    pub version: String,
    pub kind: PluginManifestKind,
    pub description: String,
    /// SPDX identifier.
    pub license: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Model globs this provider claims.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<String>,
    /// Content class a filter pack handles.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_class: Option<String>,

    pub install: InstallSource,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_vkdg_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changelog: Option<String>,
}

/// Why a manifest was rejected. Each variant names the offending field so a
/// plugin author can fix it without reading the source.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ManifestError {
    #[error("invalid YAML: {0}")]
    Parse(String),
    #[error("field `{field}` must not be empty")]
    Empty { field: &'static str },
    #[error("name {name:?} must be kebab-case: lowercase letters, digits and single hyphens")]
    Name { name: String },
    #[error("version {version:?} must be semver, e.g. 1.0.0")]
    Version { version: String },
    #[error("install must declare exactly one of `wasm`, `config_snippet` or `crate_name`")]
    SourceCount,
    #[error("a wasm install requires `checksum` in the form sha256:<64 hex chars>")]
    Checksum,
    #[error("a crate install requires `crate_version`")]
    CrateVersion,
    #[error("kind {kind:?} cannot declare `models`; that field is for providers")]
    ModelsNotAllowed { kind: PluginManifestKind },
    #[error("a filter-pack must declare `content_class`")]
    ContentClassRequired,
}

impl RegistryManifest {
    /// Parse and validate a manifest from YAML.
    pub fn from_yaml(yaml: &str) -> Result<Self, ManifestError> {
        let manifest: Self =
            serde_yaml::from_str(yaml).map_err(|e| ManifestError::Parse(e.to_string()))?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Which source kind this manifest declares.
    pub fn source_kind(&self) -> Result<SourceKind, ManifestError> {
        let i = &self.install;
        let declared = [
            i.wasm.is_some(),
            i.config_snippet.is_some(),
            i.crate_name.is_some(),
        ];
        if declared.iter().filter(|d| **d).count() != 1 {
            return Err(ManifestError::SourceCount);
        }
        Ok(if i.wasm.is_some() {
            SourceKind::Wasm
        } else if i.config_snippet.is_some() {
            SourceKind::ConfigSnippet
        } else {
            SourceKind::Crate
        })
    }

    /// True when installing this plugin requires no gateway rebuild.
    ///
    /// The point of the registry: WASM and config-only plugins are live, a crate
    /// plugin is not.
    pub fn installs_without_rebuild(&self) -> bool {
        !matches!(self.source_kind(), Ok(SourceKind::Crate))
    }

    /// Enforce every rule the registry bot checks.
    pub fn validate(&self) -> Result<(), ManifestError> {
        for (field, value) in [
            ("name", &self.name),
            ("version", &self.version),
            ("description", &self.description),
            ("license", &self.license),
        ] {
            if value.trim().is_empty() {
                return Err(ManifestError::Empty { field });
            }
        }
        if !is_kebab_case(&self.name) {
            return Err(ManifestError::Name {
                name: self.name.clone(),
            });
        }
        if !is_semver(&self.version) {
            return Err(ManifestError::Version {
                version: self.version.clone(),
            });
        }

        match self.source_kind()? {
            // Unpinned remote code is the one thing we will not install.
            SourceKind::Wasm => {
                if !self
                    .install
                    .checksum
                    .as_deref()
                    .is_some_and(is_sha256_checksum)
                {
                    return Err(ManifestError::Checksum);
                }
            }
            SourceKind::Crate => {
                if self.install.crate_version.is_none() {
                    return Err(ManifestError::CrateVersion);
                }
            }
            SourceKind::ConfigSnippet => {}
        }

        // `models` only means something for a provider; on any other kind it is a
        // copy-paste mistake that would silently do nothing.
        let provider_kind = matches!(
            self.kind,
            PluginManifestKind::Provider | PluginManifestKind::OauthProvider
        );
        if !self.models.is_empty() && !provider_kind {
            return Err(ManifestError::ModelsNotAllowed { kind: self.kind });
        }
        if self.kind == PluginManifestKind::FilterPack && self.content_class.is_none() {
            return Err(ManifestError::ContentClassRequired);
        }
        Ok(())
    }
}

/// Lowercase letters, digits and single interior hyphens.
///
/// The name becomes a provider id in config and a directory under the plugins
/// dir, so path separators and case variation are refused rather than sanitised.
pub(crate) fn is_kebab_case(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("--")
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// `major.minor.patch`, with an optional pre-release or build suffix.
fn is_semver(version: &str) -> bool {
    let core = version
        .split_once('-')
        .map_or(version, |(c, _)| c)
        .split_once('+')
        .map_or_else(
            || version.split_once('-').map_or(version, |(c, _)| c),
            |(c, _)| c,
        );
    let parts: Vec<&str> = core.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

/// `sha256:` followed by exactly 64 hex characters.
fn is_sha256_checksum(checksum: &str) -> bool {
    checksum
        .strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const WASM_MANIFEST: &str = r#"
name: deepinfra
version: "1.0.0"
kind: provider
description: "DeepInfra inference API"
license: MIT
author: "Jane Doe <jane@example.com>"
tags: [openai-compat, inference]
models: ["meta-llama/*"]
install:
  wasm: "https://example.com/deepinfra.wasm"
  checksum: "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
"#;

    /// Refutes: a schema that cannot read the format the docs publish.
    #[test]
    fn documented_provider_manifest_parses() {
        let m = RegistryManifest::from_yaml(WASM_MANIFEST).expect("documented example must parse");
        assert_eq!(m.name, "deepinfra");
        assert_eq!(m.kind, PluginManifestKind::Provider);
        assert_eq!(m.source_kind().unwrap(), SourceKind::Wasm);
        assert!(m.installs_without_rebuild());
    }

    /// Refutes: installing unpinned remote code. A URL without a checksum means
    /// whatever bytes the host serves today get executed.
    #[test]
    fn remote_wasm_requires_a_pinned_checksum() {
        let yaml = WASM_MANIFEST.replace(
            "  checksum: \"sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\"\n",
            "",
        );
        assert_eq!(
            RegistryManifest::from_yaml(&yaml).err(),
            Some(ManifestError::Checksum)
        );

        let truncated = WASM_MANIFEST.replace(
            "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "sha256:abc",
        );
        assert_eq!(
            RegistryManifest::from_yaml(&truncated).err(),
            Some(ManifestError::Checksum)
        );
    }

    /// Refutes: a name that escapes its directory or collides case-insensitively.
    #[test]
    fn names_are_constrained_to_kebab_case() {
        for bad in [
            "My-Plugin",
            "../evil",
            "plugin_name",
            "-lead",
            "trail-",
            "a--b",
        ] {
            let yaml = WASM_MANIFEST.replace("name: deepinfra", &format!("name: \"{bad}\""));
            match RegistryManifest::from_yaml(&yaml) {
                Err(ManifestError::Name { .. }) => {}
                other => panic!("{bad:?} must be rejected as a name, got {other:?}"),
            }
        }
    }

    /// Refutes: accepting a version that cannot be compared for updates.
    #[test]
    fn versions_must_be_semver() {
        for bad in ["1", "1.0", "v1.0.0", "1.0.x", ""] {
            let yaml = WASM_MANIFEST.replace("version: \"1.0.0\"", &format!("version: \"{bad}\""));
            assert!(
                RegistryManifest::from_yaml(&yaml).is_err(),
                "{bad:?} must be rejected as a version"
            );
        }
        for good in ["1.0.0", "0.2.13", "2.0.0-beta.1", "1.0.0+build5"] {
            let yaml = WASM_MANIFEST.replace("version: \"1.0.0\"", &format!("version: \"{good}\""));
            assert!(
                RegistryManifest::from_yaml(&yaml).is_ok(),
                "{good:?} must be accepted as a version"
            );
        }
    }

    /// Refutes: a manifest with two sources, where the installer would silently
    /// pick one, or with none, which cannot install at all.
    #[test]
    fn exactly_one_install_source_is_required() {
        let both = WASM_MANIFEST.replace(
            "  wasm: \"https://example.com/deepinfra.wasm\"",
            "  wasm: \"https://example.com/deepinfra.wasm\"\n  config_snippet: \"provider: openai-compat\"",
        );
        assert_eq!(
            RegistryManifest::from_yaml(&both).err(),
            Some(ManifestError::SourceCount)
        );

        let none = r#"
name: empty
version: "1.0.0"
kind: provider
description: "no source"
license: MIT
install: {}
"#;
        assert_eq!(
            RegistryManifest::from_yaml(none).err(),
            Some(ManifestError::SourceCount)
        );
    }

    /// Refutes: treating a crate plugin as live-installable. It needs a rebuild,
    /// and the CLI must say so rather than pretend to install it.
    #[test]
    fn crate_source_is_pinned_and_needs_a_rebuild() {
        let yaml = r#"
name: builtin-ish
version: "1.0.0"
kind: provider
description: "compiled in"
license: MIT
install:
  crate_name: vkdg-provider-example
  crate_version: "0.3.1"
"#;
        let m = RegistryManifest::from_yaml(yaml).expect("crate manifest parses");
        assert_eq!(m.source_kind().unwrap(), SourceKind::Crate);
        assert!(!m.installs_without_rebuild());

        let unpinned = yaml.replace("  crate_version: \"0.3.1\"\n", "");
        assert_eq!(
            RegistryManifest::from_yaml(&unpinned).err(),
            Some(ManifestError::CrateVersion)
        );
    }

    /// Refutes: fields that look meaningful on the wrong kind and quietly do
    /// nothing — `models` on a compressor, or a filter pack with no content class.
    #[test]
    fn fields_must_match_the_declared_kind() {
        let compressor_with_models = WASM_MANIFEST.replace("kind: provider", "kind: compressor");
        match RegistryManifest::from_yaml(&compressor_with_models) {
            Err(ManifestError::ModelsNotAllowed { .. }) => {}
            other => panic!("models on a compressor must be rejected, got {other:?}"),
        }

        let pack = WASM_MANIFEST
            .replace("kind: provider", "kind: filter-pack")
            .replace("models: [\"meta-llama/*\"]\n", "");
        assert_eq!(
            RegistryManifest::from_yaml(&pack).err(),
            Some(ManifestError::ContentClassRequired)
        );
    }

    /// Refutes: silently ignoring a misspelled field, which would leave an author
    /// believing a setting took effect.
    #[test]
    fn unknown_fields_are_rejected() {
        let yaml = WASM_MANIFEST.replace("tags:", "tagz:");
        match RegistryManifest::from_yaml(&yaml) {
            Err(ManifestError::Parse(_)) => {}
            other => panic!("unknown field must be rejected, got {other:?}"),
        }
    }
}
