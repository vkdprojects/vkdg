use bytes::Bytes;
use http::HeaderMap;
use vkdg_connections::ConnectionConfig;
use vkdg_operations::{ConversationEvent, Operation};

use crate::ProviderError;

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
    fn prepare(
        &self,
        operation: &Operation,
        config: &ConnectionConfig,
        token: &str,
    ) -> Result<PreparedRequest, ProviderError>;

    /// Optional stream decoder for providers that need protocol translation.
    /// Return `None` for passthrough (default behavior).
    fn stream_decoder(&self) -> Option<Box<dyn ConversationStreamDecoder>> {
        None
    }
}