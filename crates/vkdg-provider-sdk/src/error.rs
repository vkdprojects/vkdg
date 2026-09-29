/// Errors returned by provider adapter operations.
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("http: {0}")]
    Http(String),
    #[error("token refresh failed: {0}")]
    TokenRefresh(String),
    /// The upstream rejected the refresh token itself (revoked, rotated by another
    /// client, expired). Distinct from `TokenRefresh`, which covers transient
    /// failures worth retrying: this one means the account needs a new login.
    #[error("credential revoked ({status}): {message}")]
    CredentialRevoked { status: u16, message: String },
    #[error("unsupported operation for this provider")]
    UnsupportedOperation,
    #[error("config error: {0}")]
    Config(String),
    #[error("serialization: {0}")]
    Serialization(String),
}
