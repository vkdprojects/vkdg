//! VKDG provider: DeepSeek (OpenAI-compatible endpoint).
use vkdg_provider_sdk::openai_compat::OpenAiCompatAdapter;

pub static PROVIDER: OpenAiCompatAdapter = OpenAiCompatAdapter::new(
    "deepseek",
    "DeepSeek",
    "https://api.deepseek.com",
    "deepseek-chat",
);
