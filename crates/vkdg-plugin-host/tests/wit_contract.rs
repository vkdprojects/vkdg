//! The plugin ABI in `wit/` is a shipped contract, so it is tested like code.
//!
//! Two things are checked: the package parses (a malformed `.wit` would otherwise
//! only fail when someone tries to build a plugin), and the provider interface
//! still exposes every capability a real provider needs. The second part exists
//! because `plugins/providers/kiro` grew past what the original contract could
//! express — interactive login, per-account data, a binary wire protocol — and a
//! community plugin must be able to do the same without a gateway release.

use std::path::PathBuf;

use wit_parser::{Resolve, WorldItem};

fn wit_dir() -> PathBuf {
    // CARGO_MANIFEST_DIR is crates/vkdg-plugin-host.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../wit")
        .canonicalize()
        .expect("wit/ directory")
}

/// Parses `wit/`, returning the resolved package.
fn resolve() -> Resolve {
    let mut resolve = Resolve::default();
    resolve
        .push_dir(wit_dir())
        .unwrap_or_else(|e| panic!("wit/ must parse: {e:#}"));
    resolve
}

/// Refutes: a syntax error or dangling type reference landing in the shipped ABI.
#[test]
fn wit_package_parses() {
    let resolve = resolve();
    assert!(
        resolve.worlds.len() > 0,
        "wit/ must define at least one world"
    );
}

/// Every world named in `wit/` must resolve, so each plugin role is buildable.
#[test]
fn every_declared_world_resolves() {
    let resolve = resolve();
    let names: Vec<&str> = resolve
        .worlds
        .iter()
        .map(|(_, w)| w.name.as_str())
        .collect();
    // `vkdg-plugin` is the multi-role world; the rest are single-role.
    for expected in [
        "provider",
        "compressor",
        "router",
        "cache-backend",
        "auth",
        "vkdg-plugin",
    ] {
        assert!(
            names.contains(&expected),
            "world `{expected}` missing; found {names:?}"
        );
    }
}

/// Refutes: a contract that cannot express what a first-party provider does.
///
/// Each function below is required by `plugins/providers/kiro`. If one is missing,
/// a community plugin cannot reach parity and the contract is the bug.
#[test]
fn provider_contract_covers_real_provider_needs() {
    let resolve = resolve();
    let (_, world) = resolve
        .worlds
        .iter()
        .find(|(_, w)| w.name == "provider")
        .expect("provider world");

    let mut functions: Vec<String> = Vec::new();
    for item in world.exports.values() {
        if let WorldItem::Interface { id, .. } = item {
            let iface = &resolve.interfaces[*id];
            functions.extend(iface.functions.keys().cloned());
        }
    }

    for required in [
        // Identity and routing.
        "name",
        "display-name",
        "model-patterns",
        // The plugin chooses the URL, so it can route by account type or region.
        "prepare",
        "decode-response",
        // Raw bytes in, events out: lets a plugin implement a binary protocol.
        "decode-chunk",
        // Providers that send no terminal event close the stream here.
        "finish-stream",
        // Catalogues move faster than gateway releases.
        "list-models",
        // Interactive login, so credentials are not limited to config.
        "login-methods",
        "start-device-login",
        "poll-device-login",
        "start-pkce-login",
        "finish-pkce-login",
        "import-token",
        "refresh-token",
    ] {
        assert!(
            functions.iter().any(|f| f == required),
            "provider contract is missing `{required}`; a plugin could not match \
             the first-party Kiro adapter. Present: {functions:?}"
        );
    }
}
