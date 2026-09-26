//! VKDG provider: Fireworks AI (OpenAI-compatible endpoint).
use vkdg_provider_sdk::openai_compat::OpenAiCompatAdapter;

pub static PROVIDER: OpenAiCompatAdapter = OpenAiCompatAdapter::new(
    "fireworks",
    "Fireworks AI",
    "https://api.fireworks.ai/inference",
    "accounts/fireworks/models/llama-v3p3-70b-instruct",
);
