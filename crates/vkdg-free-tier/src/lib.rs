//! Free-tier provider catalog for VKDG.
//!
//! Registers known free-tier AI providers as VKDG connections.
//! Each provider requires only its API key as an env var.
//!
//! # Included providers
//!
//! | Provider | Env var | Notable free models |
//! |---|---|---|
//! | Groq | `GROQ_API_KEY` | llama-3.1-70b-versatile, gemma2-9b-it |
//! | Together.ai | `TOGETHER_API_KEY` | Llama-3.2-11B-Vision, Qwen2.5-72B |
//! | Fireworks | `FIREWORKS_API_KEY` | llama-v3p1-70b-instruct |
//!
//! # Usage
//!
//! ```toml
//! # vkdg.toml — no [[connections]] needed for free providers
//! [[routes]]
//! id = "free-fallback"
//! match_models = ["llama-*", "gemma-*", "qwen-*"]
//! strategy = "fallback_chain"
//! targets = ["groq-default", "together-default"]
//! ```
//!
//! Then install the free-tier plugin and set env vars:
//! ```bash
//! export GROQ_API_KEY=gsk_...
//! export TOGETHER_API_KEY=...
//! ```

pub mod catalog;

pub use catalog::{free_tier_connections, FIREWORKS_MODELS, GROQ_MODELS, TOGETHER_MODELS};
