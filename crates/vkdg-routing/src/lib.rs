//! Request routing for VKDG.
//!
//! The Router resolves a RequestEnvelope to a RouteResult by applying
//! an eligibility filter and a routing strategy. Strategies are pluggable
//! via the Strategy trait.

pub mod router;
pub mod scored;
pub mod scorer;
pub mod strategy;
pub mod types;

pub use router::Router;
pub use scored::ScoredStrategy;
pub use scorer::{CandidateSignals, ScoringWeights, rank_candidates};
pub use strategy::{FallbackChainStrategy, RoundRobinStrategy, Strategy};
pub use types::{
    ConnectionWeight, EligibilityFilter, PluginHooks, RouteConfig,
    RouteId, RouteResult, RoutingHints, StrategyKind,
};

#[cfg(test)]
mod tests {
    use super::*;
    use vkdg_core::{ApiType, ClientId, RequestId, TenantId};

    fn test_envelope(model: &str) -> vkdg_core::RequestEnvelope {
        vkdg_core::RequestEnvelope {
            request_id: RequestId::new(),
            client_id: ClientId("test-client".into()),
            tenant_id: TenantId("test-tenant".into()),
            session_key: None,
            api_type: ApiType::AnthropicMessages,
            model_requested: model.to_string(),
            deadline: None,
            mode_pack_override: None,
            compression_override: None,
            cache_bypass: false,
            include_think_tags: false,
            client_ip: None,
        }
    }

    #[tokio::test]
    async fn scored_strategy_selects_single_candidate() {
        let conn = vkdg_core::ConnectionId("conn-a".into());
        let route = RouteConfig {
            id: RouteId("r".into()),
            match_models: vec!["claude-*".into()],
            strategy: StrategyKind::Scored { mode_pack: "balanced".into() },
            targets: vec![conn.clone()],
            plugin_hooks: PluginHooks::default(),
        };
        let router = Router::new(vec![route]);
        let envelope = test_envelope("claude-3-5-haiku-20241022");
        let result = router.route(&envelope, &EligibilityFilter::default(), &RoutingHints::default()).await;
        assert!(result.is_ok(), "scored strategy must select from available candidates");
        assert_eq!(result.unwrap().connection_id, conn);
    }

    #[tokio::test]
    async fn scored_strategy_uses_configured_mode_pack() {
        let route = RouteConfig {
            id: RouteId("r".into()),
            match_models: vec!["gpt-*".into()],
            strategy: StrategyKind::Scored { mode_pack: "ship-fast".into() },
            targets: vec![vkdg_core::ConnectionId("fast".into()), vkdg_core::ConnectionId("cheap".into())],
            plugin_hooks: PluginHooks::default(),
        };
        let router = Router::new(vec![route]);
        let envelope = test_envelope("gpt-4o");
        let result = router.route(&envelope, &EligibilityFilter::default(), &RoutingHints::default()).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn scored_strategy_returns_no_eligible_when_all_excluded() {
        let conn = vkdg_core::ConnectionId("conn-a".into());
        let route = RouteConfig {
            id: RouteId("r".into()),
            match_models: vec!["claude-*".into()],
            strategy: StrategyKind::Scored { mode_pack: "balanced".into() },
            targets: vec![conn.clone()],
            plugin_hooks: PluginHooks::default(),
        };
        let router = Router::new(vec![route]);
        let envelope = test_envelope("claude-3-opus-20240229");
        let mut filter = EligibilityFilter::default();
        filter.excluded_connections.push(conn);
        let result = router.route(&envelope, &filter, &RoutingHints::default()).await;
        assert!(matches!(result, Err(vkdg_core::VkdgError::NoEligibleConnection)));
    }

    #[tokio::test]
    async fn scored_strategy_uses_quota_hints() {
        let mut hints = RoutingHints::default();
        hints.quota_headroom.insert(vkdg_core::ConnectionId("high-quota".into()), 0.9);
        hints.quota_headroom.insert(vkdg_core::ConnectionId("low-quota".into()), 0.1);
        hints.mode_pack = Some("quality-first".into());

        let route = RouteConfig {
            id: RouteId("r".into()),
            match_models: vec!["gpt-*".into()],
            strategy: StrategyKind::Scored { mode_pack: "balanced".into() },
            targets: vec![
                vkdg_core::ConnectionId("high-quota".into()),
                vkdg_core::ConnectionId("low-quota".into()),
            ],
            plugin_hooks: PluginHooks::default(),
        };
        let router = Router::new(vec![route]);
        let envelope = test_envelope("gpt-4o");
        let result = router.route(&envelope, &EligibilityFilter::default(), &hints).await;
        assert!(result.is_ok());
        assert_eq!(
            result.unwrap().connection_id,
            vkdg_core::ConnectionId("high-quota".into()),
            "high-quota connection must be preferred when hints are fed to scorer"
        );
    }
}
