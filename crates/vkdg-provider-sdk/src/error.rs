/// Errors returned by provider adapter operations.
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("http: {0}")]
    Http(String),
    #[error("token refresh failed: {0}")]
    TokenRefresh(String),
    #[error("unsupported operation for this provider")]
    UnsupportedOperation,
    #[error("config error: {0}")]
    Config(String),
    #[error("serialization: {0}")]
    Serialization(String),
}
