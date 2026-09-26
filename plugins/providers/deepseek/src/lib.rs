//! VKDG provider: DeepSeek (OpenAI-compatible endpoint).
use vkdg_provider_sdk::openai_compat::OpenAiCompatAdapter;

pub fn provider() -> OpenAiCompatAdapter {
    OpenAiCompatAdapter::new(
        "deepseek",
        "DeepSeek",
        "https://api.deepseek.com",
        "deepseek-chat",
    )
}
