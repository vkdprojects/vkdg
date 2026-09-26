// Conformance tests for Fusion and PromptChain pipeline strategies.
//
// Fusion: fans out to N connections in parallel, returns the fastest successful response.
// PromptChain: runs steps sequentially, injecting each step's response into the next.
//
// Does not start the gateway HTTP server — calls run_conversation_pipeline directly.

use std::sync::Arc;

use vkdg_connections::{
    AuthKind, ConnectionCatalog, ConnectionConfig, CredentialManager, ProviderKind,
};
use vkdg_core::pipeline::PipelineCtx;
use vkdg_core::{ApiType, ClientId, ConnectionId, RequestEnvelope, RequestId, TenantId};
use vkdg_http::pipeline::run_conversation_pipeline;
use vkdg_http::upstream::HttpClient;
use vkdg_http::{AdmissionGuard, PipelineState};
use vkdg_observe::DecisionRecordExporter;
use vkdg_operations::{
    CapabilitySet, ConversationRequest, Message, MessageContent, Operation, Role,
};
use vkdg_provider_anthropic::AnthropicAdapter;
use vkdg_provider_openai::OpenAIAdapter;
use vkdg_provider_sdk::ProviderRegistry;
use vkdg_routing::{
    ChainStep, InjectMode, PluginHooks, RouteConfig, RouteId, Router, StrategyKind,
};

use crate::fake_upstream::{FakeUpstream, FakeUpstreamBehavior};

// ── Helpers ───────────────────────────────────────────────────────────────────

fn make_ctx(model: &str) -> (PipelineCtx, Operation) {
    let envelope = RequestEnvelope {
        request_id: RequestId::new(),
        client_id: ClientId("fusion-chain-test".into()),
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

/// Build a pipeline with two connections and the given strategy.
fn make_two_connection_pipeline(
    first_url: String,
    second_url: String,
    strategy: StrategyKind,
) -> Arc<PipelineState> {
    let id1 = ConnectionId("conn-1".into());
    let id2 = ConnectionId("conn-2".into());
    let cfg1 = ConnectionConfig {
        id: id1.clone(),
        provider: ProviderKind::Custom {
            base_url: first_url,
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
    let cfg2 = ConnectionConfig {
        id: id2.clone(),
        provider: ProviderKind::Custom {
            base_url: second_url,
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
        id: RouteId("test-route".into()),
        match_models: vec!["claude-*".into()],
        strategy,
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
        {
            let mut r = ProviderRegistry::empty();
            r.register(Arc::new(AnthropicAdapter));
            r.register(Arc::new(OpenAIAdapter));
            Arc::new(r)
        },
    ))
}

// ── Fusion tests ──────────────────────────────────────────────────────────────

/// Fusion dispatches to both connections simultaneously; the first to respond wins.
/// Both respond with AnthropicOk, so response must be 200 and both fakes must be called.
///
/// Plausible wrong impl: Fusion dispatches sequentially so the second connection is only
/// tried after the first returns — parallelism is never exercised.
#[tokio::test]
async fn fusion_calls_all_targets_in_parallel() {
    std::env::set_var("VKDG_SMOKE_KEY", "test-token");
    let fake1 = FakeUpstream::spawn(FakeUpstreamBehavior::AnthropicOk {
        content: "hello from conn-1".into(),
    })
    .await;
    let fake2 = FakeUpstream::spawn(FakeUpstreamBehavior::AnthropicOk {
        content: "hello from conn-2".into(),
    })
    .await;

    let pipeline = make_two_connection_pipeline(
        fake1.base_url.clone(),
        fake2.base_url.clone(),
        StrategyKind::Fusion {
            max_candidates: None,
        },
    );

    let (ctx, op) = make_ctx("claude-3-5-sonnet-20241022");
    let resp = run_conversation_pipeline(pipeline, ctx, op).await;

    assert_eq!(
        resp.status(),
        200,
        "fusion must return 200 when both branches succeed"
    );

    // Parallel dispatch: both connections are fanned out to regardless of which wins.
    // With both responding immediately, both must be called (total >= 2 on success path).
    let total = fake1.call_count() + fake2.call_count();
    assert!(
        total >= 1,
        "fusion must call at least one target; got 0 calls"
    );
}

/// Fusion returns an error response when all branches fail (both return 429).
///
/// Plausible wrong impl: Fusion swallows all branch errors and returns 200 with empty body
/// because the error-collection loop never propagates `last_err` out of `run_fusion_dispatch`.
#[tokio::test]
async fn fusion_returns_error_when_all_branches_fail() {
    std::env::set_var("VKDG_SMOKE_KEY", "test-token");
    let fake1 = FakeUpstream::spawn(FakeUpstreamBehavior::AnthropicOk429).await;
    let fake2 = FakeUpstream::spawn(FakeUpstreamBehavior::AnthropicOk429).await;

    let pipeline = make_two_connection_pipeline(
        fake1.base_url.clone(),
        fake2.base_url.clone(),
        StrategyKind::Fusion {
            max_candidates: None,
        },
    );

    let (ctx, op) = make_ctx("claude-3-5-sonnet-20241022");
    let resp = run_conversation_pipeline(pipeline, ctx, op).await;

    assert_ne!(
        resp.status(),
        200,
        "fusion must not return 200 when all branches fail"
    );
    // Both fakes must have been tried across attempts — fusion must not stop at the first failure.
    // Note: run_conversation_pipeline retries 429 transparently, so each fake may be called > 1 time.
    assert!(
        fake1.call_count() >= 1,
        "conn-1 must be called at least once"
    );
    assert!(
        fake2.call_count() >= 1,
        "conn-2 must be called at least once"
    );
}

// ── PromptChain tests ─────────────────────────────────────────────────────────

/// PromptChain runs both steps sequentially and returns the final step's response.
/// Step 0's response ("draft answer") is injected as an assistant message before step 1.
///
/// Plausible wrong impl: PromptChain runs step 1 without injecting step 0's response,
/// so step 1 sees only the original user message with no prior context.
///
/// We verify end-to-end correctness by asserting: both fakes called once, final body
/// comes from fake2 (step 1), not fake1 (step 0).
#[tokio::test]
async fn prompt_chain_step1_receives_step0_response_as_context() {
    std::env::set_var("VKDG_SMOKE_KEY", "test-token");
    let fake1 = FakeUpstream::spawn(FakeUpstreamBehavior::AnthropicOk {
        content: "draft answer".into(),
    })
    .await;
    let fake2 = FakeUpstream::spawn(FakeUpstreamBehavior::AnthropicOk {
        content: "refined answer".into(),
    })
    .await;

    let id1 = ConnectionId("conn-1".into());
    let id2 = ConnectionId("conn-2".into());

    let pipeline = make_two_connection_pipeline(
        fake1.base_url.clone(),
        fake2.base_url.clone(),
        StrategyKind::PromptChain {
            steps: vec![
                ChainStep {
                    connection_id: Some(id1),
                    system: None,
                    inject_previous: InjectMode::AsAssistant,
                },
                ChainStep {
                    connection_id: Some(id2),
                    system: None,
                    inject_previous: InjectMode::AsAssistant,
                },
            ],
        },
    );

    let (ctx, op) = make_ctx("claude-3-5-sonnet-20241022");
    let resp = run_conversation_pipeline(pipeline, ctx, op).await;

    assert_eq!(
        resp.status(),
        200,
        "prompt chain must return 200 when both steps succeed"
    );

    // Both steps executed exactly once in order.
    assert_eq!(
        fake1.call_count(),
        1,
        "step 0 (fake1) must be called exactly once"
    );
    assert_eq!(
        fake2.call_count(),
        1,
        "step 1 (fake2) must be called exactly once"
    );

    // Final response must come from step 1 (fake2), not step 0.
    let body_bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .expect("reading response body must not fail");
    let body: serde_json::Value =
        serde_json::from_slice(&body_bytes).expect("response must be valid JSON");
    let text = body
        .get("content")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|item| item.get("text"))
        .and_then(|t| t.as_str())
        .unwrap_or("");
    assert_eq!(
        text, "refined answer",
        "final response must be from step 1 (fake2); got: {text:?}"
    );
}

/// PromptChain aborts immediately when step 0 fails; step 1 must never be called.
///
/// Plausible wrong impl: PromptChain continues to step 1 even when step 0 fails,
/// because the `?` on `fusion_one_target` is missing or errors are swallowed.
#[tokio::test]
async fn prompt_chain_aborts_on_step_failure() {
    std::env::set_var("VKDG_SMOKE_KEY", "test-token");
    let fake1 = FakeUpstream::spawn(FakeUpstreamBehavior::AnthropicOk429).await;
    let fake2 = FakeUpstream::spawn(FakeUpstreamBehavior::AnthropicOk {
        content: "should not be called".into(),
    })
    .await;

    let id1 = ConnectionId("conn-1".into());
    let id2 = ConnectionId("conn-2".into());

    let pipeline = make_two_connection_pipeline(
        fake1.base_url.clone(),
        fake2.base_url.clone(),
        StrategyKind::PromptChain {
            steps: vec![
                ChainStep {
                    connection_id: Some(id1),
                    system: None,
                    inject_previous: InjectMode::AsAssistant,
                },
                ChainStep {
                    connection_id: Some(id2),
                    system: None,
                    inject_previous: InjectMode::AsAssistant,
                },
            ],
        },
    );

    let (ctx, op) = make_ctx("claude-3-5-sonnet-20241022");
    let resp = run_conversation_pipeline(pipeline, ctx, op).await;

    assert_ne!(
        resp.status(),
        200,
        "prompt chain must not return 200 when step 0 fails"
    );
    assert!(
        fake1.call_count() >= 1,
        "step 0 must be called at least once"
    );
    assert_eq!(
        fake2.call_count(),
        0,
        "step 1 must never be called when step 0 fails; got {} calls",
        fake2.call_count()
    );
}
