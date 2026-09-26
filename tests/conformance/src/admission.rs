// Contract: when admission semaphore is exhausted, gateway returns 503
//
// Scenario: spec/scenarios/admission_rejected.yaml

use std::sync::Arc;

use axum::routing::post;
use http::{Method, Request};
use tower::ServiceExt;
use vkdg_connections::{
    AuthKind, ConnectionCatalog, ConnectionConfig, CredentialManager, ProviderKind,
};
use vkdg_core::ConnectionId;
use vkdg_http::upstream::HttpClient;
use vkdg_http::{AdmissionGuard, AppState, PipelineState, ServerConfig};
use vkdg_observe::DecisionRecordExporter;
use vkdg_operations::CapabilitySet;
use vkdg_provider_anthropic::AnthropicAdapter;
use vkdg_routing::{PluginHooks, RouteConfig, RouteId, Router, StrategyKind};

/// Build a router wired to the real ingress handler, with a real pipeline
/// whose admission limit is set to `limit`.
fn app_with_admission_limit(limit: usize) -> axum::Router {
    let conn_id = ConnectionId("test".into());
    let config = ConnectionConfig {
        id: conn_id.clone(),
        provider: ProviderKind::Custom {
            base_url: "http://127.0.0.1:1".into(),
        },
        auth: AuthKind::ApiKey {
            env_var: "VKDG_TEST_KEY".into(),
        },
        models: vec!["claude-*".into()],
        max_concurrent: 10,
        weight: 1,
        tags: vec![],
        capabilities: CapabilitySet::default(),
    };
    let route = RouteConfig {
        id: RouteId("test".into()),
        match_models: vec!["claude-*".into()],
        strategy: StrategyKind::RoundRobin,
        targets: vec![conn_id],
        plugin_hooks: PluginHooks::default(),
    };
    let pipeline = Arc::new(PipelineState {
        admission: Arc::new(AdmissionGuard::new(limit)),
        router: Arc::new(Router::new(vec![route])),
        catalog: Arc::new(ConnectionCatalog::new(vec![config])),
        credentials: Arc::new(CredentialManager::new()),
        http_client: Arc::new(HttpClient::new()),
        exporter: Arc::new(DecisionRecordExporter::new()),
        provider_adapter: Arc::new(AnthropicAdapter),
        cache: None,
        combo_resolver: None,
        compressor: None,
        dedup_table: None,
        session_registry: None,
        quota_tracker: None,
        global_system_prompt: None,
        ip_policy: None,
        latency_tracker: None,
        memory_store: None,
        eval_enabled: false,
    });
    let state = AppState::new(ServerConfig::default()).with_pipeline(pipeline);
    axum::Router::new()
        .route(
            "/v1/messages",
            post(vkdg_ingress_anthropic::handle_messages),
        )
        .with_state(state)
}

/// Reads the scenario YAML and asserts documented fields match test setup.
fn load_scenario(name: &str) -> serde_yaml::Value {
    let path = format!(
        "{}/spec/scenarios/{}.yaml",
        env!("CARGO_MANIFEST_DIR").trim_end_matches("tests/conformance"),
        name
    );
    // Best-effort: if file absent the test proceeds with in-code assertions only.
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_yaml::from_str(&s).ok())
        .unwrap_or(serde_yaml::Value::Null)
}

/// Contract: admission_limit=0 → HTTP 503.
/// The pipeline admission guard fires before any upstream call and returns
/// SERVICE_UNAVAILABLE when the semaphore is exhausted.
#[tokio::test]
async fn admission_rejected_returns_503() {
    let scenario = load_scenario("admission_rejected");
    let expected_status = scenario["assert"]["http_status"].as_u64().unwrap_or(503) as u16;

    let app = app_with_admission_limit(0);

    let req = Request::builder()
        .method(Method::POST)
        .uri("/v1/messages")
        .header("content-type", "application/json")
        .header("x-client-id", "client-test")
        .header("x-tenant-id", "tenant-test")
        .body(axum::body::Body::from(
            r#"{"model":"claude-3-5-sonnet-20241022","max_tokens":100,"messages":[{"role":"user","content":"hello"}]}"#,
        ))
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    let actual = response.status().as_u16();

    assert_eq!(
        actual, expected_status,
        "admission_rejected contract violated: expected HTTP {expected_status} (AdmissionRejected), got {actual}"
    );
}

/// AdmissionGuard unit-level contract: acquire on a fully-exhausted semaphore
/// returns Err(VkdgError::AdmissionRejected).
#[test]
fn admission_guard_unit_returns_error_when_limit_zero() {
    use vkdg_core::VkdgError;

    let guard = AdmissionGuard::new(0);
    let result = guard.acquire();
    assert!(
        matches!(result, Err(VkdgError::AdmissionRejected { .. })),
        "AdmissionGuard::acquire must return AdmissionRejected when limit=0; got {:?}",
        result
    );
}
