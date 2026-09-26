//! Free-tier provider definitions.
//!
//! Each provider is declared as a static slice of ConnectionConfig.
//! Operators call free_tier_connections() to get the full list,
//! then add them to their ConnectionCatalog.

use vkdg_connections::{AuthKind, ConnectionConfig, ProviderKind};
use vkdg_core::{CapabilitySet, ConnectionId};

/// Models available on Groq free tier (as of 2026).
/// Rate limits apply; see https://console.groq.com/docs/rate-limits
pub const GROQ_MODELS: &[&str] = &[
    "llama-3.1-70b-versatile",
    "llama-3.1-8b-instant",
    "llama-3.2-11b-vision-preview",
    "llama-3.2-90b-vision-preview",
    "gemma2-9b-it",
    "gemma-7b-it",
    "mixtral-8x7b-32768",
];

/// Models available on Together.ai free tier.
pub const TOGETHER_MODELS: &[&str] = &[
    "meta-llama/Llama-3.2-11B-Vision-Instruct-Turbo",
    "meta-llama/Meta-Llama-3.1-70B-Instruct-Turbo",
    "Qwen/Qwen2.5-72B-Instruct-Turbo",
    "mistralai/Mixtral-8x7B-Instruct-v0.1",
];

/// Models available on Fireworks free tier.
pub const FIREWORKS_MODELS: &[&str] = &[
    "accounts/fireworks/models/llama-v3p1-70b-instruct",
    "accounts/fireworks/models/llama-v3p1-8b-instruct",
    "accounts/fireworks/models/mixtral-8x7b-instruct",
];

/// Returns ConnectionConfig entries for all free-tier providers.
///
/// Only providers whose API key env var is set are included.
/// Silently skips providers with missing keys — callers decide
/// whether to require at least one.
pub fn free_tier_connections() -> Vec<ConnectionConfig> {
    let mut conns = Vec::new();

    // Groq — OpenAI-compatible base URL
    if std::env::var("GROQ_API_KEY").is_ok() {
        conns.push(ConnectionConfig {
            id: ConnectionId("groq-default".into()),
            provider: ProviderKind::Custom {
                base_url: "https://api.groq.com/openai".into(),
            },
            auth: AuthKind::ApiKey {
                env_var: "GROQ_API_KEY".into(),
            },
            models: GROQ_MODELS.iter().map(|s| s.to_string()).collect(),
            max_concurrent: 30,
            weight: 1,
            tags: vec!["free-tier".into(), "groq".into()],
            capabilities: CapabilitySet::default(),
        });
    }

    // Together.ai — OpenAI-compatible
    if std::env::var("TOGETHER_API_KEY").is_ok() {
        conns.push(ConnectionConfig {
            id: ConnectionId("together-default".into()),
            provider: ProviderKind::Custom {
                base_url: "https://api.together.xyz/v1".into(),
            },
            auth: AuthKind::ApiKey {
                env_var: "TOGETHER_API_KEY".into(),
            },
            models: TOGETHER_MODELS.iter().map(|s| s.to_string()).collect(),
            max_concurrent: 20,
            weight: 1,
            tags: vec!["free-tier".into(), "together".into()],
            capabilities: CapabilitySet::default(),
        });
    }

    // Fireworks — OpenAI-compatible
    if std::env::var("FIREWORKS_API_KEY").is_ok() {
        conns.push(ConnectionConfig {
            id: ConnectionId("fireworks-default".into()),
            provider: ProviderKind::Custom {
                base_url: "https://api.fireworks.ai/inference/v1".into(),
            },
            auth: AuthKind::ApiKey {
                env_var: "FIREWORKS_API_KEY".into(),
            },
            models: FIREWORKS_MODELS.iter().map(|s| s.to_string()).collect(),
            max_concurrent: 25,
            weight: 1,
            tags: vec!["free-tier".into(), "fireworks".into()],
            capabilities: CapabilitySet::default(),
        });
    }

    conns
}

#[cfg(test)]
mod tests {
    use super::*;

    // Plausible wrong impl: free_tier_connections() panics when no keys are set
    #[test]
    fn no_keys_returns_empty_not_panic() {
        let result = std::panic::catch_unwind(free_tier_connections);
        assert!(
            result.is_ok(),
            "free_tier_connections must not panic with no keys set"
        );
    }

    // Plausible wrong impl: GROQ_API_KEY check fails but connection still included
    #[test]
    fn groq_not_included_without_key() {
        if std::env::var("GROQ_API_KEY").is_err() {
            let conns = free_tier_connections();
            assert!(
                !conns.iter().any(|c| c.id.0 == "groq-default"),
                "groq must not appear without GROQ_API_KEY"
            );
        }
    }

    // Plausible wrong impl: all providers have the same connection ID (collision)
    #[test]
    fn each_provider_has_unique_connection_id() {
        let conns = free_tier_connections();
        let ids: std::collections::HashSet<_> = conns.iter().map(|c| &c.id.0).collect();
        assert_eq!(
            ids.len(),
            conns.len(),
            "each provider must have a unique connection ID"
        );
    }

    // Plausible wrong impl: provider registered with wrong base_url
    #[test]
    fn groq_uses_openai_compatible_base_url() {
        std::env::set_var("GROQ_API_KEY", "test-key");
        let conns = free_tier_connections();
        let groq = conns.iter().find(|c| c.id.0 == "groq-default");
        assert!(groq.is_some(), "groq must be registered when key is set");
        if let Some(conn) = groq {
            if let ProviderKind::Custom { base_url } = &conn.provider {
                assert!(
                    base_url.contains("groq.com"),
                    "groq base_url must point to api.groq.com"
                );
                assert!(
                    base_url.contains("openai"),
                    "groq uses OpenAI-compatible endpoint"
                );
            } else {
                panic!("groq must use Custom provider kind");
            }
        }
        std::env::remove_var("GROQ_API_KEY");
    }
}
