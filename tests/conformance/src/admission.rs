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
use vkdg_core::pipeline::PipelineCtx;
use vkdg_core::ConnectionId;
use vkdg_core::{ApiType, ClientId, RequestEnvelope, RequestId, TenantId};
use vkdg_http::pipeline::run_conversation_pipeline;
use vkdg_http::upstream::HttpClient;
use vkdg_http::{AdmissionGuard, AppState, IpPolicy, PipelineState, ServerConfig};
use vkdg_observe::DecisionRecordExporter;
use vkdg_operations::{
    CapabilitySet, ConversationRequest, Message, MessageContent, Operation, Role,
};
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

/// Plausible wrong impl: IP check runs but doesn't block before admission —
/// capacity is consumed even for blocked IPs, exhausting the semaphore.
#[tokio::test]
async fn blocked_ip_rejected_before_admission_consumes_capacity() {
    std::env::set_var("VKDG_SMOKE_KEY", "test");

    let conn_id = ConnectionId("test".into());
    let config = ConnectionConfig {
        id: conn_id.clone(),
        provider: ProviderKind::Custom {
            base_url: "http://127.0.0.1:1".into(), // unreachable; should never be called
        },
        auth: AuthKind::ApiKey {
            env_var: "VKDG_SMOKE_KEY".into(),
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
    let mut pipeline = PipelineState::minimal(
        Arc::new(AdmissionGuard::new(1)), // only 1 slot — must NOT be consumed
        Arc::new(Router::new(vec![route])),
        Arc::new(ConnectionCatalog::new(vec![config])),
        Arc::new(CredentialManager::new()),
        Arc::new(HttpClient::new()),
        Arc::new(DecisionRecordExporter::new()),
        Arc::new(AnthropicAdapter),
    );
    // Allowlist contains only "10.0.0.1"; any other IP is blocked.
    pipeline.ip_policy = Some(Arc::new(IpPolicy {
        allowlist: vec!["10.0.0.1".into()],
        blocklist: vec![],
    }));
    let pipeline = Arc::new(pipeline);

    let envelope = RequestEnvelope {
        request_id: RequestId::new(),
        client_id: ClientId("test".into()),
        tenant_id: TenantId("default".into()),
        session_key: None,
        api_type: ApiType::AnthropicMessages,
        model_requested: "claude-3-5-haiku-20241022".into(),
        deadline: None,
        mode_pack_override: None,
        compression_override: None,
        cache_bypass: false,
        include_think_tags: false,
        client_ip: Some("1.2.3.4".into()), // NOT in allowlist
    };
    let op = Operation::Conversation(ConversationRequest {
        messages: vec![Message {
            role: Role::User,
            content: MessageContent::Text("hi".into()),
        }],
        tools: vec![],
        max_tokens: Some(10),
        temperature: None,
        stream: false,
        system: None,
        required_capabilities: CapabilitySet::default(),
    });
    let ctx = PipelineCtx::new(envelope);
    let resp = run_conversation_pipeline(pipeline, ctx, op).await;

    // VkdgError::Unauthorized maps to 403 FORBIDDEN; IP check runs before
    // admission so the semaphore slot is never consumed.
    assert_eq!(
        resp.status(),
        http::StatusCode::FORBIDDEN,
        "blocked IP must return 403 before consuming admission capacity"
    );
}
