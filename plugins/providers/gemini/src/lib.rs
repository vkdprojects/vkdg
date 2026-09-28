//! VKDG provider: Google Gemini (OpenAI-compatible endpoint).
use vkdg_provider_sdk::openai_compat::OpenAiCompatAdapter;

pub fn provider() -> OpenAiCompatAdapter {
    OpenAiCompatAdapter::new(
        "gemini",
        "Google Gemini",
        "https://generativelanguage.googleapis.com/v1beta/openai",
        "gemini-2.0-flash",
    )
    .with_meta(
        'G',
        "#4285F4",
        Some("https://aistudio.google.com"),
        Some("Google Gemini, multimodal, free tier."),
    )
}
