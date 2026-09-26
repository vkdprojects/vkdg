//! VKDG provider: Together AI (OpenAI-compatible endpoint).
use vkdg_provider_sdk::openai_compat::OpenAiCompatAdapter;

pub fn provider() -> OpenAiCompatAdapter {
    OpenAiCompatAdapter::new(
        "together",
        "Together AI",
        "https://api.together.xyz",
        "meta-llama/Llama-3.3-70B-Instruct-Turbo",
    )
}
