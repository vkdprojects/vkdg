//! VKDG provider: Groq (OpenAI-compatible endpoint).
use vkdg_provider_sdk::openai_compat::OpenAiCompatAdapter;

pub static PROVIDER: OpenAiCompatAdapter = OpenAiCompatAdapter::new(
    "groq",
    "Groq",
    "https://api.groq.com/openai",
    "llama-3.3-70b-versatile",
);
