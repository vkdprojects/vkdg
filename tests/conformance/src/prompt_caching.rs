// Prompt caching through the whole pipeline: what the client marks reaches the
// Anthropic-format upstream verbatim, and a client that marks nothing still gets
// breakpoints. Only the request body is observed here; whether the upstream
// really caches shows up as `cache_read_input_tokens > 0` in a live response and
// cannot be proved against a fake.

use std::sync::Arc;

use serde_json::{json, Value};
use vkdg_connections::{
    AuthKind, ConnectionCatalog, ConnectionConfig, CredentialManager, ProviderKind,
};
use vkdg_core::pipeline::PipelineCtx;
use vkdg_core::{ApiType, ClientId, ConnectionId, RequestEnvelope, RequestId, TenantId};
use vkdg_http::pipeline::run_conversation_pipeline;
use vkdg_http::upstream::HttpClient;
use vkdg_http::{AdmissionGuard, PipelineState};
use vkdg_observe::DecisionRecordExporter;
use vkdg_operations::Operation;
use vkdg_provider_anthropic::AnthropicAdapter;
use vkdg_provider_sdk::ProviderRegistry;
use vkdg_routing::{PluginHooks, RouteConfig, RouteId, Router, StrategyKind};

use crate::fake_upstream::{FakeUpstream, FakeUpstreamBehavior};

const MODEL: &str = "claude-3-5-haiku-20241022";

/// Sends `client_body` (an Anthropic Messages request, as a client writes it)
/// through ingress decoding and the pipeline; returns what the upstream received.
async fn upstream_body(client_body: Value) -> Value {
    std::env::set_var("VKDG_SMOKE_KEY", "test-token");
    let fake = FakeUpstream::spawn(FakeUpstreamBehavior::AnthropicOk {
        content: "ok".into(),
    })
    .await;
    let id = ConnectionId("acct".into());
    let config = ConnectionConfig {
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
        capabilities: Default::default(),
    };
    let route = RouteConfig {
        id: RouteId("claude".into()),
        match_models: vec!["claude-*".into()],
        strategy: StrategyKind::RoundRobin,
        targets: vec![id],
        plugin_hooks: PluginHooks::default(),
    };
    let mut registry = ProviderRegistry::empty();
    registry.register(Arc::new(AnthropicAdapter));
    let pipeline = Arc::new(PipelineState::minimal(
        Arc::new(AdmissionGuard::new(10)),
        Arc::new(Router::new(vec![route])),
        Arc::new(ConnectionCatalog::new(vec![config])),
        Arc::new(CredentialManager::new()),
        Arc::new(HttpClient::new()),
        Arc::new(DecisionRecordExporter::new()),
        Arc::new(registry),
    ));

    let bytes = serde_json::to_vec(&client_body).unwrap();
    let (_, op) = vkdg_ingress_anthropic::decode_request(&bytes).expect("client body decodes");
    assert!(matches!(op, Operation::Conversation(_)));
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
    let status = run_conversation_pipeline(pipeline, PipelineCtx::new(envelope), op)
        .await
        .status()
        .as_u16();
    assert_eq!(status, 200);
    let bodies = fake.bodies();
    assert_eq!(bodies.len(), 1, "one upstream call expected");
    bodies.into_iter().next().unwrap()
}

fn marker_count(body: &Value) -> usize {
    body.to_string().matches("\"cache_control\"").count()
}

/// Plausible wrong impl defeated: dropping the client's breakpoints (or their
/// `ttl`) anywhere between ingress and the upstream body, or adding the gateway's
/// own on top of a client that already placed some.
#[tokio::test]
async fn client_cache_control_reaches_the_upstream_with_its_ttl_and_nothing_is_added() {
    let body = upstream_body(json!({
        "model": MODEL,
        "max_tokens": 16,
        "system": [
            {"type": "text", "text": "stable rules"},
            {"type": "text", "text": "project notes",
             "cache_control": {"type": "ephemeral", "ttl": "1h"}}
        ],
        "tools": [
            {"name": "read", "input_schema": {"type": "object"}},
            {"name": "write", "input_schema": {"type": "object"}}
        ],
        "messages": [
            {"role": "user", "content": [
                {"type": "text", "text": "hello",
                 "cache_control": {"type": "ephemeral"}}
            ]},
            {"role": "assistant", "content": "hi"},
            {"role": "user", "content": "go on"}
        ]
    }))
    .await;

    assert_eq!(body["system"][0].get("cache_control"), None);
    assert_eq!(
        body["system"][1]["cache_control"],
        json!({"type": "ephemeral", "ttl": "1h"})
    );
    assert_eq!(
        body["messages"][0]["content"][0]["cache_control"],
        json!({"type": "ephemeral"})
    );
    assert_eq!(marker_count(&body), 2, "no breakpoint added: {body}");
}

/// Plausible wrong impl defeated: never placing breakpoints for clients that do
/// not know the feature, or placing them on the wrong blocks.
#[tokio::test]
async fn a_request_without_markers_gets_default_breakpoints() {
    let body = upstream_body(json!({
        "model": MODEL,
        "max_tokens": 16,
        "system": "you are a coding agent",
        "tools": [
            {"name": "read", "input_schema": {"type": "object"}},
            {"name": "write", "input_schema": {"type": "object"}}
        ],
        "messages": [{"role": "user", "content": "hello"}]
    }))
    .await;

    let ephemeral = json!({"type": "ephemeral"});
    assert_eq!(body["tools"][0].get("cache_control"), None);
    assert_eq!(body["tools"][1]["cache_control"], ephemeral);
    assert_eq!(body["system"][0]["text"], "you are a coding agent");
    assert_eq!(body["system"][0]["cache_control"], ephemeral);
    assert_eq!(body["messages"][0]["content"][0]["text"], "hello");
    assert_eq!(
        body["messages"][0]["content"][0]["cache_control"],
        ephemeral
    );
    assert_eq!(marker_count(&body), 3);
}

/// Plausible wrong impl defeated: forwarding more than the four breakpoints
/// Anthropic allows, which the upstream answers with a 400.
#[tokio::test]
async fn more_than_four_markers_are_trimmed_to_the_last_four() {
    let marked =
        |t: &str| json!({"type": "text", "text": t, "cache_control": {"type": "ephemeral"}});
    let body = upstream_body(json!({
        "model": MODEL,
        "max_tokens": 16,
        "system": [marked("s1"), marked("s2")],
        "messages": [
            {"role": "user", "content": [marked("m1"), marked("m2"), marked("m3")]}
        ]
    }))
    .await;
    assert_eq!(marker_count(&body), 4, "{body}");
    assert_eq!(
        body["system"][0].get("cache_control"),
        None,
        "oldest dropped"
    );
    assert!(body["system"][1].get("cache_control").is_some());
    assert!(body["messages"][0]["content"][2]
        .get("cache_control")
        .is_some());
}
