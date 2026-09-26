//! VKDG provider: SambaNova Cloud (OpenAI-compatible, free tier).
use vkdg_provider_sdk::openai_compat::OpenAiCompatAdapter;

pub fn provider() -> OpenAiCompatAdapter {
    OpenAiCompatAdapter::new(
        "sambanova",
        "SambaNova",
        "https://api.sambanova.ai",
        "Meta-Llama-3.1-70B-Instruct",
    )
}
