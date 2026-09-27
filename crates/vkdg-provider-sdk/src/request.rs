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
}

/// Every provider adapter must implement this trait.
///
/// # Implementation
/// - `id()` must be globally unique and stable (used as registry key and in configs)
/// - `prepare()` is called once per request on the hot path — no allocations beyond HeaderMap + body serialization
/// - Never call upstream from prepare(); that is the pipeline's job
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

    /// List prices per model, specific patterns first. Empty (the default)
    /// means the provider bills some other way (subscription, free tier) and
    /// its requests have no per-token cost, shown as unknown rather than $0.
    fn prices(&self) -> &[vkdg_core::pricing::ModelPrice] {
        &[]
    }
}
