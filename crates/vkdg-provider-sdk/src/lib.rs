//! VKDG Provider SDK — trait definitions for provider plugins.
//!
//! Implement [`ProviderAdapter`] to add a new provider.
//! Implement [`OAuthProvider`] in addition for OAuth-backed connections.
//!
//! # Plugin philosophy
//! Every provider is a separate crate under `plugins/providers/<name>/`.
//! Adding a provider never requires modifying the VKDG core.
//! Register the provider with [`ProviderRegistry`].

pub mod anthropic_messages;
pub mod dialect;
pub mod dynamic_catalog;
pub mod error;
pub mod model_catalog;
pub mod oauth;
pub mod openai_compat;
pub mod pkce;
pub mod registry;
pub mod request;
mod sampling_json;
mod turns;
pub mod usage;

pub use dialect::{
    decode_anthropic_json, decode_openai_json, json_decoder_for, sse_decoder_for,
    AnthropicSseDecoder, FrameError, JsonDecoder, OpenAiSseDecoder, SseFrame, SseFramer,
    DEFAULT_MAX_EVENT_BYTES, DEFAULT_MAX_TOOL_ARGUMENT_BYTES,
};
pub use dynamic_catalog::DynamicModelCatalog;
pub use error::ProviderError;
pub use model_catalog::ModelCatalog;
pub use oauth::{
    find_login_method, resolve_login_params, run_device_login, DeviceAuthorization, DevicePoll,
    LoginField, LoginMethod, LoginParams, LoginResult, LoginState, OAuthConfig, OAuthFlow,
    OAuthProvider, PkceAuthorization, TokenPair,
};
pub use pkce::{
    build_pkce_authorization, parse_pkce_callback, random_b64url, validated_loopback_redirect,
};
pub use registry::ProviderRegistry;
pub use request::{
    upstream_model, ConversationStreamDecoder, PreparedRequest, ProviderAdapter, ProviderCategory,
    ProviderMeta,
};
pub use usage::{UsageProvider, UsageSnapshot};
pub use vkdg_connections::Credential;
