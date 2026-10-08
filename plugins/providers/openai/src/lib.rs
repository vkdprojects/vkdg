//! OpenAI provider adapter — implements [`ProviderAdapter`] for the OpenAI
//! Chat Completions wire format, plus SSE upstream decode.

pub mod decode;
pub mod prepare;

pub use prepare::OpenAIAdapter;
