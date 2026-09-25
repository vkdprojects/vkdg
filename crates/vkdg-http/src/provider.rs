use bytes::Bytes;
use http::HeaderMap;
use vkdg_connections::ConnectionConfig;
use vkdg_core::VkdgError;
use vkdg_operations::Operation;

/// Fields the pipeline needs to send an upstream request.
/// Produced by [`ProviderAdapter::prepare`]; the pipeline core knows nothing
/// about wire formats, auth headers, or URL structure.
pub struct PreparedRequest {
    pub url: String,
    pub headers: HeaderMap,
    pub body: Bytes,
    pub is_streaming: bool,
}

/// Contract every provider adapter must satisfy.
/// The pipeline calls `prepare()` once per request; the adapter handles all
/// provider-specific serialisation, auth header injection, and URL resolution.
pub trait ProviderAdapter: Send + Sync {
    fn name(&self) -> &str;

    fn prepare(
        &self,
        operation: &Operation,
        config: &ConnectionConfig,
        token: &str,
    ) -> Result<PreparedRequest, VkdgError>;
}
