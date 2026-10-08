// Conversation affinity: the first request of a conversation picks a connection
// through the route's strategy; every later turn stays on that connection so the
// provider's per-account prompt cache keeps hitting.
//
// No client sends a session key, so the gateway derives one from the start of the
// conversation. These tests drive the pipeline with `session_key: None`, the way
// the ingress crates hand requests over.

use std::sync::Arc;

use vkdg_connections::{
    AuthKind, ConnectionCatalog, ConnectionConfig, ConnectionState, CredentialManager,
    ProviderKind, SessionRegistry,
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
use vkdg_provider_sdk::ProviderRegistry;
use vkdg_routing::{PluginHooks, RouteConfig, RouteId, Router, StrategyKind};

use crate::fake_upstream::{FakeUpstream, FakeUpstreamBehavior};

const MODEL: &str = "claude-3-5-haiku-20241022";

struct Fixture {
    pipeline: Arc<PipelineState>,
    upstreams: Vec<FakeUpstream>,
    ids: Vec<ConnectionId>,
}

async fn fixture() -> Fixture {
    std::env::set_var("VKDG_SMOKE_KEY", "test-token");
    let mut upstreams = Vec::new();
    let mut ids = Vec::new();
    let mut configs = Vec::new();
    for n in 0..2 {
        let fake = FakeUpstream::spawn(FakeUpstreamBehavior::AnthropicOk {
            content: "ok".into(),
        })
        .await;
        let id = ConnectionId(format!("acct-{n}"));
        configs.push(ConnectionConfig {
            id: id.clone(),
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
        });
        ids.push(id);
        upstreams.push(fake);
    }
    let route = RouteConfig {
        id: RouteId("claude".into()),
        match_models: vec!["claude-*".into()],
        // Alternates on every request: the strategy a per-request balancer uses.
        strategy: StrategyKind::RoundRobin,
        targets: ids.clone(),
        plugin_hooks: PluginHooks::default(),
    };
    let mut registry = ProviderRegistry::empty();
    registry.register(Arc::new(AnthropicAdapter));
    let mut pipeline = PipelineState::minimal(
        Arc::new(AdmissionGuard::new(10)),
        Arc::new(Router::new(vec![route])),
        Arc::new(ConnectionCatalog::new(configs)),
        Arc::new(CredentialManager::new()),
        Arc::new(HttpClient::new()),
        Arc::new(DecisionRecordExporter::new()),
        Arc::new(registry),
    );
    pipeline.session_registry = Some(SessionRegistry::new(3600));
    Fixture {
        pipeline: Arc::new(pipeline),
        upstreams,
        ids,
    }
}

fn text(role: Role, s: &str) -> Message {
    Message {
        role,
        content: MessageContent::Text(s.into()),
    }
}

/// Turn `turn` (0-based) of the conversation that opened with `opening`: the
/// history grows by an assistant/user pair per turn, as a real client resends it.
fn conversation(opening: &str, turn: usize) -> Vec<Message> {
    let mut messages = vec![text(Role::User, opening)];
    for t in 0..turn {
        messages.push(text(Role::Assistant, &format!("answer {t}")));
        messages.push(text(Role::User, &format!("follow-up {t}")));
    }
    messages
}

async fn send(f: &Fixture, messages: Vec<Message>) -> u16 {
    let envelope = RequestEnvelope {
        request_id: RequestId::new(),
        client_id: ClientId("omp".into()),
        tenant_id: TenantId("default".into()),
        session_key: None,
        api_type: ApiType::AnthropicMessages,
        model_requested: MODEL.into(),
        deadline: None,
        mode_pack_override: None,
        compression_override: None,
        cache_bypass: false,
        include_think_tags: false,
        client_ip: None,
    };
    let op = Operation::Conversation(ConversationRequest {
        model: MODEL.into(),
        messages,
        max_tokens: Some(10),
        system: Some("you are a coding agent".into()),
        ..Default::default()
    });
    run_conversation_pipeline(Arc::clone(&f.pipeline), PipelineCtx::new(envelope), op)
        .await
        .status()
        .as_u16()
}

fn calls(f: &Fixture) -> Vec<usize> {
    f.upstreams.iter().map(FakeUpstream::call_count).collect()
}

/// Plausible wrong impl defeated: balancing per request. The strategy alternates
/// connections on every call, so a gateway that re-routes each turn splits one
/// conversation across both accounts and the prompt cache never hits.
#[tokio::test]
async fn turns_of_one_conversation_stay_on_one_connection() {
    let f = fixture().await;
    for turn in 0..4 {
        assert_eq!(
            send(&f, conversation("refactor the parser", turn)).await,
            200
        );
    }
    let calls = calls(&f);
    assert!(
        calls.contains(&4) && calls.contains(&0),
        "all four turns must land on one connection, got {calls:?}"
    );
}

/// Plausible wrong impl defeated: pinning every client to one connection. A
/// second conversation is a new first request and must be routed afresh, so load
/// still spreads over the accounts.
#[tokio::test]
async fn distinct_conversations_spread_across_connections() {
    let f = fixture().await;
    for turn in 0..2 {
        assert_eq!(
            send(&f, conversation("refactor the parser", turn)).await,
            200
        );
        assert_eq!(
            send(&f, conversation("write the changelog", turn)).await,
            200
        );
    }
    assert_eq!(
        calls(&f),
        vec![2, 2],
        "each conversation keeps its own connection and both accounts are used"
    );
}

/// Plausible wrong impl defeated: the pin overrides eligibility. A pinned account
/// in cooldown (429) must be dropped for this turn, not retried into a failure.
#[tokio::test]
async fn pinned_connection_in_cooldown_is_replaced() {
    let f = fixture().await;
    assert_eq!(send(&f, conversation("refactor the parser", 0)).await, 200);
    let pinned = calls(&f)
        .iter()
        .position(|&c| c == 1)
        .expect("first turn reached one upstream");

    let conn = f.pipeline.catalog.get(&f.ids[pinned]).unwrap();
    conn.write().await.state = ConnectionState::Cooldown {
        until: chrono::Utc::now() + chrono::Duration::seconds(300),
        failure_count: 1,
    };

    assert_eq!(send(&f, conversation("refactor the parser", 1)).await, 200);
    let other = 1 - pinned;
    assert_eq!(
        (
            f.upstreams[pinned].call_count(),
            f.upstreams[other].call_count()
        ),
        (1, 1),
        "second turn must go to the healthy connection"
    );
}
