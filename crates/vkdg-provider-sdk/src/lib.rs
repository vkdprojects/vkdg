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
pub mod registry;
pub mod request;

pub use error::ProviderError;
pub use oauth::{OAuthConfig, OAuthFlow, OAuthProvider, TokenPair};
pub use registry::ProviderRegistry;
pub use request::{PreparedRequest, ProviderAdapter};
