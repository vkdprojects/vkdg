//! VKDG provider: Mistral AI (OpenAI-compatible endpoint).
use vkdg_provider_sdk::openai_compat::OpenAiCompatAdapter;

pub fn provider() -> OpenAiCompatAdapter {
    OpenAiCompatAdapter::new(
        "mistral",
        "Mistral AI",
        "https://api.mistral.ai",
        "mistral-large-latest",
    )
}
