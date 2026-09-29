//! Bridges a WASM component to the [`ProviderAdapter`] trait.
//!
//! This is what makes a community plugin indistinguishable from a first-party one:
//! the pipeline resolves both through `ProviderRegistry` by id and calls the same
//! trait, so a provider can ship as a `.wasm` and be swapped without rebuilding
//! the gateway.
//!
//! Values cross the boundary as JSON. WIT records would be marginally cheaper, but
//! JSON keeps the plugin ABI stable while the internal Rust types keep moving, and
//! a provider call is already dominated by an upstream HTTP round trip.

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use vkdg_connections::{ConnectionConfig, Credential};
use vkdg_operations::Operation;
use vkdg_provider_sdk::{PreparedRequest, ProviderAdapter, ProviderError};

use crate::{PluginManifest, WasmPluginInstance};

/// Export names the component must provide, mirroring `wit/provider.wit`.
mod export {
    pub const NAME: &str = "name";
    pub const DISPLAY_NAME: &str = "display-name";
    pub const MODEL_PATTERNS: &str = "model-patterns";
    pub const PREPARE: &str = "prepare";
}

/// What the gateway hands a plugin for `prepare`.
///
/// The connection is passed without credentials; the token travels in `credential`
/// so a plugin cannot accidentally read one out of config it was not given.
#[derive(Serialize)]
struct PrepareInput<'a> {
    operation: &'a Operation,
    connection: ConnectionJson<'a>,
    credential: CredentialJson<'a>,
}

/// Connection fields a plugin is allowed to see.
#[derive(Serialize)]
struct ConnectionJson<'a> {
    id: &'a str,
    provider: &'a str,
    models: &'a [String],
    max_concurrent: u32,
    weight: u32,
    tags: &'a [String],
    base_url: Option<&'a str>,
}

#[derive(Serialize)]
struct CredentialJson<'a> {
    token: &'a str,
    extra: &'a HashMap<String, String>,
}

/// What a plugin returns from `prepare`.
#[derive(Deserialize)]
struct PreparedRequestJson {
    url: String,
    #[serde(default)]
    headers: Vec<(String, String)>,
    /// Body bytes. A plugin sending JSON emits its UTF-8 bytes here.
    #[serde(default)]
    body: Vec<u8>,
    #[serde(default)]
    is_streaming: bool,
}

/// A provider adapter backed by a WASM component.
pub struct WasmProviderAdapter {
    id: String,
    display_name: String,
    model_patterns: Vec<String>,
    instance: Arc<WasmPluginInstance>,
}

impl WasmProviderAdapter {
    /// Build an adapter from an already-compiled component.
    ///
    /// Identity and model patterns are read once here rather than per request:
    /// they are static for the life of the plugin, and `ProviderAdapter::id`
    /// cannot fail.
    pub fn new(
        instance: Arc<WasmPluginInstance>,
        manifest: &PluginManifest,
    ) -> Result<Self, String> {
        // Fall back to the manifest id when the component does not export `name`,
        // so a minimal plugin still registers under a predictable key.
        let id = instance
            .call_string_fn(export::NAME, "")
            .unwrap_or_else(|_| manifest.id.0.clone());
        let display_name = instance
            .call_string_fn(export::DISPLAY_NAME, "")
            .unwrap_or_else(|_| id.clone());
        let model_patterns = instance
            .call_string_fn(export::MODEL_PATTERNS, "")
            .ok()
            .and_then(|json| serde_json::from_str::<Vec<String>>(&json).ok())
            .unwrap_or_default();

        if id.trim().is_empty() {
            return Err("plugin reported an empty provider id".to_owned());
        }
        Ok(Self {
            id,
            display_name,
            model_patterns,
            instance,
        })
    }

    /// Model ids or globs this plugin claims.
    pub fn model_patterns(&self) -> &[String] {
        &self.model_patterns
    }
}

impl ProviderAdapter for WasmProviderAdapter {
    fn id(&self) -> &str {
        &self.id
    }

    fn display_name(&self) -> &str {
        &self.display_name
    }

    fn prepare(
        &self,
        operation: &Operation,
        config: &ConnectionConfig,
        credential: &Credential,
    ) -> Result<PreparedRequest, ProviderError> {
        let input = PrepareInput {
            operation,
            connection: ConnectionJson {
                id: &config.id.0,
                provider: config.provider.adapter_id(),
                models: &config.models,
                max_concurrent: config.max_concurrent,
                weight: config.weight,
                tags: &config.tags,
                base_url: None,
            },
            credential: CredentialJson {
                token: &credential.token,
                extra: credential.extra.as_ref(),
            },
        };
        let payload = serde_json::to_string(&input)
            .map_err(|e| ProviderError::Serialization(e.to_string()))?;

        let out = self
            .instance
            .call_string_fn(export::PREPARE, &payload)
            .map_err(ProviderError::Http)?;
        let prepared: PreparedRequestJson =
            serde_json::from_str(&out).map_err(|e| ProviderError::Serialization(e.to_string()))?;

        let mut headers = http::HeaderMap::new();
        for (name, value) in &prepared.headers {
            let name = http::HeaderName::try_from(name.as_str())
                .map_err(|_| ProviderError::Http(format!("plugin sent invalid header {name:?}")))?;
            let value = http::HeaderValue::from_str(value)
                .map_err(|_| ProviderError::Http(format!("invalid value for header {name:?}")))?;
            headers.insert(name, value);
        }

        Ok(PreparedRequest {
            url: prepared.url,
            headers,
            body: prepared.body.into(),
            is_streaming: prepared.is_streaming,
        })
    }
}
