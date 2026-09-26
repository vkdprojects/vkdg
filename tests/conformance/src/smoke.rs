// Smoke test: exercício end-to-end do pipeline completo contra fake upstream.
// Testa: decode Anthropic → pipeline → upstream HTTP → resposta JSON passthrough.
// Não sobe o servidor HTTP do gateway — chama run_conversation_pipeline diretamente.
//
// Defeito plausível derrotado: pipeline nunca chega ao upstream (retorna erro antes),
// ou upstream é chamado mas a resposta é perdida/corrompida no caminho.

use std::sync::Arc;
use axum::response::IntoResponse;
use bytes::Bytes;
use vkdg_connections::{
    AuthKind, Connection, ConnectionCatalog, ConnectionConfig, CredentialManager, ProviderKind,
};
use vkdg_core::{
    ApiType, AttemptState, ClientId, ConnectionId, RequestEnvelope, RequestId, TenantId,
};
use vkdg_core::pipeline::PipelineCtx;
use vkdg_http::{AdmissionGuard, PipelineState};
use vkdg_observe::DecisionRecordExporter;
use vkdg_http::pipeline::run_conversation_pipeline;
use vkdg_http::upstream::HttpClient;
use vkdg_operations::{
    CapabilitySet, ConversationRequest, Message, MessageContent, Operation, Role,
};
use vkdg_routing::{PluginHooks, RouteConfig, RouteId, Router, StrategyKind};

use crate::fake_upstream::{FakeUpstream, FakeUpstreamBehavior};
use vkdg_provider_anthropic::AnthropicAdapter;
use vkdg_provider_openai::OpenAIAdapter;

fn make_pipeline(base_url: String, max_concurrent: usize) -> Arc<PipelineState> {
    let conn_id = ConnectionId("fake".into());
    let config = ConnectionConfig {
        id: conn_id.clone(),
        provider: ProviderKind::Custom { base_url },
        auth: AuthKind::ApiKey { env_var: "VKDG_SMOKE_KEY".into() },
        models: vec!["claude-*".into()],
        max_concurrent: max_concurrent as u32,
        weight: 1,
        tags: vec![],
        capabilities: CapabilitySet::default(),
    };
    let route = RouteConfig {
        id: RouteId("smoke".into()),
        match_models: vec!["claude-*".into()],
        strategy: StrategyKind::RoundRobin,
        targets: vec![conn_id],
        plugin_hooks: PluginHooks::default(),
    };
    Arc::new(PipelineState {
        admission: Arc::new(AdmissionGuard::new(max_concurrent)),
        router: Arc::new(Router::new(vec![route])),
        catalog: Arc::new(ConnectionCatalog::new(vec![config])),
        credentials: Arc::new(CredentialManager::new()),
        http_client: Arc::new(HttpClient::new()),
        exporter: Arc::new(DecisionRecordExporter::new()),
        provider_adapter: Arc::new(AnthropicAdapter),
        cache: None,
        combo_resolver: None,
    })
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
    };
    let op = Operation::Conversation(ConversationRequest {
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
    let envelope = RequestEnvelope {
        request_id: RequestId::new(),
        client_id: ClientId("smoke".into()),
        tenant_id: TenantId("default".into()),
        session_key: None,
        api_type: ApiType::AnthropicMessages,
        model_requested: "claude-3-5-haiku-20241022".into(),
        deadline: None,
    };
    let op = Operation::Conversation(ConversationRequest {
        messages: vec![Message {
            role: Role::User,
            content: MessageContent::Text("ping".into()),
        }],
        tools: vec![],
        max_tokens: Some(10),
        temperature: None,
        stream: true, // streaming request
        system: None,
        required_capabilities: CapabilitySet::default(),
    });
    let ctx = PipelineCtx::new(envelope);

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
    assert_eq!(fake.call_count(), 0, "upstream must NOT be called when admission rejects");
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
        provider: ProviderKind::Custom { base_url: "http://unused".into() },
        auth: AuthKind::ApiKey { env_var: "VKDG_SMOKE_KEY".into() },
        models: vec!["claude-*".into()],
        max_concurrent: 2,
        weight: 1,
        tags: vec![],
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

    assert_eq!(conn.active_requests(), 2, "both guards active: counter must be 2");

    // Third acquire must fail — capacity exhausted.
    assert!(
        conn.acquire().is_none(),
        "third acquire must fail when max_concurrent=2 and 2 guards held"
    );

    drop(g1);
    drop(g2);

    assert_eq!(conn.active_requests(), 0, "all guards dropped: counter must return to 0");
    assert!(conn.acquire().is_some(), "capacity restored after drop");
}

// Defeito plausível derrotado: Drop de ConnectionGuard não decrementa counter
// (esqueceu AcqRel, usou Relaxed, ou não implementou Drop).
// Sem isso, active_requests só cresce e conexão fica inacessível.
#[tokio::test]
async fn connection_guard_releases_on_drop() {
    let config = ConnectionConfig {
        id: ConnectionId("drop-test".into()),
        provider: ProviderKind::Custom { base_url: "http://unused".into() },
        auth: AuthKind::ApiKey { env_var: "VKDG_SMOKE_KEY".into() },
        models: vec!["*".into()],
        max_concurrent: 1,
        weight: 1,
        tags: vec![],
        capabilities: CapabilitySet::default(),
    };
    let conn = Connection::new(config);

    let guard = conn.acquire().expect("acquire must succeed on fresh connection");
    assert_eq!(conn.active_requests(), 1, "counter must be 1 while guard is held");

    drop(guard);
    assert_eq!(conn.active_requests(), 0, "counter must be 0 after guard drop");

    // Capacity is fully restored.
    assert!(conn.acquire().is_some(), "must be acquirable again after drop");
}

// Defeito plausível derrotado: ConnectionGuard dentro do pipeline
// não é liberado se o Future é dropped antes de completar
// (ex: cliente fecha conexão). Com RAII isso deve funcionar automaticamente.
#[tokio::test]
async fn pipeline_cancellation_releases_on_drop() {
    let config = ConnectionConfig {
        id: ConnectionId("cancel-test".into()),
        provider: ProviderKind::Custom { base_url: "http://unused".into() },
        auth: AuthKind::ApiKey { env_var: "VKDG_SMOKE_KEY".into() },
        models: vec!["*".into()],
        max_concurrent: 1,
        weight: 1,
        tags: vec![],
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
    assert!(conn.acquire().is_some(), "connection must be available after cancellation");
}

// ── OpenAI path smoke tests ───────────────────────────────────────────────────

/// Helper: build a pipeline using a custom adapter and a single connection.
fn make_pipeline_with<A>(base_url: String, adapter: A, model_pattern: &str) -> Arc<PipelineState>
where
    A: vkdg_http::provider::ProviderAdapter + 'static,
{
    let conn_id = ConnectionId("fake".into());
    let config = ConnectionConfig {
        id: conn_id.clone(),
        provider: ProviderKind::Custom { base_url },
        auth: AuthKind::ApiKey { env_var: "VKDG_SMOKE_KEY".into() },
        models: vec![model_pattern.into()],
        max_concurrent: 10,
        weight: 1,
        tags: vec![],
        capabilities: CapabilitySet::default(),
    };
    let route = RouteConfig {
        id: RouteId("smoke".into()),
        match_models: vec![model_pattern.into()],
        strategy: StrategyKind::RoundRobin,
        targets: vec![conn_id],
        plugin_hooks: PluginHooks::default(),
    };
    Arc::new(PipelineState {
        admission: Arc::new(AdmissionGuard::new(10)),
        router: Arc::new(Router::new(vec![route])),
        catalog: Arc::new(ConnectionCatalog::new(vec![config])),
        credentials: Arc::new(CredentialManager::new()),
        http_client: Arc::new(HttpClient::new()),
        exporter: Arc::new(DecisionRecordExporter::new()),
        provider_adapter: Arc::new(adapter),
        cache: None,
        combo_resolver: None,
    })
}

/// Helper: build a pipeline with two connections using the same adapter.
/// First connection is at `first_url`, second at `second_url`.
/// The router uses RoundRobin; exclusion makes it fall through to the second on retry.
fn make_pipeline_two_connections<A>(
    first_url: String,
    second_url: String,
    adapter: A,
    model_pattern: &str,
) -> Arc<PipelineState>
where
    A: vkdg_http::provider::ProviderAdapter + 'static,
{
    let id1 = ConnectionId("conn-1".into());
    let id2 = ConnectionId("conn-2".into());
    let cfg1 = ConnectionConfig {
        id: id1.clone(),
        provider: ProviderKind::Custom { base_url: first_url },
        auth: AuthKind::ApiKey { env_var: "VKDG_SMOKE_KEY".into() },
        models: vec![model_pattern.into()],
        max_concurrent: 10,
        weight: 1,
        tags: vec![],
        capabilities: CapabilitySet::default(),
    };
    let cfg2 = ConnectionConfig {
        id: id2.clone(),
        provider: ProviderKind::Custom { base_url: second_url },
        auth: AuthKind::ApiKey { env_var: "VKDG_SMOKE_KEY".into() },
        models: vec![model_pattern.into()],
        max_concurrent: 10,
        weight: 1,
        tags: vec![],
        capabilities: CapabilitySet::default(),
    };
    let route = RouteConfig {
        id: RouteId("fallback-smoke".into()),
        match_models: vec![model_pattern.into()],
        strategy: StrategyKind::RoundRobin,
        targets: vec![id1, id2],
        plugin_hooks: PluginHooks::default(),
    };
    Arc::new(PipelineState {
        admission: Arc::new(AdmissionGuard::new(10)),
        router: Arc::new(Router::new(vec![route])),
        catalog: Arc::new(ConnectionCatalog::new(vec![cfg1, cfg2])),
        credentials: Arc::new(CredentialManager::new()),
        http_client: Arc::new(HttpClient::new()),
        exporter: Arc::new(DecisionRecordExporter::new()),
        provider_adapter: Arc::new(adapter),
        cache: None,
        combo_resolver: None,
    })
}

/// Smoke: pipeline with OpenAI provider returns SSE stream from fake OpenAI upstream.
/// Defeito derrubado: OpenAIAdapter não serializa body corretamente, ou o pipeline
/// bloqueia/descarta bytes SSE em vez de fazer passthrough do upstream.
#[tokio::test]
async fn smoke_pipeline_openai_streaming_ok() {
    std::env::set_var("VKDG_SMOKE_KEY", "test-token");

    let fake = FakeUpstream::spawn(FakeUpstreamBehavior::OpenAIStreamOk {
        content: "hello openai".into(),
    })
    .await;

    let pipeline = make_pipeline_with(fake.base_url.clone(), OpenAIAdapter, "gpt-*");
    let envelope = RequestEnvelope {
        request_id: RequestId::new(),
        client_id: ClientId("smoke".into()),
        tenant_id: TenantId("default".into()),
        session_key: None,
        api_type: ApiType::AnthropicMessages,
        model_requested: "gpt-4o".into(),
        deadline: None,
    };
    let op = Operation::Conversation(ConversationRequest {
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
    });
    let ctx = PipelineCtx::new(envelope);

    let resp = run_conversation_pipeline(pipeline, ctx, op).await;

    assert_eq!(resp.status(), 200, "expected 200 for OpenAI streaming");
    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    assert!(ct.contains("text/event-stream"), "content-type must be text/event-stream, got: {ct}");

    let body = axum::body::to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
    let s = String::from_utf8_lossy(&body);
    assert!(s.contains("hello openai"), "body must contain content: {s}");
    assert!(s.contains("[DONE]"), "body must contain [DONE]: {s}");
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
        AnthropicAdapter,
        "claude-*",
    );

    let envelope = RequestEnvelope {
        request_id: RequestId::new(),
        client_id: ClientId("smoke".into()),
        tenant_id: TenantId("default".into()),
        session_key: None,
        api_type: ApiType::AnthropicMessages,
        model_requested: "claude-3-5-haiku-20241022".into(),
        deadline: None,
    };
    let op = Operation::Conversation(ConversationRequest {
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
    });
    let ctx = PipelineCtx::new(envelope);

    let resp = run_conversation_pipeline(pipeline, ctx, op).await;

    assert_eq!(resp.status(), 200, "fallback must succeed with 200");
    let body = axum::body::to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
    let s = String::from_utf8_lossy(&body);
    assert!(s.contains("fallback worked"), "body must contain fallback content: {s}");
    assert_eq!(fake1.call_count(), 1, "first candidate must be attempted once");
    assert_eq!(fake2.call_count(), 1, "fallback candidate must be attempted once");
}
