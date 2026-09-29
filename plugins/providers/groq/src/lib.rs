//! VKDG provider: Groq (OpenAI-compatible endpoint).
use vkdg_provider_sdk::openai_compat::OpenAiCompatAdapter;

pub fn provider() -> OpenAiCompatAdapter {
    OpenAiCompatAdapter::new(
        "groq",
        "Groq",
        "https://api.groq.com/openai",
        "llama-3.3-70b-versatile",
    )
    .with_meta(
        'G',
        "#f97316",
        Some("https://console.groq.com"),
        Some("Fast inference via LPU hardware."),
    )
}
