//! VKDG provider: NVIDIA NIM (OpenAI-compatible, free tier).
use vkdg_provider_sdk::openai_compat::OpenAiCompatAdapter;

pub fn provider() -> OpenAiCompatAdapter {
    OpenAiCompatAdapter::new(
        "nvidia-nim",
        "NVIDIA NIM",
        "https://integrate.api.nvidia.com/v1",
        "meta/llama-3.1-70b-instruct",
    )
}
