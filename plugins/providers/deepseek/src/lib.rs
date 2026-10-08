//! VKDG provider: DeepSeek (OpenAI-compatible endpoint).
use vkdg_provider_sdk::openai_compat::OpenAiCompatAdapter;

pub fn provider() -> OpenAiCompatAdapter {
    OpenAiCompatAdapter::new(
        "deepseek",
        "DeepSeek",
        "https://api.deepseek.com",
        "deepseek-chat",
    )
    .with_meta(
        'D',
        "#4B9EF6",
        Some("https://platform.deepseek.com"),
        Some("Open-source frontier models."),
    )
}
