// Smoke test: end-to-end pipeline exercise against a fake upstream.
// Tests: decode Anthropic -> pipeline -> upstream HTTP -> JSON response passthrough.
// Does not start the gateway HTTP server — calls run_conversation_pipeline directly.
//
// Plausible wrong impl defeated: pipeline never reaches upstream (returns error first),
// or upstream is called but the response is dropped or corrupted.

use std::sync::Arc;
use vkdg_connections::{
    AuthKind, Connection, ConnectionCatalog, ConnectionConfig, CredentialManager, ProviderKind,
    SessionRegistry,
};
use vkdg_core::pipeline::PipelineCtx;
use vkdg_core::{
    ApiType, ClientId, ConnectionId, RequestEnvelope, RequestId, SessionKey, TenantId,
};
use vkdg_http::pipeline::run_conversation_pipeline;
use vkdg_http::upstream::HttpClient;
use vkdg_http::{AdmissionGuard, PipelineState};
use vkdg_observe::DecisionRecordExporter;
use vkdg_operations::{
    CapabilitySet, ConversationRequest, Message, MessageContent, Operation, Role,
};
use vkdg_routing::{PluginHooks, RouteConfig, RouteId, Router, StrategyKind};

use crate::fake_upstream::{FakeUpstream, FakeUpstreamBehavior};
use vkdg_provider_anthropic::AnthropicAdapter;
use vkdg_provider_openai::OpenAIAdapter;
use vkdg_provider_sdk::ProviderRegistry;

fn test_registry() -> Arc<ProviderRegistry> {
    let mut r = ProviderRegistry::empty();
    r.register(Arc::new(AnthropicAdapter));
    r.register(Arc::new(OpenAIAdapter));
    Arc::new(r)
}

/// The connection kind whose adapter speaks `adapter_id`'s dialect: Anthropic
/// Messages for `anthropic`, `OpenAI` Chat for the rest. Tests state their real
/// upstream dialect through the adapter they pass.
fn provider_for(adapter_id: &str, base_url: String) -> ProviderKind {
    if adapter_id == "anthropic" {
        ProviderKind::AnthropicCompat { base_url }
    } else {
        ProviderKind::Custom { base_url }
    }
}

fn make_pipeline(base_url: String, max_concurrent: usize) -> Arc<PipelineState> {
    let conn_id = ConnectionId("fake".into());
    let config = ConnectionConfig {
        id: conn_id.clone(),
        // The fixtures of `make_pipeline` answer in Anthropic Messages.
        provider: ProviderKind::AnthropicCompat { base_url },
        auth: AuthKind::ApiKey {
            env_var: "VKDG_SMOKE_KEY".into(),
        },
        models: vec!["claude-*".into()],
        max_concurrent: u32::try_from(max_concurrent).unwrap_or(u32::MAX),
        weight: 1,
        tags: vec![],
        endpoint: None,
        capabilities: CapabilitySet::default(),
    };
    let route = RouteConfig {
        id: RouteId("smoke".into()),
        match_models: vec!["claude-*".into()],
        strategy: StrategyKind::RoundRobin,
        targets: vec![conn_id],
        plugin_hooks: PluginHooks::default(),
    };
    Arc::new(PipelineState::minimal(
        Arc::new(AdmissionGuard::new(max_concurrent)),
        Arc::new(Router::new(vec![route])),
        Arc::new(ConnectionCatalog::new(vec![config])),
        Arc::new(CredentialManager::new()),
        Arc::new(HttpClient::new()),
        Arc::new(DecisionRecordExporter::new()),
        test_registry(),
    ))
}

fn make_ctx(model: &str) -> (PipelineCtx, Operation) {
    let envelope = RequestEnvelope {
        request_id: RequestId::new(),
        client_id: ClientId("smoke".into()),
        tenant_id: TenantId("default".into()),
        session_key: None,
        api_type: ApiType::AnthropicMessages,
        model_requested: model.into(),
        deadline: None,
        mode_pack_override: None,
        compression_override: None,
        cache_bypass: false,
        include_think_tags: false,
        client_ip: None,
    };
    let op = Operation::Conversation(ConversationRequest {
        model: "test-model".into(),
        messages: vec![Message {
            role: Role::User,
            content: MessageContent::Text("ping".into()),
        }],
        tools: vec![],
        max_tokens: Some(10),
        temperature: None,
        stream: false,
        system: None,
        required_capabilities: CapabilitySet::default(),
        thinking: None,
        ..Default::default()
    });
    (PipelineCtx::new(envelope), op)
}

/// Same as `make_ctx` but with stream: true for testing SSE passthrough.
fn make_ctx_streaming(model: &str, api_type: ApiType) -> (PipelineCtx, Operation) {
    let envelope = RequestEnvelope {
        request_id: RequestId::new(),
        client_id: ClientId("smoke".into()),
        tenant_id: TenantId("default".into()),
        session_key: None,
        api_type,
        model_requested: model.into(),
        deadline: None,
        mode_pack_override: None,
        compression_override: None,
        cache_bypass: false,
        include_think_tags: false,
        client_ip: None,
    };
    let op = Operation::Conversation(ConversationRequest {
        model: "test-model".into(),
        messages: vec![Message {
            role: Role::User,
            content: MessageContent::Text("ping".into()),
        }],
        tools: vec![],
        max_tokens: Some(10),
        temperature: None,
        stream: true,
        system: None,
        required_capabilities: CapabilitySet::default(),
        thinking: None,
        ..Default::default()
    });
    (PipelineCtx::new(envelope), op)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Smoke: pipeline routes to fake upstream and returns 200 with Anthropic JSON body.
/// Wrong impl defeated: pipeline returns 502/501 before reaching upstream,
/// or upstream is called but response is dropped/replaced.
#[tokio::test]
async fn smoke_pipeline_non_streaming_ok() {
    // Env var needed for CredentialManager ApiKey lookup.
    std::env::set_var("VKDG_SMOKE_KEY", "test-token");

    let fake = FakeUpstream::spawn(FakeUpstreamBehavior::AnthropicOk {
        content: "pong".into(),
    })
    .await;

    let pipeline = make_pipeline(fake.base_url.clone(), 10);
    let (ctx, op) = make_ctx("claude-3-5-haiku-20241022");

    let response = run_conversation_pipeline(pipeline, ctx, op).await;
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(status, 200, "expected 200, got {status}. body: {json}");
    assert_eq!(fake.call_count(), 1, "upstream must be called exactly once");
    // Body is passed through from fake upstream verbatim.
    assert_eq!(
        json["content"][0]["text"], "pong",
        "response content must match fake upstream reply"
    );
}

/// Smoke: pipeline routes streaming request and returns text/event-stream.
/// Wrong impl defeated: streaming response gets double-encoded or content-type is wrong.
#[tokio::test]
async fn smoke_pipeline_streaming_ok() {
    std::env::set_var("VKDG_SMOKE_KEY", "test-token");

    let fake = FakeUpstream::spawn(FakeUpstreamBehavior::AnthropicStreamOk {
        content: "stream-pong".into(),
    })
    .await;

    let pipeline = make_pipeline(fake.base_url.clone(), 10);
    let (ctx, op) = make_ctx_streaming("claude-3-5-haiku-20241022", ApiType::AnthropicMessages);

    let response = run_conversation_pipeline(pipeline, ctx, op).await;
    let status = response.status();
    let ct = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    assert_eq!(status, 200, "expected 200 for streaming");
    assert!(
        ct.contains("text/event-stream"),
        "content-type must be text/event-stream, got: {ct}"
    );

    // Drain the body and verify it contains SSE frames (not double-encoded).
    let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap();
    let body_str = String::from_utf8_lossy(&body);
    assert!(
        body_str.contains("data: "),
        "body must contain raw SSE data frames, got: {body_str}"
    );
    assert!(
        body_str.contains("stream-pong"),
        "body must contain the streamed content, got: {body_str}"
    );
    assert!(
        body_str.contains("[DONE]"),
        "body must contain [DONE] sentinel, got: {body_str}"
    );
    assert_eq!(fake.call_count(), 1);
}

/// Admission exhausted before upstream is ever called.
/// Wrong impl defeated: request reaches routing/upstream even when semaphore is 0.
#[tokio::test]
async fn smoke_admission_rejected_upstream_never_called() {
    std::env::set_var("VKDG_SMOKE_KEY", "test-token");

    let fake = FakeUpstream::spawn(FakeUpstreamBehavior::AnthropicOk {
        content: "should-not-appear".into(),
    })
    .await;

    // max_concurrent = 0 → admission always rejects
    let pipeline = make_pipeline(fake.base_url.clone(), 0);
    let (ctx, op) = make_ctx("claude-3-5-haiku-20241022");

    let response = run_conversation_pipeline(pipeline, ctx, op).await;
    assert_eq!(response.status(), 503, "admission=0 must yield 503");
    assert_eq!(
        fake.call_count(),
        0,
        "upstream must NOT be called when admission rejects"
    );
}

// Defeito plausível derrotado: ConnectionCatalog.eligible() retorna a mesma
// conexão para dois requests simultâneos mas active_requests não sobe
// (race condition no fetch_add), causando exceder max_concurrent.
//
// Verifica: 2 tasks simultâneas, cada uma reserva a mesma conexão via acquire(),
// active_requests chega a 2 durante a execução, e cai a 0 quando ambos terminam.
#[tokio::test]
async fn concurrent_streams_same_connection() {
    let config = ConnectionConfig {
        id: ConnectionId("concurrent".into()),
        provider: ProviderKind::Custom {
            base_url: "http://unused".into(),
        },
        auth: AuthKind::ApiKey {
            env_var: "VKDG_SMOKE_KEY".into(),
        },
        models: vec!["claude-*".into()],
        max_concurrent: 2,
        weight: 1,
        tags: vec![],
        endpoint: None,
        capabilities: CapabilitySet::default(),
    };
    let conn = Arc::new(Connection::new(config));

    let conn1 = Arc::clone(&conn);
    let conn2 = Arc::clone(&conn);

    let (g1, g2) = tokio::join!(
        async move { conn1.acquire() },
        async move { conn2.acquire() },
    );

    let g1 = g1.expect("first acquire must succeed");
    let g2 = g2.expect("second acquire must succeed");

    assert_eq!(
        conn.active_requests(),
        2,
        "both guards active: counter must be 2"
    );

    // Third acquire must fail — capacity exhausted.
    assert!(
        conn.acquire().is_none(),
        "third acquire must fail when max_concurrent=2 and 2 guards held"
    );

    drop(g1);
    drop(g2);

    assert_eq!(
        conn.active_requests(),
        0,
        "all guards dropped: counter must return to 0"
    );
    assert!(conn.acquire().is_some(), "capacity restored after drop");
}

// Defeito plausível derrotado: Drop de ConnectionGuard não decrementa counter
// (esqueceu AcqRel, usou Relaxed, ou não implementou Drop).
// Sem isso, active_requests só cresce e conexão fica inacessível.
#[tokio::test]
async fn connection_guard_releases_on_drop() {
    let config = ConnectionConfig {
        id: ConnectionId("drop-test".into()),
        provider: ProviderKind::Custom {
            base_url: "http://unused".into(),
        },
        auth: AuthKind::ApiKey {
            env_var: "VKDG_SMOKE_KEY".into(),
        },
        models: vec!["*".into()],
        max_concurrent: 1,
        weight: 1,
        tags: vec![],
        endpoint: None,
        capabilities: CapabilitySet::default(),
    };
    let conn = Connection::new(config);

    let guard = conn
        .acquire()
        .expect("acquire must succeed on fresh connection");
    assert_eq!(
        conn.active_requests(),
        1,
        "counter must be 1 while guard is held"
    );

    drop(guard);
    assert_eq!(
        conn.active_requests(),
        0,
        "counter must be 0 after guard drop"
    );

    // Capacity is fully restored.
    assert!(
        conn.acquire().is_some(),
        "must be acquirable again after drop"
    );
}

// Plausible wrong impl defeated: ConnectionGuard inside the pipeline
// is not released if the Future is dropped before completing
// (e.g. client closes the connection). With RAII this must work automatically.
#[tokio::test]
async fn pipeline_cancellation_releases_on_drop() {
    let config = ConnectionConfig {
        id: ConnectionId("cancel-test".into()),
        provider: ProviderKind::Custom {
            base_url: "http://unused".into(),
        },
        auth: AuthKind::ApiKey {
            env_var: "VKDG_SMOKE_KEY".into(),
        },
        models: vec!["*".into()],
        max_concurrent: 1,
        weight: 1,
        tags: vec![],
        endpoint: None,
        capabilities: CapabilitySet::default(),
    };
    let conn = Connection::new(config);

    // Simulate a pipeline mid-flight: guard acquired, then future is cancelled (dropped).
    {
        let _guard = conn.acquire().expect("acquire must succeed");
        assert_eq!(conn.active_requests(), 1);
        // _guard dropped here — simulates tokio cancelling the future mid-pipeline
    }

    assert_eq!(
        conn.active_requests(),
        0,
        "cancellation (drop) must release connection guard"
    );
    assert!(
        conn.acquire().is_some(),
        "connection must be available after cancellation"
    );
}

// ── OpenAI path smoke tests ───────────────────────────────────────────────────

/// Helper: build a pipeline using a custom adapter and a single connection.
fn make_pipeline_with<A: vkdg_http::provider::ProviderAdapter + 'static>(
    base_url: String,
    adapter: &A,
    model_pattern: &str,
) -> Arc<PipelineState> {
    let conn_id = ConnectionId("fake".into());
    let config = ConnectionConfig {
        id: conn_id.clone(),
        provider: provider_for(adapter.id(), base_url),
        auth: AuthKind::ApiKey {
            env_var: "VKDG_SMOKE_KEY".into(),
        },
        models: vec![model_pattern.into()],
        max_concurrent: 10,
        weight: 1,
        tags: vec![],
        endpoint: None,
        capabilities: CapabilitySet::default(),
    };
    let route = RouteConfig {
        id: RouteId("smoke".into()),
        match_models: vec![model_pattern.into()],
        strategy: StrategyKind::RoundRobin,
        targets: vec![conn_id],
        plugin_hooks: PluginHooks::default(),
    };
    Arc::new(PipelineState::minimal(
        Arc::new(AdmissionGuard::new(10)),
        Arc::new(Router::new(vec![route])),
        Arc::new(ConnectionCatalog::new(vec![config])),
        Arc::new(CredentialManager::new()),
        Arc::new(HttpClient::new()),
        Arc::new(DecisionRecordExporter::new()),
        test_registry(),
    ))
}

/// Helper: build a pipeline with two connections using the same adapter.
/// First connection is at `first_url`, second at `second_url`.
/// The router uses `RoundRobin`; exclusion makes it fall through to the second on retry.
fn make_pipeline_two_connections<A: vkdg_http::provider::ProviderAdapter + 'static>(
    first_url: String,
    second_url: String,
    adapter: &A,
    model_pattern: &str,
) -> Arc<PipelineState> {
    let id1 = ConnectionId("conn-1".into());
    let id2 = ConnectionId("conn-2".into());
    let cfg1 = ConnectionConfig {
        id: id1.clone(),
        provider: provider_for(adapter.id(), first_url),
        auth: AuthKind::ApiKey {
            env_var: "VKDG_SMOKE_KEY".into(),
        },
        models: vec![model_pattern.into()],
        max_concurrent: 10,
        weight: 1,
        tags: vec![],
        endpoint: None,
        capabilities: CapabilitySet::default(),
    };
    let cfg2 = ConnectionConfig {
        id: id2.clone(),
        provider: provider_for(adapter.id(), second_url),
        auth: AuthKind::ApiKey {
            env_var: "VKDG_SMOKE_KEY".into(),
        },
        models: vec![model_pattern.into()],
        max_concurrent: 10,
        weight: 1,
        tags: vec![],
        endpoint: None,
        capabilities: CapabilitySet::default(),
    };
    let route = RouteConfig {
        id: RouteId("fallback-smoke".into()),
        match_models: vec![model_pattern.into()],
        strategy: StrategyKind::RoundRobin,
        targets: vec![id1, id2],
        plugin_hooks: PluginHooks::default(),
    };
    Arc::new(PipelineState::minimal(
        Arc::new(AdmissionGuard::new(10)),
        Arc::new(Router::new(vec![route])),
        Arc::new(ConnectionCatalog::new(vec![cfg1, cfg2])),
        Arc::new(CredentialManager::new()),
        Arc::new(HttpClient::new()),
        Arc::new(DecisionRecordExporter::new()),
        test_registry(),
    ))
}

/// Smoke: pipeline with `OpenAI` provider returns SSE stream from fake `OpenAI` upstream.
/// Defeito derrubado: `OpenAIAdapter` não serializa body corretamente, ou o pipeline
/// bloqueia/descarta bytes SSE em vez de fazer passthrough do upstream.
#[tokio::test]
async fn smoke_pipeline_openai_streaming_ok() {
    std::env::set_var("VKDG_SMOKE_KEY", "test-token");

    let fake = FakeUpstream::spawn(FakeUpstreamBehavior::OpenAIStreamOk {
        content: "hello openai".into(),
    })
    .await;

    let pipeline = make_pipeline_with(fake.base_url.clone(), &OpenAIAdapter, "gpt-*");
    let (ctx, op) = make_ctx_streaming("gpt-4o", ApiType::AnthropicMessages);

    let resp = run_conversation_pipeline(pipeline, ctx, op).await;

    assert_eq!(resp.status(), 200, "expected 200 for OpenAI streaming");
    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    assert!(
        ct.contains("text/event-stream"),
        "content-type must be text/event-stream, got: {ct}"
    );

    let body = axum::body::to_bytes(resp.into_body(), 64 * 1024)
        .await
        .unwrap();
    let s = String::from_utf8_lossy(&body);
    // An Anthropic client over an OpenAI upstream gets Anthropic SSE, not the
    // upstream's chunks: framed events, the text, usage, and no OpenAI sentinel.
    assert!(s.contains("event: message_start"), "{s}");
    assert!(
        s.contains("\"text\":\"hello openai\""),
        "body must contain content: {s}"
    );
    assert!(s.contains("event: message_stop"), "{s}");
    assert!(
        !s.contains("[DONE]"),
        "OpenAI sentinel leaked to an Anthropic client: {s}"
    );
    assert!(
        !s.contains("chat.completion.chunk"),
        "upstream chunks leaked: {s}"
    );
    let delta = s
        .split("\n\n")
        .find(|frame| frame.starts_with("event: message_delta"))
        .unwrap_or_else(|| panic!("no message_delta in {s}"));
    assert!(
        delta.contains("\"input_tokens\":10") && delta.contains("\"output_tokens\":5"),
        "usage must reach the Anthropic client: {delta}"
    );
    assert_eq!(fake.call_count(), 1, "upstream must be called exactly once");
}

/// Smoke: pipeline retries on 429 and routes to the second connection.
/// Defeito derrubado: retry ocorre após commit (não deveria), ou o segundo
/// candidato não é tentado quando o primeiro retorna 429.
#[tokio::test]
async fn smoke_fallback_anthropic_429_retries_openai() {
    std::env::set_var("VKDG_SMOKE_KEY", "test-token");

    // First fake returns 429; second returns success with the fallback content.
    let fake1 = FakeUpstream::spawn(FakeUpstreamBehavior::AnthropicOk429).await;
    let fake2 = FakeUpstream::spawn(FakeUpstreamBehavior::AnthropicStreamOk {
        content: "fallback worked".into(),
    })
    .await;

    // Two connections; round-robin selects conn-1 first; on 429 the pipeline
    // excludes it and re-routes to conn-2.
    let pipeline = make_pipeline_two_connections(
        fake1.base_url.clone(),
        fake2.base_url.clone(),
        &AnthropicAdapter,
        "claude-*",
    );

    let (ctx, op) = make_ctx_streaming("claude-3-5-haiku-20241022", ApiType::AnthropicMessages);

    let resp = run_conversation_pipeline(pipeline, ctx, op).await;

    assert_eq!(resp.status(), 200, "fallback must succeed with 200");
    let body = axum::body::to_bytes(resp.into_body(), 64 * 1024)
        .await
        .unwrap();
    let s = String::from_utf8_lossy(&body);
    assert!(
        s.contains("fallback worked"),
        "body must contain fallback content: {s}"
    );
    assert_eq!(
        fake1.call_count(),
        1,
        "first candidate must be attempted once"
    );
    assert_eq!(
        fake2.call_count(),
        1,
        "fallback candidate must be attempted once"
    );
}

/// Plausible wrong impl: with no sibling to fall back to, the retry fails with
/// "no eligible connection" (502) because the 429 just put the only connection
/// in cooldown, and that replaces the real error. The client must see the
/// upstream 429, not a gateway-made 502 that hides why.
#[tokio::test]
async fn sole_connection_429_reaches_the_client_as_429() {
    std::env::set_var("VKDG_SMOKE_KEY", "test-token");
    let fake = FakeUpstream::spawn(FakeUpstreamBehavior::AnthropicOk429).await;
    let pipeline = make_pipeline_with(fake.base_url.clone(), &AnthropicAdapter, "claude-*");
    let (ctx, op) = make_ctx_streaming("claude-3-5-haiku-20241022", ApiType::AnthropicMessages);

    let resp = run_conversation_pipeline(pipeline, ctx, op).await;

    assert_eq!(resp.status(), 429, "the upstream status must survive");
    assert_eq!(fake.call_count(), 1, "nothing else to try, no second call");
}

/// Plausible wrong impl: while the only connection cools down after a 429, every
/// request gets a gateway 502 "no eligible connection". Clients read 502 as an
/// outage and hammer it; a 429 with `retry-after` tells them to wait, and the
/// upstream must not be called again before the cooldown ends.
#[tokio::test]
async fn request_during_cooldown_gets_429_with_retry_after_not_502() {
    std::env::set_var("VKDG_SMOKE_KEY", "test-token");
    let fake = FakeUpstream::spawn(FakeUpstreamBehavior::AnthropicOk429).await;
    let pipeline = make_pipeline_with(fake.base_url.clone(), &AnthropicAdapter, "claude-*");

    let (ctx, op) = make_ctx_streaming("claude-3-5-haiku-20241022", ApiType::AnthropicMessages);
    let first = run_conversation_pipeline(Arc::clone(&pipeline), ctx, op).await;
    assert_eq!(first.status(), 429);

    let (ctx, op) = make_ctx_streaming("claude-3-5-haiku-20241022", ApiType::AnthropicMessages);
    let second = run_conversation_pipeline(pipeline, ctx, op).await;
    assert_eq!(
        second.status(),
        429,
        "cooling down is a rate limit, not an outage"
    );
    let retry_after: u32 = second
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .expect("retry-after header");
    assert!(retry_after >= 1, "retry-after={retry_after}");
    assert_eq!(fake.call_count(), 1, "no upstream call while cooling down");
}

/// Plausible wrong impl: stale session pin causes `NoEligibleConnection` (502)
/// instead of falling through to route-based selection when the pinned
/// connection no longer exists in the catalog.
#[tokio::test]
async fn session_stickiness_stale_pin_falls_through_to_routing() {
    std::env::set_var("VKDG_SMOKE_KEY", "test-token");

    let fake = FakeUpstream::spawn(FakeUpstreamBehavior::AnthropicOk {
        content: "ok".into(),
    })
    .await;

    let registry = SessionRegistry::new(3600);
    // Pin the session to a connection that does NOT exist in the catalog.
    registry
        .pin(
            "sess-stale".into(),
            ConnectionId("non-existent-conn".into()),
        )
        .await;

    let conn_id = ConnectionId("real-conn".into());
    let config = ConnectionConfig {
        id: conn_id.clone(),
        provider: ProviderKind::AnthropicCompat {
            base_url: fake.base_url.clone(),
        },
        auth: AuthKind::ApiKey {
            env_var: "VKDG_SMOKE_KEY".into(),
        },
        models: vec!["claude-*".into()],
        max_concurrent: 10,
        weight: 1,
        tags: vec![],
        endpoint: None,
        capabilities: CapabilitySet::default(),
    };
    let route = RouteConfig {
        id: RouteId("real".into()),
        match_models: vec!["claude-*".into()],
        strategy: StrategyKind::RoundRobin,
        targets: vec![conn_id],
        plugin_hooks: PluginHooks::default(),
    };
    let mut pipeline = PipelineState::minimal(
        Arc::new(AdmissionGuard::new(10)),
        Arc::new(Router::new(vec![route])),
        Arc::new(ConnectionCatalog::new(vec![config])),
        Arc::new(CredentialManager::new()),
        Arc::new(HttpClient::new()),
        Arc::new(DecisionRecordExporter::new()),
        test_registry(),
    );
    // SessionRegistry::new() already returns Arc<Self>; no extra wrapping needed.
    pipeline.session_registry = Some(registry);
    let pipeline = Arc::new(pipeline);

    let envelope = RequestEnvelope {
        request_id: RequestId::new(),
        client_id: ClientId("test".into()),
        tenant_id: TenantId("default".into()),
        session_key: Some(SessionKey("sess-stale".into())),
        api_type: ApiType::AnthropicMessages,
        model_requested: "claude-3-5-haiku-20241022".into(),
        deadline: None,
        mode_pack_override: None,
        compression_override: None,
        cache_bypass: false,
        include_think_tags: false,
        client_ip: None,
    };
    let (_, op) = make_ctx("claude-3-5-haiku-20241022");
    let ctx = PipelineCtx::new(envelope);

    let resp = run_conversation_pipeline(pipeline, ctx, op).await;

    assert_eq!(
        resp.status(),
        200,
        "stale session pin must fall through to route-based selection, got {}",
        resp.status()
    );
    assert_eq!(
        fake.call_count(),
        1,
        "real connection must be called exactly once"
    );
}

/// Plausible wrong impl: request with no explicit route fails immediately with
/// `NoEligibleConnection` instead of falling through to auto-route via `catalog.eligible()`.
/// Auto-routing must find the catalog connection even when the router has no routes.
#[tokio::test]
async fn auto_routing_zero_config_routes_without_explicit_route() {
    std::env::set_var("VKDG_SMOKE_KEY", "test-token");
    let fake = FakeUpstream::spawn(FakeUpstreamBehavior::AnthropicOk {
        content: "auto-routed".into(),
    })
    .await;

    let conn_id = ConnectionId("auto-conn".into());
    let config = ConnectionConfig {
        id: conn_id.clone(),
        provider: ProviderKind::AnthropicCompat {
            base_url: fake.base_url.clone(),
        },
        auth: AuthKind::ApiKey {
            env_var: "VKDG_SMOKE_KEY".into(),
        },
        models: vec!["claude-*".into()],
        max_concurrent: 10,
        weight: 1,
        tags: vec![],
        endpoint: None,
        capabilities: CapabilitySet::default(),
    };
    // EMPTY router — no explicit routes. Auto-routing must find the catalog connection.
    let pipeline = Arc::new(PipelineState::minimal(
        Arc::new(AdmissionGuard::new(10)),
        Arc::new(Router::new(vec![])),
        Arc::new(ConnectionCatalog::new(vec![config])),
        Arc::new(CredentialManager::new()),
        Arc::new(HttpClient::new()),
        Arc::new(DecisionRecordExporter::new()),
        test_registry(),
    ));

    let (ctx, op) = make_ctx("claude-3-5-haiku-20241022");
    let resp = run_conversation_pipeline(pipeline, ctx, op).await;

    assert_eq!(
        resp.status(),
        200,
        "auto-route must find catalog connection without explicit route, got {}",
        resp.status()
    );
    assert_eq!(
        fake.call_count(),
        1,
        "connection must be called exactly once"
    );
}

/// Plausible wrong impl: context-relay is called but `relay_on_rotation` is
/// never wired in inner.rs, so the new connection receives no conversation
/// history.  This test calls `relay_on_rotation` directly to verify it
/// injects the [Account rotation] block into the system prompt.
#[test]
fn context_relay_injects_history_on_account_rotation() {
    use vkdg_http::pipeline::phases::relay_on_rotation;

    let mut op = Operation::Conversation(ConversationRequest {
        model: "test-model".into(),
        messages: vec![
            Message {
                role: Role::User,
                content: MessageContent::Text("Hello".into()),
            },
            Message {
                role: Role::Assistant,
                content: MessageContent::Text("Hi there".into()),
            },
        ],
        tools: vec![],
        max_tokens: Some(10),
        temperature: None,
        stream: false,
        system: None,
        required_capabilities: CapabilitySet::default(),
        thinking: None,
        ..Default::default()
    });

    relay_on_rotation(
        &mut op,
        &ConnectionId("conn-a".into()),
        &ConnectionId("conn-b".into()),
    );

    let Operation::Conversation(req) = &op else {
        panic!("expected Conversation");
    };
    let system = req.system.as_deref().unwrap_or("");
    assert!(
        system.contains("Account rotation"),
        "relay block must mention rotation; got: {system:?}"
    );
    assert!(
        system.contains("conn-a"),
        "relay block must include source connection id; got: {system:?}"
    );
    assert!(
        system.contains("Hello") || system.contains("Hi there"),
        "relay block must include recent message content; got: {system:?}"
    );
}

/// Plausible wrong impl: relay fires even when `relay_enabled=false`, leaking
/// internal routing details into every system prompt.
/// Verifies `relay_on_rotation` is a no-op when there is no `system_preferred` pin
/// (i.e. no session pin means no rotation possible).
#[test]
fn context_relay_noop_on_non_conversation_operation() {
    use vkdg_http::pipeline::phases::relay_on_rotation;
    use vkdg_operations::ImageGenerateRequest;

    let mut op = Operation::ImageGenerate(ImageGenerateRequest {
        prompt: "a cat".into(),
        model: None,
        n: None,
        size: None,
        quality: None,
        style: None,
        response_format: None,
        user: None,
    });

    // Must not panic or alter a non-conversation operation.
    relay_on_rotation(
        &mut op,
        &ConnectionId("conn-a".into()),
        &ConnectionId("conn-b".into()),
    );

    // If we reach here without panic and op is still ImageGenerate, the guard works.
    assert!(
        matches!(op, Operation::ImageGenerate(_)),
        "non-conversation operation must be unchanged"
    );
}

/// Plausible wrong impl: a stream with zero content deltas (only `message_start` +
/// `message_delta` with `stop_reason`) produces an empty body or panics instead of
/// forwarding the SSE stream with a valid `stop_reason` event.
/// Verifies the pipeline does not crash on an empty content stream.
#[tokio::test]
async fn empty_stream_returns_valid_stop_reason() {
    std::env::set_var("VKDG_SMOKE_KEY", "test-token");

    let fake = FakeUpstream::spawn(FakeUpstreamBehavior::AnthropicStreamEmpty).await;
    let pipeline = make_pipeline(fake.base_url.clone(), 10);
    let (ctx, op) = make_ctx_streaming("claude-3-5-haiku-20241022", ApiType::AnthropicMessages);

    let response = run_conversation_pipeline(pipeline, ctx, op).await;
    let status = response.status();
    let ct = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    assert_eq!(status, 200, "empty stream must return 200, got {status}");
    assert!(
        ct.contains("text/event-stream"),
        "content-type must be text/event-stream for empty stream, got: {ct}"
    );

    // Drain the body; the stream must be readable without panic and must carry
    // the `message_delta` with `stop_reason` (forwarded verbatim from the upstream).
    let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap();
    let body_str = String::from_utf8_lossy(&body);
    assert!(
        body_str.contains("end_turn") || body_str.contains("stop_reason"),
        "empty stream body must contain stop_reason event, got: {body_str}"
    );
    assert_eq!(fake.call_count(), 1, "upstream must be called exactly once");
}

/// Uses the real Kiro converter and decoder; only the transport destination is
/// redirected to loopback so this smoke never contacts a paid upstream.
struct LocalKiroAdapter {
    url: String,
}

impl vkdg_provider_sdk::ProviderAdapter for LocalKiroAdapter {
    fn id(&self) -> &'static str {
        "kiro"
    }

    fn display_name(&self) -> &'static str {
        "Kiro loopback smoke"
    }

    fn prepare(
        &self,
        operation: &Operation,
        config: &ConnectionConfig,
        credential: &vkdg_connections::Credential,
    ) -> Result<vkdg_provider_sdk::PreparedRequest, vkdg_provider_sdk::ProviderError> {
        let mut prepared =
            vkdg_provider_kiro::KiroAdapter.prepare(operation, config, credential)?;
        prepared.url.clone_from(&self.url);
        Ok(prepared)
    }

    fn stream_decoder(&self) -> Option<Box<dyn vkdg_provider_sdk::ConversationStreamDecoder>> {
        vkdg_provider_kiro::KiroAdapter.stream_decoder()
    }
}

// Structural tokens from incident requests: no original text, argument, name,
// identifier, credential or URL is retained. A<n>e/t keeps call count and whether
// assistant text was empty; U<n> keeps the user's text-block count.
fn anonymized_history(shape: &str) -> Vec<serde_json::Value> {
    use serde_json::json;
    let mut messages = Vec::new();
    let mut pending = std::collections::VecDeque::new();
    let mut next_id = 0;
    for token in shape.split_whitespace() {
        match token.as_bytes()[0] {
            b'S' => messages.push(json!({"role":"system","content":"system instructions"})),
            b'U' => {
                let count: usize = token[1..].parse().expect("block count");
                messages.push(json!({"role":"user","content":(0..count)
                    .map(|_| json!({"type":"text","text":"user instruction"})).collect::<Vec<_>>()}));
            }
            b'A' => {
                let count: usize = token[1..token.len() - 1].parse().expect("call count");
                let calls: Vec<_> = (0..count).map(|_| {
                    let id = format!("call_{next_id}");
                    next_id += 1;
                    pending.push_back(id.clone());
                    json!({"id":id,"type":"function","function":{"name":"inspect","arguments":"{}"}})
                }).collect();
                messages.push(json!({"role":"assistant",
                    "content":if token.ends_with('e') { "" } else { "assistant text" },
                    "tool_calls":calls}));
            }
            b'T' => messages.push(json!({"role":"tool",
                "tool_call_id":pending.pop_front().expect("matching fixture call"),
                "content":"tool output"})),
            _ => panic!("unknown structural token"),
        }
    }
    assert!(pending.is_empty(), "fixture must have complete pairs");
    messages
}

/// Independent upstream validator: every result must answer the immediately
/// preceding assistant, and that assistant's calls must all be answered once.
fn strict_kiro_history(body: &serde_json::Value) -> Result<(), String> {
    let state = &body["conversationState"];
    let history = state["history"].as_array().ok_or("missing history")?;
    if history.len() + 1 > 100 {
        return Err("history cap exceeded".into());
    }
    let mut previous_assistant = false;
    let mut pending = Vec::new();
    for (i, item) in history
        .iter()
        .chain(std::iter::once(&state["currentMessage"]))
        .enumerate()
    {
        if let Some(assistant) = item.get("assistantResponseMessage") {
            if previous_assistant || !pending.is_empty() {
                return Err(format!("unanswered assistant at turn {i}"));
            }
            pending = assistant["toolUses"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|u| u["toolUseId"].as_str())
                .collect();
            previous_assistant = true;
        } else {
            let user = &item["userInputMessage"];
            let results: Vec<_> = user["userInputMessageContext"]["toolResults"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|r| r["toolUseId"].as_str())
                .collect();
            if results != pending {
                return Err(format!("TOOL_USE_RESULT_MISMATCH at turn {i}"));
            }
            if i > 0 && !previous_assistant {
                return Err(format!("consecutive users at turn {i}"));
            }
            pending.clear();
            previous_assistant = false;
        }
    }
    if !pending.is_empty() {
        return Err("unanswered final call".into());
    }
    if !history[0]["userInputMessage"]["content"]
        .as_str()
        .is_some_and(|s| s.contains("system instructions"))
    {
        return Err("system instructions lost".into());
    }
    Ok(())
}

/// Refutes a cap that splits tool exchanges after real Chat Completions ingress
/// and pipeline dispatch, including the next short user confirmation.
#[tokio::test]
async fn smoke_kiro_history_cap_real_http_ingress() {
    use axum::{extract::State, response::IntoResponse, routing::post, Json};
    use serde_json::{json, Value};
    use std::sync::Mutex;
    use tokio::net::TcpListener;
    use vkdg_http::{AppState, ServerConfig};

    let observed = Arc::new(Mutex::new(Vec::<Value>::new()));
    let upstream = axum::Router::new()
        .route(
            "/generateAssistantResponse",
            post(
                |State(seen): State<Arc<Mutex<Vec<Value>>>>, bytes: bytes::Bytes| async move {
                    let body: Value = serde_json::from_slice(&bytes).expect("Kiro JSON request");
                    let validation = strict_kiro_history(&body);
                    seen.lock().expect("observed requests").push(body);
                    if let Err(reason) = validation {
                        return (
                            http::StatusCode::BAD_REQUEST,
                            Json(json!({"reason":reason})),
                        )
                            .into_response();
                    }
                    let mut bytes = Vec::new();
                    let frame = aws_smithy_types::event_stream::Message::new(
                        json!({"content":"confirmed"}).to_string(),
                    )
                    .add_header(aws_smithy_types::event_stream::Header::new(
                        ":message-type",
                        aws_smithy_types::event_stream::HeaderValue::String("event".into()),
                    ))
                    .add_header(aws_smithy_types::event_stream::Header::new(
                        ":event-type",
                        aws_smithy_types::event_stream::HeaderValue::String(
                            "assistantResponseEvent".into(),
                        ),
                    ));
                    aws_smithy_eventstream::frame::write_message_to(&frame, &mut bytes)
                        .expect("event frame");
                    (
                        http::StatusCode::OK,
                        [("content-type", "application/vnd.amazon.eventstream")],
                        bytes,
                    )
                        .into_response()
                },
            ),
        )
        .with_state(observed.clone());
    let upstream_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("upstream bind");
    let upstream_url = format!(
        "http://{}/generateAssistantResponse",
        upstream_listener.local_addr().expect("upstream addr")
    );
    let upstream_task = tokio::spawn(async move {
        axum::serve(upstream_listener, upstream)
            .await
            .expect("upstream serve");
    });
    std::env::set_var("VKDG_KIRO_CAP_SMOKE_KEY", "synthetic-key");
    let conn_id = ConnectionId("kiro-cap-smoke".into());
    let config = ConnectionConfig {
        id: conn_id.clone(),
        provider: ProviderKind::Plugin { id: "kiro".into() },
        auth: AuthKind::ApiKey {
            env_var: "VKDG_KIRO_CAP_SMOKE_KEY".into(),
        },
        models: vec!["claude-*".into()],
        max_concurrent: 1,
        weight: 1,
        tags: vec![],
        endpoint: None,
        capabilities: CapabilitySet::default(),
    };
    let route = RouteConfig {
        id: RouteId("kiro-cap-smoke".into()),
        match_models: vec!["claude-*".into()],
        strategy: StrategyKind::RoundRobin,
        targets: vec![conn_id],
        plugin_hooks: PluginHooks::default(),
    };
    let mut registry = ProviderRegistry::empty();
    registry.register(Arc::new(LocalKiroAdapter { url: upstream_url }));
    let pipeline = Arc::new(PipelineState::minimal(
        Arc::new(AdmissionGuard::new(1)),
        Arc::new(Router::new(vec![route])),
        Arc::new(ConnectionCatalog::new(vec![config])),
        Arc::new(CredentialManager::new()),
        Arc::new(HttpClient::new()),
        Arc::new(DecisionRecordExporter::new()),
        Arc::new(registry),
    ));
    let gateway = axum::Router::new()
        .route(
            "/v1/chat/completions",
            post(vkdg_ingress_openai::handle_chat_completions),
        )
        .route_layer(axum::middleware::from_fn_with_state(
            vkdg_http::DataAuth::disabled(),
            vkdg_http::require_api_key,
        ))
        .with_state(AppState::new(ServerConfig::default()).with_pipeline(pipeline));
    let gateway_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("gateway bind");
    let gateway_url = format!(
        "http://{}/v1/chat/completions",
        gateway_listener.local_addr().expect("gateway addr")
    );
    let gateway_task = tokio::spawn(async move {
        axum::serve(gateway_listener, gateway)
            .await
            .expect("gateway serve");
    });
    let shapes = [
        (111, "S U2 U1 A1e T A2e T T A1e T A1e T A1e T A2e T T A2e T T A2e T T A2e T T A1e T A1e T A1e T A1e T A1e T A1e T A1e T A1e T A1e T A2e T T A1t T A1e T A1e T A1t T A1e T A1e T A1e T A1e T A0t U1 A2e T T A1t T A1e T A1e T A1e T A1e T A1e T A1e T A1e T A1t T A2e T T A1e T A1e T A1t T A1t T A1e T A1t T A1e T A1t T A1e T A1e T A0t U1"),
        (113, "S U2 U1 A1e T A2e T T A1e T A1e T A1e T A2e T T A2e T T A2e T T A2e T T A1e T A1e T A1e T A1e T A1e T A1e T A1e T A1e T A1e T A2e T T A1t T A1e T A1e T A1t T A1e T A1e T A1e T A1e T A0t U1 A2e T T A1t T A1e T A1e T A1e T A1e T A1e T A1e T A1e T A1t T A2e T T A1e T A1e T A1t T A1t T A1e T A1t T A1e T A1t T A1e T A1e T A0t U1 U1 U1"),
        (103, "S U2 A1t T A1t T A1t T A1t T A1e T A1t T A1t T A1t T A1t T A1e T A1t T A1t T A1t T A1e T U1 A1t T A1t T A1t T A1t T A1t T A1t T A1t T A1t T A1t T A1t T A1t T A1e T A1t T A1t T A1t T A1t T A1t T A1t T A1t T A1t T A1t T A1t T A1t T A1t T A1e T A1t T A1t T A1t T A1e T A1t T A1e T A1t T A1t T A1t T A1t T A1e T"),
        (161, "S U2 A1e T A1e T A3e T T T A2t T T U1 A2e T T U1 A2t T T A2t T T A2e T T A2e T T A1t T A1e T A1e T A1e T A1t T A1e T A2t T T A1e T A1e T A1t T A2t T T A1e T U1 A2t T T A1e T A1t T A1e T A1t T A1e T A2e T T A1t T A1e T A1e T A1e T A1t T A1e T A1t T A1e T A1t T U1 A2t T T U1 A3t T T T A1e T A2e T T A2e T T A2e T T A2e T T A2e T T A2e T T A2e T T A1e T U1 A2t T T U1 A1t T A1e T A1e T A1t T A1e T A1t T A1t T A1e T A1e T A1e T A1t T U1 A1e T U1 A1t T U1 U1 U1"),
    ];
    let client = reqwest::Client::new();
    for (count, shape) in shapes {
        let mut messages = anonymized_history(shape);
        assert_eq!(messages.len(), count, "incident shape message count");
        for followup in [false, true] {
            if followup {
                messages.push(json!({"role":"assistant","content":"confirmed"}));
                messages.push(json!({"role":"user","content":"Sim, pf"}));
            }
            let response = client.post(&gateway_url).json(&json!({
                "model":"claude-sonnet-4.5","messages":messages,"stream":true,
                "tools":[{"type":"function","function":{"name":"inspect","parameters":{"type":"object"}}}]
            })).send().await.expect("gateway request");
            assert_eq!(
                response.status(),
                http::StatusCode::OK,
                "shape {count}, followup={followup}"
            );
            let stream = response.text().await.expect("client stream");
            assert!(
                stream.contains("confirmed"),
                "upstream answer must reach client"
            );
            assert!(stream.contains("[DONE]"), "client stream must complete");
            let seen = observed.lock().expect("observed body");
            let sent = seen.last().expect("one upstream request");
            assert!(strict_kiro_history(sent).is_ok());
            let history = sent["conversationState"]["history"]
                .as_array()
                .expect("history");
            let calls: usize = history
                .iter()
                .map(|item| {
                    item["assistantResponseMessage"]["toolUses"]
                        .as_array()
                        .map_or(0, Vec::len)
                })
                .sum();
            println!("sanitized shape={count} followup={followup} downstream_turns={} tool_calls={calls} adjacency=valid system=preserved current=preserved",
                history.len() + 1);
            if followup {
                assert_eq!(
                    sent["conversationState"]["currentMessage"]["userInputMessage"]["content"],
                    "Sim, pf"
                );
            }
        }
    }
    assert_eq!(observed.lock().expect("requests").len(), 8);
    gateway_task.abort();
    upstream_task.abort();
}
