//! Install, list and remove against a real directory.
//!
//! The lifecycle is the part a user touches, and the part where a mistake is
//! expensive: a bad install that half-lands leaves a gateway that will not start,
//! and an unverified artefact means running someone else's bytes. These tests hold
//! both lines.

use std::fs;
use std::path::PathBuf;

use vkdg_plugin_host::{PluginStore, RegistryManifest};

/// A unique directory per test, so tests can run in parallel.
fn temp_root(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "vkdg-plugin-store-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    fs::create_dir_all(&root).expect("temp root");
    root
}

fn component() -> Vec<u8> {
    wat::parse_str("(component)").expect("component assembles")
}

fn sha256_of(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn wasm_manifest(name: &str, checksum: &str) -> RegistryManifest {
    let yaml = format!(
        r#"
name: {name}
version: "1.0.0"
kind: provider
description: "test provider"
license: MIT
models: ["custom-*"]
install:
  wasm: "https://example.com/{name}.wasm"
  checksum: "sha256:{checksum}"
"#
    );
    RegistryManifest::from_yaml(&yaml).expect("manifest parses")
}

/// Refutes: writing an artefact without checking it is the one the manifest
/// pinned, which is how a compromised mirror gets executed.
#[test]
fn checksum_mismatch_is_refused_before_anything_is_written() {
    let root = temp_root("checksum");
    let store = PluginStore::new(&root);
    let bytes = component();
    // A valid-looking but wrong digest.
    let manifest = wasm_manifest("mismatch", &"a".repeat(64));

    let err = store
        .install(&manifest, Some(&bytes))
        .expect_err("a wrong checksum must fail the install");
    assert!(
        err.to_string().contains("checksum mismatch"),
        "unexpected error: {err}"
    );
    assert!(
        !root.join("mismatch").exists(),
        "a refused install must leave nothing behind"
    );
    fs::remove_dir_all(&root).ok();
}

/// Refutes: accepting bytes that are not a loadable component, which would only
/// fail on the first request instead of at install time.
#[test]
fn unloadable_component_is_refused() {
    let root = temp_root("badwasm");
    let store = PluginStore::new(&root);
    let bytes = b"definitely not wasm".to_vec();
    let manifest = wasm_manifest("broken", &sha256_of(&bytes));

    let err = store
        .install(&manifest, Some(&bytes))
        .expect_err("garbage must not install even with a matching checksum");
    assert!(
        err.to_string().contains("wasm component"),
        "unexpected error: {err}"
    );
    assert!(!root.join("broken").exists());
    fs::remove_dir_all(&root).ok();
}

/// Refutes: an install that does not round-trip, so the gateway cannot find the
/// plugin it just installed.
#[test]
fn install_then_list_then_remove_round_trips() {
    let root = temp_root("roundtrip");
    let store = PluginStore::new(&root);
    let bytes = component();
    let manifest = wasm_manifest("good-plugin", &sha256_of(&bytes));

    let installed = store
        .install(&manifest, Some(&bytes))
        .expect("valid install must succeed");
    assert!(
        installed.wasm_path().is_some(),
        "a wasm install must leave the component on disk"
    );
    // The host manifest is what the runtime loads, so it must point at that file.
    let host = installed.host_manifest();
    assert_eq!(host.id.0, "good-plugin");
    assert_eq!(host.version, "1.0.0");

    let listed = store.list().expect("list");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].manifest.name, "good-plugin");
    assert_eq!(listed[0].manifest.models, ["custom-*"]);

    assert!(store.get("good-plugin").expect("get").is_some());
    assert!(store.get("absent").expect("get").is_none());

    store.remove("good-plugin").expect("remove");
    assert!(store.list().expect("list").is_empty());
    assert!(
        store.remove("good-plugin").is_err(),
        "removing twice must be an error, not a silent no-op"
    );
    fs::remove_dir_all(&root).ok();
}

/// Refutes: listing a directory that was left half-written, which would surface a
/// plugin the gateway cannot actually load.
#[test]
fn incomplete_install_is_not_listed() {
    let root = temp_root("partial");
    let store = PluginStore::new(&root);
    // A component with no manifest beside it: what a crashed install leaves.
    let dir = root.join("half-done");
    fs::create_dir_all(&dir).expect("dir");
    fs::write(dir.join("plugin.wasm"), component()).expect("write");

    assert!(
        store.list().expect("list").is_empty(),
        "a directory without a manifest is an incomplete install"
    );
    fs::remove_dir_all(&root).ok();
}

/// Refutes: pretending to install a crate plugin. It is compiled into the binary,
/// so the CLI has to say that rather than fail obscurely.
#[test]
fn crate_plugins_are_reported_as_needing_a_rebuild() {
    let root = temp_root("crate");
    let store = PluginStore::new(&root);
    let manifest = RegistryManifest::from_yaml(
        r#"
name: compiled-in
version: "1.0.0"
kind: provider
description: "compiled provider"
license: MIT
install:
  crate_name: vkdg-provider-example
  crate_version: "0.1.0"
"#,
    )
    .expect("manifest parses");

    let err = store
        .install(&manifest, None)
        .expect_err("a crate plugin cannot be installed at runtime");
    assert!(
        err.to_string().contains("compiled into the binary"),
        "error must explain why: {err}"
    );
    fs::remove_dir_all(&root).ok();
}

/// Refutes: a config-only plugin needing a component. Its whole point is zero code.
#[test]
fn config_only_plugin_installs_without_a_component() {
    let root = temp_root("configonly");
    let store = PluginStore::new(&root);
    let manifest = RegistryManifest::from_yaml(
        r#"
name: openrouter
version: "1.0.0"
kind: provider
description: "OpenAI-compatible aggregator"
license: MIT
models: ["openrouter/*"]
install:
  config_snippet: |
    provider: openai-compat
    base_url: https://openrouter.ai/api
"#,
    )
    .expect("manifest parses");

    let installed = store
        .install(&manifest, None)
        .expect("config-only install needs no bytes");
    assert!(
        installed.wasm_path().is_none(),
        "a config-only plugin must not write a component"
    );
    assert_eq!(store.list().expect("list").len(), 1);
    fs::remove_dir_all(&root).ok();
}
