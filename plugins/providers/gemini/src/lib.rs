//! VKDG provider: Google Gemini (OpenAI-compatible endpoint).
use vkdg_provider_sdk::openai_compat::OpenAiCompatAdapter;

pub static PROVIDER: OpenAiCompatAdapter = OpenAiCompatAdapter::new(
    "gemini",
    "Google Gemini",
    "https://generativelanguage.googleapis.com/v1beta/openai",
    "gemini-2.0-flash",
);
