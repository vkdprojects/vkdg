//! Request routing for VKDG.
//!
//! The Router resolves a `RequestEnvelope` to a `RouteResult` by applying
//! an eligibility filter and a routing strategy. Strategies are pluggable
//! via the Strategy trait.

pub mod router;
pub mod scored;
pub mod scorer;
pub mod strategy;
pub mod types;

pub use router::Router;
pub use scored::ScoredStrategy;
pub use scorer::{rank_candidates, CandidateSignals, ScoringWeights};
pub use strategy::{
    FallbackChainStrategy, LowestLatencyStrategy, PowerOfTwoChoicesStrategy, RoundRobinStrategy,
    Strategy,
};
pub use types::{
    ChainStep, ConnectionWeight, EligibilityFilter, InjectMode, PluginHooks, RouteConfig, RouteId,
    RouteResult, RoutingHints, StrategyKind,
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
            strategy: StrategyKind::Scored {
                mode_pack: "balanced".into(),
            },
            targets: vec![conn.clone()],
            plugin_hooks: PluginHooks::default(),
        };
        let router = Router::new(vec![route]);
        let envelope = test_envelope("claude-3-5-haiku-20241022");
        let result = router
            .route(
                &envelope,
                &EligibilityFilter::default(),
                &RoutingHints::default(),
            )
            .await;
        assert!(
            result.is_ok(),
            "scored strategy must select from available candidates"
        );
        assert_eq!(result.unwrap().connection_id, conn);
    }

    #[tokio::test]
    async fn scored_strategy_uses_configured_mode_pack() {
        let route = RouteConfig {
            id: RouteId("r".into()),
            match_models: vec!["gpt-*".into()],
            strategy: StrategyKind::Scored {
                mode_pack: "ship-fast".into(),
            },
            targets: vec![
                vkdg_core::ConnectionId("fast".into()),
                vkdg_core::ConnectionId("cheap".into()),
            ],
            plugin_hooks: PluginHooks::default(),
        };
        let router = Router::new(vec![route]);
        let envelope = test_envelope("gpt-4o");
        let result = router
            .route(
                &envelope,
                &EligibilityFilter::default(),
                &RoutingHints::default(),
            )
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn scored_strategy_returns_no_eligible_when_all_excluded() {
        let conn = vkdg_core::ConnectionId("conn-a".into());
        let route = RouteConfig {
            id: RouteId("r".into()),
            match_models: vec!["claude-*".into()],
            strategy: StrategyKind::Scored {
                mode_pack: "balanced".into(),
            },
            targets: vec![conn.clone()],
            plugin_hooks: PluginHooks::default(),
        };
        let router = Router::new(vec![route]);
        let envelope = test_envelope("claude-3-opus-20240229");
        let mut filter = EligibilityFilter::default();
        filter.excluded_connections.push(conn);
        let result = router
            .route(&envelope, &filter, &RoutingHints::default())
            .await;
        assert!(matches!(
            result,
            Err(vkdg_core::VkdgError::NoEligibleConnection)
        ));
    }

    #[tokio::test]
    async fn scored_strategy_uses_quota_hints() {
        let mut hints = RoutingHints::default();
        hints
            .quota_headroom
            .insert(vkdg_core::ConnectionId("high-quota".into()), 0.9);
        hints
            .quota_headroom
            .insert(vkdg_core::ConnectionId("low-quota".into()), 0.1);
        hints.mode_pack = Some("quality-first".into());

        let route = RouteConfig {
            id: RouteId("r".into()),
            match_models: vec!["gpt-*".into()],
            strategy: StrategyKind::Scored {
                mode_pack: "balanced".into(),
            },
            targets: vec![
                vkdg_core::ConnectionId("high-quota".into()),
                vkdg_core::ConnectionId("low-quota".into()),
            ],
            plugin_hooks: PluginHooks::default(),
        };
        let router = Router::new(vec![route]);
        let envelope = test_envelope("gpt-4o");
        let result = router
            .route(&envelope, &EligibilityFilter::default(), &hints)
            .await;
        assert!(result.is_ok());
        assert_eq!(
            result.unwrap().connection_id,
            vkdg_core::ConnectionId("high-quota".into()),
            "high-quota connection must be preferred when hints are fed to scorer"
        );
    }

    // Plausible wrong impl: Fusion strategy returns NoEligibleConnection when targets exist.
    #[tokio::test]
    async fn fusion_strategy_returns_route_result_with_all_targets() {
        let route = RouteConfig {
            id: RouteId("fusion-route".into()),
            match_models: vec!["claude-*".into()],
            strategy: StrategyKind::Fusion {
                max_candidates: Some(2),
            },
            targets: vec![
                vkdg_core::ConnectionId("conn-a".into()),
                vkdg_core::ConnectionId("conn-b".into()),
                vkdg_core::ConnectionId("conn-c".into()),
            ],
            plugin_hooks: PluginHooks::default(),
        };
        let router = Router::new(vec![route]);
        let envelope = test_envelope("claude-3-5-haiku-20241022");
        let result = router
            .route(
                &envelope,
                &EligibilityFilter::default(),
                &RoutingHints::default(),
            )
            .await;
        assert!(result.is_ok());
        let r = result.unwrap();
        // max_candidates=2 → exactly 2 targets in fusion_targets
        assert_eq!(
            r.fusion_targets.len(),
            2,
            "Fusion with max_candidates=2 must return 2 targets"
        );
        assert_eq!(
            r.connection_id,
            vkdg_core::ConnectionId("conn-a".into()),
            "primary connection must be the first eligible target"
        );
    }

    // Plausible wrong impl: Fusion with no excluded connections still returns fewer than all targets.
    #[tokio::test]
    async fn fusion_strategy_no_max_returns_all_targets() {
        let route = RouteConfig {
            id: RouteId("fusion-all".into()),
            match_models: vec!["claude-*".into()],
            strategy: StrategyKind::Fusion {
                max_candidates: None,
            },
            targets: vec![
                vkdg_core::ConnectionId("a".into()),
                vkdg_core::ConnectionId("b".into()),
                vkdg_core::ConnectionId("c".into()),
            ],
            plugin_hooks: PluginHooks::default(),
        };
        let router = Router::new(vec![route]);
        let envelope = test_envelope("claude-3-opus-20240229");
        let result = router
            .route(
                &envelope,
                &EligibilityFilter::default(),
                &RoutingHints::default(),
            )
            .await;
        assert!(result.is_ok());
        let r = result.unwrap();
        assert_eq!(
            r.fusion_targets.len(),
            3,
            "Fusion with no max must return all 3 targets"
        );
    }

    // Plausible wrong impl: Fusion includes excluded connections in fusion_targets.
    #[tokio::test]
    async fn fusion_strategy_excludes_filtered_connections() {
        let route = RouteConfig {
            id: RouteId("fusion-excl".into()),
            match_models: vec!["claude-*".into()],
            strategy: StrategyKind::Fusion {
                max_candidates: None,
            },
            targets: vec![
                vkdg_core::ConnectionId("a".into()),
                vkdg_core::ConnectionId("b".into()),
                vkdg_core::ConnectionId("c".into()),
            ],
            plugin_hooks: PluginHooks::default(),
        };
        let router = Router::new(vec![route]);
        let envelope = test_envelope("claude-3-5-sonnet-20241022");
        // Exclude "a" — only "b" and "c" should be in fusion_targets.
        let mut filter = EligibilityFilter::default();
        filter
            .excluded_connections
            .push(vkdg_core::ConnectionId("a".into()));
        let result = router
            .route(&envelope, &filter, &RoutingHints::default())
            .await;
        assert!(result.is_ok());
        let r = result.unwrap();
        assert_eq!(r.fusion_targets.len(), 2);
        assert!(
            !r.fusion_targets
                .contains(&vkdg_core::ConnectionId("a".into())),
            "excluded connection must not appear in fusion_targets"
        );
    }

    // Plausible wrong impl: Fusion returns Ok when all targets are excluded.
    #[tokio::test]
    async fn fusion_strategy_all_excluded_returns_no_eligible() {
        let route = RouteConfig {
            id: RouteId("fusion-empty".into()),
            match_models: vec!["claude-*".into()],
            strategy: StrategyKind::Fusion {
                max_candidates: None,
            },
            targets: vec![vkdg_core::ConnectionId("only".into())],
            plugin_hooks: PluginHooks::default(),
        };
        let router = Router::new(vec![route]);
        let envelope = test_envelope("claude-3-haiku-20240307");
        let mut filter = EligibilityFilter::default();
        filter
            .excluded_connections
            .push(vkdg_core::ConnectionId("only".into()));
        let result = router
            .route(&envelope, &filter, &RoutingHints::default())
            .await;
        assert!(
            matches!(result, Err(vkdg_core::VkdgError::NoEligibleConnection)),
            "Fusion with all targets excluded must return NoEligibleConnection"
        );
    }

    fn rr_route(id: &str, model: &str, target: &str) -> RouteConfig {
        RouteConfig {
            id: RouteId(id.into()),
            match_models: vec![model.into()],
            strategy: StrategyKind::RoundRobin,
            targets: vec![vkdg_core::ConnectionId(target.into())],
            plugin_hooks: PluginHooks::default(),
        }
    }

    // Found live: editing config.yaml bumped the admin revision but the data
    // plane kept routing with the startup route table.
    #[tokio::test]
    async fn replaced_routes_take_effect_on_the_next_request() {
        let router = Router::new(vec![rr_route("r1", "foo-*", "c1")]);
        let (f, h) = (EligibilityFilter::default(), RoutingHints::default());
        assert!(router.route(&test_envelope("foo-1"), &f, &h).await.is_ok());

        router.replace_routes(vec![rr_route("r2", "bar-*", "c2")]);
        assert!(matches!(
            router.route(&test_envelope("foo-1"), &f, &h).await,
            Err(vkdg_core::VkdgError::NoRouteMatched)
        ));
        let r = router.route(&test_envelope("bar-1"), &f, &h).await.unwrap();
        assert_eq!(r.route_id, RouteId("r2".into()));
    }

    // Combos are edited at runtime and config routes come from the file: a
    // reload must not drop combos, and a combo edit must not drop the file's routes.
    #[tokio::test]
    async fn combo_routes_match_first_and_survive_config_reloads() {
        let router = Router::new(vec![rr_route("cfg", "shared", "c-cfg")]);
        let (f, h) = (EligibilityFilter::default(), RoutingHints::default());
        router.replace_combo_routes(vec![rr_route("combo", "shared", "c-combo")]);
        let r = router
            .route(&test_envelope("shared"), &f, &h)
            .await
            .unwrap();
        assert_eq!(r.route_id, RouteId("combo".into()), "combo wins");

        router.replace_routes(vec![
            rr_route("cfg", "shared", "c-cfg"),
            rr_route("cfg2", "other", "c2"),
        ]);
        let r = router
            .route(&test_envelope("shared"), &f, &h)
            .await
            .unwrap();
        assert_eq!(r.route_id, RouteId("combo".into()), "reload kept the combo");

        router.replace_combo_routes(vec![]);
        let r = router
            .route(&test_envelope("shared"), &f, &h)
            .await
            .unwrap();
        assert_eq!(
            r.route_id,
            RouteId("cfg".into()),
            "combo edit kept config routes"
        );
        assert!(router.route(&test_envelope("other"), &f, &h).await.is_ok());
    }

    fn two_target_route(strategy: StrategyKind) -> RouteConfig {
        RouteConfig {
            id: RouteId("r".into()),
            match_models: vec!["*".into()],
            strategy,
            targets: vec![
                vkdg_core::ConnectionId("slow".into()),
                vkdg_core::ConnectionId("fast".into()),
            ],
            plugin_hooks: PluginHooks::default(),
        }
    }

    fn latency_hints() -> RoutingHints {
        let mut h = RoutingHints::default();
        h.latency_p50_ms
            .insert(vkdg_core::ConnectionId("slow".into()), 900);
        h.latency_p50_ms
            .insert(vkdg_core::ConnectionId("fast".into()), 80);
        h
    }

    // lowest_latency parsed from config but silently routed round-robin.
    #[tokio::test]
    async fn lowest_latency_prefers_the_faster_target() {
        let router = Router::new(vec![two_target_route(StrategyKind::LowestLatency)]);
        for _ in 0..6 {
            let r = router
                .route(
                    &test_envelope("m"),
                    &EligibilityFilter::default(),
                    &latency_hints(),
                )
                .await
                .unwrap();
            assert_eq!(r.connection_id.0, "fast");
        }
    }

    // Prod bug: with two eligible targets P2C always sampled both and took the
    // lower latency, so it behaved as lowest_latency. The faster account took
    // every request, the other got none for hours and was never re-measured.
    // Refutes "P2C picks the lower latency of its two samples".
    #[tokio::test]
    async fn power_of_two_choices_does_not_starve_the_slower_target() {
        let router = Router::new(vec![two_target_route(StrategyKind::PowerOfTwoChoices)]);
        let mut slow = 0;
        let mut fast = 0;
        for _ in 0..200 {
            let r = router
                .route(
                    &test_envelope("m"),
                    &EligibilityFilter::default(),
                    &latency_hints(),
                )
                .await
                .unwrap();
            if r.connection_id.0 == "slow" {
                slow += 1;
            } else {
                fast += 1;
            }
        }
        assert!(
            slow >= 60 && fast >= 60,
            "equal load must spread: slow={slow} fast={fast}"
        );
    }

    // Refutes "P2C ignores in-flight load": the loaded target must lose even
    // when it is the faster one.
    #[tokio::test]
    async fn power_of_two_choices_picks_the_less_loaded_target() {
        let router = Router::new(vec![two_target_route(StrategyKind::PowerOfTwoChoices)]);
        let mut hints = latency_hints();
        hints
            .in_flight
            .insert(vkdg_core::ConnectionId("fast".into()), 3);
        hints
            .in_flight
            .insert(vkdg_core::ConnectionId("slow".into()), 1);
        for _ in 0..20 {
            let r = router
                .route(&test_envelope("m"), &EligibilityFilter::default(), &hints)
                .await
                .unwrap();
            assert_eq!(r.connection_id.0, "slow");
        }
    }

    // No latency data yet (cold start): still pick an eligible target.
    #[tokio::test]
    async fn latency_strategy_without_data_still_routes() {
        let router = Router::new(vec![two_target_route(StrategyKind::LowestLatency)]);
        let r = router
            .route(
                &test_envelope("m"),
                &EligibilityFilter::default(),
                &RoutingHints::default(),
            )
            .await;
        assert!(r.is_ok());
    }

    // A route that matched but has no usable target must not look like "no
    // route": the pipeline falls back to any connection only on the latter, and
    // treating both alike let requests escape the matched route's targets.
    #[tokio::test]
    async fn matched_route_with_no_usable_target_is_not_a_miss() {
        let router = Router::new(vec![rr_route("r1", "foo-*", "c1")]);
        let h = RoutingHints::default();
        let all_out = EligibilityFilter {
            excluded_connections: vec![vkdg_core::ConnectionId("c1".into())],
            reason_map: Default::default(),
        };
        assert!(matches!(
            router.route(&test_envelope("foo-1"), &all_out, &h).await,
            Err(vkdg_core::VkdgError::NoEligibleConnection)
        ));
        assert!(matches!(
            router
                .route(&test_envelope("bar-1"), &EligibilityFilter::default(), &h)
                .await,
            Err(vkdg_core::VkdgError::NoRouteMatched)
        ));
    }
}
