//! VKDG provider: Fireworks AI (OpenAI-compatible endpoint).
use vkdg_provider_sdk::openai_compat::OpenAiCompatAdapter;

pub fn provider() -> OpenAiCompatAdapter {
    OpenAiCompatAdapter::new(
        "fireworks",
        "Fireworks AI",
        "https://api.fireworks.ai/inference",
        "accounts/fireworks/models/llama-v3p3-70b-instruct",
    )
    .with_meta(
        'F',
        "#FF4A00",
        Some("https://fireworks.ai"),
        Some("Fast model serving API."),
    )
}
