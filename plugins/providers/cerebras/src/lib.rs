//! VKDG provider: Cerebras (OpenAI-compatible, free tier).
use vkdg_provider_sdk::openai_compat::OpenAiCompatAdapter;

pub fn provider() -> OpenAiCompatAdapter {
    OpenAiCompatAdapter::new(
        "cerebras",
        "Cerebras",
        "https://api.cerebras.ai/v1",
        "llama3.1-70b",
    )
}
