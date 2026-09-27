//! On-disk store for installed plugins.
//!
//! Layout under the plugins directory (`$VKDG_PLUGINS_DIR`, else
//! `~/.config/vkdg/plugins`):
//!
//! ```text
//! <dir>/<name>/manifest.yaml   the published manifest, as installed
//! <dir>/<name>/plugin.wasm     the component (wasm installs only)
//! ```
//!
//! One directory per plugin makes removal a single `remove_dir_all` and makes a
//! half-finished install visible rather than silently partial: the manifest is
//! written last, so a directory without one is incomplete and is not listed.
//!
//! Installs are verified before they land. A `.wasm` is checked against the
//! manifest checksum and compiled, so a corrupt or hostile artefact is rejected
//! while the previous version is still in place.

use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::registry_manifest::{RegistryManifest, SourceKind};
use crate::{PluginId, PluginKind, PluginManifest, PluginRole, WasmPluginInstance};

const MANIFEST_FILE: &str = "manifest.yaml";
const WASM_FILE: &str = "plugin.wasm";

/// Why an install or removal failed.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("io: {0}")]
    Io(String),
    #[error("manifest: {0}")]
    Manifest(String),
    #[error("checksum mismatch: manifest declares {expected}, artefact is sha256:{actual}")]
    Checksum { expected: String, actual: String },
    #[error("not a usable wasm component: {0}")]
    InvalidComponent(String),
    #[error("plugin {0:?} is not installed")]
    NotInstalled(String),
    #[error("{0}")]
    Unsupported(String),
}

/// An installed plugin.
#[derive(Debug, Clone)]
pub struct InstalledPlugin {
    pub manifest: RegistryManifest,
    /// Directory holding this plugin's files.
    pub dir: PathBuf,
}

impl InstalledPlugin {
    /// Path to the component, when this plugin ships one.
    pub fn wasm_path(&self) -> Option<PathBuf> {
        let path = self.dir.join(WASM_FILE);
        path.is_file().then_some(path)
    }

    /// The host-side manifest used to load this plugin.
    pub fn host_manifest(&self) -> PluginManifest {
        PluginManifest {
            id: PluginId::new(self.manifest.name.clone()),
            version: self.manifest.version.clone(),
            kind: PluginKind::Wasm {
                path: self.dir.join(WASM_FILE).display().to_string(),
            },
            hooks: vec![],
            roles: vec![role_for(&self.manifest)],
            description: self.manifest.description.clone(),
            memory_limit_mb: None,
            cpu_timeout_ms: None,
        }
    }
}

/// Maps a published kind onto the host's role enum.
fn role_for(manifest: &RegistryManifest) -> PluginRole {
    use crate::registry_manifest::PluginManifestKind as K;
    match manifest.kind {
        K::Provider | K::OauthProvider => PluginRole::Provider,
        K::FilterPack | K::Compressor => PluginRole::Compressor,
        K::Router => PluginRole::Router,
        K::CacheBackend => PluginRole::CacheBackend,
        K::Auth => PluginRole::Auth,
    }
}

/// The plugins directory and the operations over it.
pub struct PluginStore {
    root: PathBuf,
}

impl PluginStore {
    /// Store rooted at `$VKDG_PLUGINS_DIR`, else `plugins/` next to
    /// `$VKDG_ACCOUNTS_DB`, else `$HOME/.config/vkdg/plugins`, else
    /// `/var/lib/vkdg/plugins`.
    ///
    /// Plugins must live on the same persistent volume as accounts and keys. A
    /// container has no real home (`HOME` unset or `/`), and a relative path
    /// resolves inside the image: both lose every install on redeploy.
    pub fn from_env() -> Self {
        Self {
            root: default_root(
                std::env::var_os("VKDG_PLUGINS_DIR"),
                std::env::var_os("VKDG_ACCOUNTS_DB"),
                std::env::var_os("HOME"),
            ),
        }
    }

    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Load every installed provider plugin that ships a component.
    ///
    /// One broken plugin must not stop the gateway: each result is returned so
    /// the caller registers the good ones and logs the rest by name.
    pub fn load_providers(&self) -> Vec<(String, Result<crate::WasmProviderAdapter, String>)> {
        let installed = match self.list() {
            Ok(list) => list,
            Err(e) => return vec![(self.root.display().to_string(), Err(e.to_string()))],
        };
        installed
            .into_iter()
            .filter(|p| role_for(&p.manifest) == PluginRole::Provider)
            .filter_map(|p| {
                let path = p.wasm_path()?;
                let manifest = p.host_manifest();
                let loaded = std::fs::read(&path)
                    .map_err(|e| format!("read {}: {e}", path.display()))
                    .and_then(|bytes| crate::WasmPluginInstance::from_bytes(&bytes, &manifest))
                    .and_then(|instance| {
                        crate::WasmProviderAdapter::new(std::sync::Arc::new(instance), &manifest)
                    });
                Some((p.manifest.name.clone(), loaded))
            })
            .collect()
    }

    /// Installed plugins, sorted by name. Directories without a manifest are
    /// incomplete installs and are skipped.
    pub fn list(&self) -> Result<Vec<InstalledPlugin>, StoreError> {
        if !self.root.is_dir() {
            return Ok(Vec::new());
        }
        let mut found = Vec::new();
        let entries = fs::read_dir(&self.root).map_err(io)?;
        for entry in entries {
            let dir = entry.map_err(io)?.path();
            if !dir.is_dir() {
                continue;
            }
            let manifest_path = dir.join(MANIFEST_FILE);
            if !manifest_path.is_file() {
                continue;
            }
            let yaml = fs::read_to_string(&manifest_path).map_err(io)?;
            // A manifest that no longer validates is reported, not hidden: the
            // operator needs to know an installed plugin will not load.
            let manifest = RegistryManifest::from_yaml(&yaml)
                .map_err(|e| StoreError::Manifest(format!("{}: {e}", manifest_path.display())))?;
            found.push(InstalledPlugin { manifest, dir });
        }
        found.sort_by(|a, b| a.manifest.name.cmp(&b.manifest.name));
        Ok(found)
    }

    /// One installed plugin by name.
    pub fn get(&self, name: &str) -> Result<Option<InstalledPlugin>, StoreError> {
        let dir = self.root.join(name);
        let manifest_path = dir.join(MANIFEST_FILE);
        if !manifest_path.is_file() {
            return Ok(None);
        }
        let yaml = fs::read_to_string(&manifest_path).map_err(io)?;
        let manifest =
            RegistryManifest::from_yaml(&yaml).map_err(|e| StoreError::Manifest(e.to_string()))?;
        Ok(Some(InstalledPlugin { manifest, dir }))
    }

    /// Install a plugin from its manifest plus the component bytes.
    ///
    /// `wasm_bytes` is required for a wasm install and ignored otherwise. The
    /// artefact is verified against the manifest checksum and compiled before
    /// anything is written, so a failed install leaves the previous version intact.
    pub fn install(
        &self,
        manifest: &RegistryManifest,
        wasm_bytes: Option<&[u8]>,
    ) -> Result<InstalledPlugin, StoreError> {
        manifest
            .validate()
            .map_err(|e| StoreError::Manifest(e.to_string()))?;
        let source = manifest
            .source_kind()
            .map_err(|e| StoreError::Manifest(e.to_string()))?;

        let bytes = match source {
            SourceKind::Wasm => {
                let bytes = wasm_bytes.ok_or_else(|| {
                    StoreError::Unsupported("a wasm install needs the component bytes".into())
                })?;
                let expected = manifest.install.checksum.as_deref().unwrap_or_default();
                let actual = sha256_hex(bytes);
                if !expected.eq_ignore_ascii_case(&format!("sha256:{actual}")) {
                    return Err(StoreError::Checksum {
                        expected: expected.to_owned(),
                        actual,
                    });
                }
                // Compile before installing: a component that cannot load is
                // rejected now rather than at the first request.
                let host_manifest = PluginManifest {
                    id: PluginId::new(manifest.name.clone()),
                    version: manifest.version.clone(),
                    kind: PluginKind::Wasm {
                        path: String::new(),
                    },
                    hooks: vec![],
                    roles: vec![role_for(manifest)],
                    description: manifest.description.clone(),
                    memory_limit_mb: None,
                    cpu_timeout_ms: None,
                };
                WasmPluginInstance::from_bytes(bytes, &host_manifest)
                    .map_err(StoreError::InvalidComponent)?;
                Some(bytes)
            }
            SourceKind::ConfigSnippet => None,
            SourceKind::Crate => {
                return Err(StoreError::Unsupported(format!(
                    "plugin {:?} ships as a Rust crate and is compiled into the binary; \
                     it cannot be installed at runtime",
                    manifest.name
                )));
            }
        };

        let dir = self.root.join(&manifest.name);
        fs::create_dir_all(&dir).map_err(io)?;
        if let Some(bytes) = bytes {
            fs::write(dir.join(WASM_FILE), bytes).map_err(io)?;
        }
        // Written last: a directory without a manifest is an incomplete install
        // and `list` skips it.
        let yaml =
            serde_yaml::to_string(manifest).map_err(|e| StoreError::Manifest(e.to_string()))?;
        fs::write(dir.join(MANIFEST_FILE), yaml).map_err(io)?;

        Ok(InstalledPlugin {
            manifest: manifest.clone(),
            dir,
        })
    }

    /// Remove an installed plugin.
    pub fn remove(&self, name: &str) -> Result<(), StoreError> {
        let dir = self.root.join(name);
        if !dir.join(MANIFEST_FILE).is_file() {
            return Err(StoreError::NotInstalled(name.to_owned()));
        }
        fs::remove_dir_all(&dir).map_err(io)
    }
}

fn io(e: std::io::Error) -> StoreError {
    StoreError::Io(e.to_string())
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// `$XDG_CONFIG_HOME`, else `~/.config`.
fn default_root(
    plugins_dir: Option<std::ffi::OsString>,
    accounts_db: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
) -> PathBuf {
    if let Some(dir) = plugins_dir {
        return dir.into();
    }
    if let Some(parent) = accounts_db
        .as_ref()
        .and_then(|db| Path::new(db).parent())
        .filter(|p| !p.as_os_str().is_empty())
    {
        return parent.join("plugins");
    }
    match home {
        Some(h) if !h.is_empty() && h != "/" => Path::new(&h).join(".config/vkdg/plugins"),
        _ => PathBuf::from("/var/lib/vkdg/plugins"),
    }
}

#[cfg(test)]
mod root_tests {
    use super::default_root;
    use std::path::PathBuf;

    // A plugin installed into the image instead of the volume vanished on redeploy.
    #[test]
    fn plugins_follow_the_data_volume() {
        let s = |v: &str| Some(v.into());
        assert_eq!(
            default_root(s("/p"), s("/data/accounts.db"), s("/home/u")),
            PathBuf::from("/p")
        );
        assert_eq!(
            default_root(None, s("/var/lib/vkdg/accounts.db"), None),
            PathBuf::from("/var/lib/vkdg/plugins")
        );
        assert_eq!(
            default_root(None, None, s("/home/u")),
            PathBuf::from("/home/u/.config/vkdg/plugins")
        );
        for home in [None, s("/"), s("")] {
            assert_eq!(
                default_root(None, None, home),
                PathBuf::from("/var/lib/vkdg/plugins")
            );
        }
    }
}
