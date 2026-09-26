// Contract: when no route/connection is eligible, Router returns NoEligibleConnection.
//           fallback_chain skips excluded candidates and picks the next.
//
// These tests exercise vkdg-routing directly (no HTTP layer).
// Most PASS — routing is implemented in Phase A.

use vkdg_core::{ApiType, ClientId, ConnectionId, RequestEnvelope, RequestId, TenantId, VkdgError};
use vkdg_routing::{
    ConnectionWeight, EligibilityFilter, FallbackChainStrategy, RouteConfig, RouteId,
    RoundRobinStrategy, Router, Strategy, StrategyKind,
};

fn make_envelope(model: &str) -> RequestEnvelope {
    RequestEnvelope {
        request_id: RequestId::default(),
        client_id: ClientId("client-test".into()),
        tenant_id: TenantId("tenant-test".into()),
        session_key: None,
        api_type: ApiType::AnthropicMessages,
        model_requested: model.to_string(),
        deadline: None,
        mode_pack_override: None,
        compression_override: None,
        cache_bypass: false,
        include_think_tags: false,
    }
}

fn conn(id: &str) -> ConnectionId {
    ConnectionId(id.to_string())
}

/// Contract: Router with no routes returns NoEligibleConnection.
/// PASSES — Router::route is already implemented.
#[tokio::test]
async fn no_eligible_connection_returns_error_no_routes() {
    let router = Router::new(vec![]);
    let envelope = make_envelope("claude-3-5-sonnet-20241022");
    let filter = EligibilityFilter::default();

    let result = router.route(&envelope, &filter).await;
    assert!(
        matches!(result, Err(VkdgError::NoEligibleConnection)),
        "Expected NoEligibleConnection with empty route table; got {:?}",
        result
    );
}

/// Contract: Router with a matching route but all candidates excluded returns
/// NoEligibleConnection. PASSES.
#[tokio::test]
async fn no_eligible_connection_all_candidates_excluded() {
    let route = RouteConfig {
        id: RouteId("r-1".into()),
        match_models: vec!["claude-3-5-sonnet-20241022".to_string()],
        strategy: StrategyKind::FallbackChain,
        targets: vec![conn("conn-001")],
        plugin_hooks: Default::default(),
    };
    let router = Router::new(vec![route]);
    let envelope = make_envelope("claude-3-5-sonnet-20241022");

    let mut filter = EligibilityFilter::default();
    filter.excluded_connections.push(conn("conn-001"));

    let result = router.route(&envelope, &filter).await;
    assert!(
        matches!(result, Err(VkdgError::NoEligibleConnection)),
        "Expected NoEligibleConnection when only candidate is excluded; got {:?}",
        result
    );
}

/// Contract: FallbackChain skips excluded connections and selects the first
/// non-excluded candidate. PASSES.
#[tokio::test]
async fn fallback_chain_skips_excluded_selects_second() {
    let route = RouteConfig {
        id: RouteId("r-fallback".into()),
        match_models: vec!["claude-3-5-sonnet-20241022".to_string()],
        strategy: StrategyKind::FallbackChain,
        targets: vec![conn("conn-001"), conn("conn-002")],
        plugin_hooks: Default::default(),
    };
    let router = Router::new(vec![route]);
    let envelope = make_envelope("claude-3-5-sonnet-20241022");

    let mut filter = EligibilityFilter::default();
    filter.excluded_connections.push(conn("conn-001"));
    filter
        .reason_map
        .insert(conn("conn-001"), "circuit open".to_string());

    let result = router.route(&envelope, &filter).await.unwrap();
    assert_eq!(
        result.connection_id,
        conn("conn-002"),
        "FallbackChain must skip excluded conn-001 and select conn-002"
    );
    assert_eq!(result.excluded.len(), 1);
    assert_eq!(result.excluded[0].connection_id, conn("conn-001"));
}

/// Contract: model glob matching — prefix pattern `claude-3*` matches any
/// claude-3 model. PASSES.
#[tokio::test]
async fn router_glob_prefix_match() {
    let route = RouteConfig {
        id: RouteId("r-glob".into()),
        match_models: vec!["claude-3*".to_string()],
        strategy: StrategyKind::RoundRobin,
        targets: vec![conn("conn-001")],
        plugin_hooks: Default::default(),
    };
    let router = Router::new(vec![route]);
    let filter = EligibilityFilter::default();

    for model in &["claude-3-opus", "claude-3-5-sonnet-20241022", "claude-3-haiku"] {
        let envelope = make_envelope(model);
        let result = router.route(&envelope, &filter).await;
        assert!(
            result.is_ok(),
            "Glob 'claude-3*' should match '{model}'; got {:?}",
            result
        );
    }
}

/// Contract: model with no matching route returns NoEligibleConnection.
/// PASSES.
#[tokio::test]
async fn router_no_matching_route_for_model() {
    let route = RouteConfig {
        id: RouteId("r-exact".into()),
        match_models: vec!["gpt-4o".to_string()],
        strategy: StrategyKind::RoundRobin,
        targets: vec![conn("conn-001")],
        plugin_hooks: Default::default(),
    };
    let router = Router::new(vec![route]);
    let envelope = make_envelope("claude-3-5-sonnet-20241022");
    let filter = EligibilityFilter::default();

    let result = router.route(&envelope, &filter).await;
    assert!(
        matches!(result, Err(VkdgError::NoEligibleConnection)),
        "Model not matching any route must return NoEligibleConnection; got {:?}",
        result
    );
}
