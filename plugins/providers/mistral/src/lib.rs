//! VKDG provider: Mistral AI (OpenAI-compatible endpoint).
use vkdg_provider_sdk::openai_compat::OpenAiCompatAdapter;

pub fn provider() -> OpenAiCompatAdapter {
    OpenAiCompatAdapter::new(
        "mistral",
        "Mistral AI",
        "https://api.mistral.ai",
        "mistral-large-latest",
    )
    .with_meta(
        'M',
        "#FF6F00",
        Some("https://console.mistral.ai"),
        Some("European frontier models."),
    )
}
