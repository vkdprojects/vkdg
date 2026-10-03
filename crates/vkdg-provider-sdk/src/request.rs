use bytes::Bytes;
use http::HeaderMap;
use vkdg_connections::{ConnectionConfig, Credential};
use vkdg_operations::{ConversationEvent, Operation};

use crate::{OAuthProvider, ProviderError};

/// The assembled upstream HTTP request. Produced by [`ProviderAdapter::prepare`].
/// The pipeline core knows nothing about wire formats, auth headers, or URLs.
pub struct PreparedRequest {
    pub url: String,
    pub headers: HeaderMap,
    pub body: Bytes,
    pub is_streaming: bool,
}

/// The model id to send upstream: the one the client asked for (after any
/// combo rewrite), or `default` when the request names none.
///
/// Never read `ConnectionConfig::models` for this. That list holds route
/// patterns (`claude-*`), so a glob would go upstream, and a multi-model
/// connection would pin every call to its first entry.
pub fn upstream_model<'a>(
    req: &'a vkdg_operations::ConversationRequest,
    default: &'a str,
) -> &'a str {
    let m = req.model.trim();
    if m.is_empty() || m.contains(['*', '?']) {
        default
    } else {
        m
    }
}

/// Decoder for provider-specific streaming protocols.
/// Used when the provider returns raw bytes that need to be decoded into
/// [`ConversationEvent`]s before re-encoding to the client's SSE format.
pub trait ConversationStreamDecoder: Send {
    /// Feed raw bytes from the upstream stream.
    /// Returns decoded events. Must handle partial frames (incremental parsing).
    fn feed(&mut self, chunk: bytes::Bytes) -> Vec<ConversationEvent>;

    /// Called once when the upstream byte stream ends. Returns trailing events
    /// for protocols without an explicit terminal frame (e.g. Kiro).
    fn finish(&mut self) -> Vec<ConversationEvent> {
        Vec::new()
    }

    /// Kiro-specific: percentage of context window used, if the provider reported it.
    /// Default is `None` (most providers do not report this).
    fn context_usage_pct(&self) -> Option<f64> {
        None
    }
}

/// Every provider adapter must implement this trait.
///
/// # Implementation
/// - `id()` must be globally unique and stable (used as registry key and in configs)
/// - `prepare()` is called once per request on the hot path — no allocations beyond `HeaderMap` + body serialization
/// - Never call upstream from `prepare()`; that is the pipeline's job
pub trait ProviderAdapter: Send + Sync {
    /// Stable identifier: "anthropic", "openai", "gemini", "groq", etc.
    fn id(&self) -> &str;

    /// Human-readable name for admin UI.
    fn display_name(&self) -> &str;

    /// Assemble the upstream request for this operation.
    ///
    /// `credential.token` is the API key or access token; `credential.extra` holds
    /// per-account plugin data persisted at login/refresh (empty for API keys).
    fn prepare(
        &self,
        operation: &Operation,
        config: &ConnectionConfig,
        credential: &Credential,
    ) -> Result<PreparedRequest, ProviderError>;

    /// OAuth capability (login + refresh). Return `Some(self)` when the plugin
    /// implements [`OAuthProvider`]; `None` means API-key only.
    fn oauth(&self) -> Option<&dyn OAuthProvider> {
        None
    }

    /// Optional stream decoder for providers that need protocol translation.
    /// Return `None` for passthrough (default behavior).
    fn stream_decoder(&self) -> Option<Box<dyn ConversationStreamDecoder>> {
        None
    }

    /// Account credit / quota reporting. Return `Some(self)` when the plugin
    /// implements [`UsageProvider`](crate::UsageProvider); `None` means the
    /// provider exposes no usage figures.
    fn usage(&self) -> Option<&dyn crate::UsageProvider> {
        None
    }

    /// Live model discovery. Return `Some(self)` when the plugin implements
    /// [`ModelCatalog`](crate::ModelCatalog); `None` means the provider cannot
    /// enumerate models and connections keep their declared patterns.
    fn model_catalog(&self) -> Option<&dyn crate::ModelCatalog> {
        None
    }

    /// List prices per model, specific patterns first. Empty (the default)
    /// means the provider bills some other way (subscription, free tier) and
    /// its requests have no per-token cost, shown as unknown rather than $0.
    fn prices(&self) -> &[vkdg_core::pricing::ModelPrice] {
        &[]
    }

    /// Model patterns for the connection created automatically when an account
    /// of this provider is connected (`claude-*`, `gpt-*`, exact ids). Choose
    /// patterns that only this provider serves: the router round-robins across
    /// every connection whose patterns match, so `*` would also receive other
    /// providers' models. The default `*` is for plugins that do not say.
    /// Return an empty list when the provider cannot serve requests yet: no
    /// connection is created and the operator adds one by hand.
    fn default_models(&self) -> Vec<String> {
        vec!["*".to_owned()]
    }

    /// The dialect this provider's conversation responses use for `config`.
    /// Declared, the gateway translates responses (stream and not) to the
    /// client's dialect when they differ, and passes bytes through untouched when
    /// they match. `None` (the default) never translates: right for providers that
    /// speak another protocol and decode it themselves ([`Self::stream_decoder`]).
    fn wire_format(&self, _config: &ConnectionConfig) -> Option<vkdg_operations::WireFormat> {
        None
    }

    /// Metadata for the admin console UI. Defaults work for any plugin that
    /// only implements `id()` and `display_name()`.
    fn meta(&self) -> ProviderMeta {
        ProviderMeta {
            icon_char: self.id().chars().next().unwrap_or('?').to_ascii_uppercase(),
            icon_color: "#6b7280",
            category: ProviderCategory::LlmApi,
            site_url: None,
            description: None,
        }
    }
}

/// Visual and discovery metadata for the admin console UI.
/// Returned by `ProviderAdapter::meta()` and serialised into
/// `GET /admin/v1/providers/oauth` and `GET /admin/v1/providers`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProviderMeta {
    /// Single character used as an icon when no image is available.
    pub icon_char: char,
    /// CSS color string for the icon background (e.g. `"#f97316"`).
    pub icon_color: &'static str,
    pub category: ProviderCategory,
    /// Public website or docs URL.
    pub site_url: Option<&'static str>,
    /// One-liner shown under the provider name in the grid.
    pub description: Option<&'static str>,
}

/// Broad category used to group providers in the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderCategory {
    /// Hosted LLM with an API key.
    LlmApi,
    /// OAuth / device-code account login (AI IDE assistants, etc.).
    OauthIde,
    /// Self-hosted or OpenAI-compatible endpoint.
    Compatible,
}
