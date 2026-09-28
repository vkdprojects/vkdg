//! VKDG Provider SDK — trait definitions for provider plugins.
//!
//! Implement [`ProviderAdapter`] to add a new provider.
//! Implement [`OAuthProvider`] in addition for OAuth-backed connections.
//!
//! # Plugin philosophy
//! Every provider is a separate crate under `plugins/providers/<name>/`.
//! Adding a provider never requires modifying the VKDG core.
//! Register the provider with [`ProviderRegistry`].

pub mod error;
pub mod oauth;
pub mod openai_compat;
pub mod registry;
pub mod request;

pub use error::ProviderError;
pub use oauth::{
    find_login_method, resolve_login_params, run_device_login, DeviceAuthorization, DevicePoll,
    LoginField, LoginMethod, LoginParams, LoginResult, LoginState, OAuthConfig, OAuthFlow,
    OAuthProvider, PkceAuthorization, TokenPair,
};
pub use registry::ProviderRegistry;
pub use request::{upstream_model, ConversationStreamDecoder, PreparedRequest, ProviderAdapter};
pub use vkdg_connections::Credential;
