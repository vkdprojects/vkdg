//! Plugin management over the admin API, backing the console's plugin screen.
//!
//! These endpoints wrap the same [`PluginStore`] the CLI uses, so installing from
//! the console and from `vkdg plugin install` land in one place with one set of
//! checks. Nothing here parses a manifest by hand: it goes through
//! `RegistryManifest`, which enforces the checksum and name rules.
//!
//! Installing runs untrusted bytes through a compile step, so it is admin-only
//! like every other mutating endpoint.

use axum::{
    extract::{Path, State},
    response::{IntoResponse, Response},
    Json,
};
use http::{HeaderMap, StatusCode};
use serde::{Deserialize, Serialize};
use vkdg_plugin_host::{PluginStore, RegistryManifest};

use crate::{
    error::{AdminError, AdminErrorResponse},
    handlers::session::get_session,
    router::AdminState,
};

/// An installed plugin as the console shows it.
#[derive(Serialize)]
pub struct PluginSummary {
    name: String,
    version: String,
    kind: String,
    description: String,
    tags: Vec<String>,
    models: Vec<String>,
    /// False for plugins compiled into the binary, which cannot be removed here.
    removable: bool,
}

#[derive(Serialize)]
pub struct PluginList {
    items: Vec<PluginSummary>,
    total: usize,
    /// Where plugins live, so the console can tell the operator.
    directory: String,
}

/// Install request: a manifest plus, for a wasm plugin, the component bytes.
///
/// Bytes arrive base64-encoded because the manifest travels as JSON; the console
/// already has the file in hand from a file picker.
#[derive(Deserialize)]
pub struct InstallRequest {
    /// The manifest YAML, exactly as published.
    manifest: String,
    /// Base64 `.wasm`. Required for a wasm install, ignored otherwise.
    #[serde(default)]
    wasm_base64: Option<String>,
}

fn unauthorized() -> Response {
    AdminErrorResponse(StatusCode::UNAUTHORIZED, AdminError::unauthorized()).into_response()
}

fn bad_request(message: impl Into<String>) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({ "error": message.into() })),
    )
        .into_response()
}

pub async fn list_plugins(State(state): State<AdminState>, headers: HeaderMap) -> Response {
    if get_session(&state, &headers).is_none() {
        return unauthorized();
    }
    let store = PluginStore::from_env();
    let installed = match store.list() {
        Ok(list) => list,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response()
        }
    };
    let items: Vec<PluginSummary> = installed
        .iter()
        .map(|p| PluginSummary {
            name: p.manifest.name.clone(),
            version: p.manifest.version.clone(),
            kind: format!("{:?}", p.manifest.kind),
            description: p.manifest.description.clone(),
            tags: p.manifest.tags.clone(),
            models: p.manifest.models.clone(),
            removable: true,
        })
        .collect();
    let total = items.len();
    Json(PluginList {
        items,
        total,
        directory: store.root().display().to_string(),
    })
    .into_response()
}

pub async fn install_plugin(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Json(req): Json<InstallRequest>,
) -> Response {
    if get_session(&state, &headers).is_none() {
        return unauthorized();
    }
    let manifest = match RegistryManifest::from_yaml(&req.manifest) {
        Ok(m) => m,
        Err(e) => return bad_request(e.to_string()),
    };
    let wasm = match req.wasm_base64.as_deref().map(decode_base64) {
        Some(Ok(bytes)) => Some(bytes),
        Some(Err(e)) => return bad_request(format!("wasm_base64: {e}")),
        None => None,
    };

    let store = PluginStore::from_env();
    // The store verifies the checksum and compiles the component before writing,
    // so a bad upload cannot replace a working plugin.
    match store.install(&manifest, wasm.as_deref()) {
        Ok(installed) => (
            StatusCode::CREATED,
            Json(serde_json::json!({
                "name": installed.manifest.name,
                "version": installed.manifest.version,
                "directory": installed.dir.display().to_string(),
            })),
        )
            .into_response(),
        Err(e) => bad_request(e.to_string()),
    }
}

pub async fn remove_plugin(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Response {
    if get_session(&state, &headers).is_none() {
        return unauthorized();
    }
    let store = PluginStore::from_env();
    match store.remove(&name) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// Decode standard base64, which is what a browser's `btoa`/`FileReader` produces.
///
/// Hand-rolled to avoid a dependency for one call site; rejects any character
/// outside the alphabet so a truncated upload fails loudly.
fn decode_base64(input: &str) -> Result<Vec<u8>, String> {
    const INVALID: u8 = 0xFF;
    fn value(c: u8) -> u8 {
        match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => INVALID,
        }
    }

    let cleaned: Vec<u8> = input.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    let body = cleaned
        .strip_suffix(b"==")
        .unwrap_or_else(|| cleaned.strip_suffix(b"=").unwrap_or(&cleaned));
    let pad = cleaned.len() - body.len();
    if cleaned.len() % 4 != 0 {
        return Err("length is not a multiple of 4".into());
    }

    let mut out = Vec::with_capacity(body.len() / 4 * 3);
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    for &c in body {
        let v = value(c);
        if v == INVALID {
            return Err(format!("invalid character {:?}", c as char));
        }
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    // Padding accounts for the bits that do not complete a byte.
    let _ = pad;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Refutes: a decoder that silently accepts corrupt input, which would install
    /// a truncated component.
    #[test]
    fn base64_round_trips_and_rejects_corruption() {
        // "hello" and a byte string with padding.
        assert_eq!(decode_base64("aGVsbG8=").unwrap(), b"hello");
        assert_eq!(decode_base64("YQ==").unwrap(), b"a");
        assert_eq!(decode_base64("").unwrap(), b"");
        // Whitespace from a wrapped upload is tolerated.
        assert_eq!(decode_base64("aGVs\nbG8=").unwrap(), b"hello");

        assert!(decode_base64("aGVsbG8").is_err(), "bad length");
        assert!(decode_base64("aGVs!G8=").is_err(), "invalid character");
    }
}
