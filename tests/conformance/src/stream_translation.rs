//! Pipeline-level conformance for providers whose wire protocol is not SSE.
//!
//! A provider adapter may return a stream in its own binary protocol (Kiro sends
//! AWS EventStream). The pipeline must decode it to `ConversationEvent`s and
//! re-encode them in the dialect the client spoke. Kiro sends no explicit stop
//! event, so termination depends on `ConversationStreamDecoder::finish()` running
//! when upstream closes.
//!
//! Refutes: dropping `finish()` (stream ends with no terminal events), and
//! hand-rolled dialect encoders that emit an empty `message_start`, hardcode
//! block index 0, or silently drop tool calls.

use std::sync::Arc;

use aws_smithy_eventstream::frame::write_message_to;
use aws_smithy_types::event_stream::{Header, HeaderValue as AwsValue, Message as AwsMessage};
use serde_json::{json, Value};
use vkdg_connections::{
    AuthKind, ConnectionCatalog, ConnectionConfig, Credential, CredentialManager, ProviderKind,
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
use vkdg_provider_kiro::stream_decoder::KiroStreamDecoder;
use vkdg_provider_sdk::{
    ConversationStreamDecoder, PreparedRequest, ProviderAdapter, ProviderError, ProviderRegistry,
};
use vkdg_routing::{PluginHooks, RouteConfig, RouteId, Router, StrategyKind};

use crate::fake_upstream::{FakeUpstream, FakeUpstreamBehavior};

// ── Kiro EventStream fixture ──────────────────────────────────────────────────

fn frame(event_type: &'static str, payload: Value) -> Vec<u8> {
    let mut buf = Vec::new();
    write_message_to(
        &AwsMessage::new(payload.to_string())
            .add_header(Header::new(
                ":message-type",
                AwsValue::String("event".into()),
            ))
            .add_header(Header::new(
                ":event-type",
                AwsValue::String(event_type.into()),
            ))
            .add_header(Header::new(
                ":content-type",
                AwsValue::String("application/json".into()),
            )),
        &mut buf,
    )
    .expect("encode event stream frame");
    buf
}

/// Text, then one tool call — and no stop event, exactly as Kiro behaves.
fn kiro_stream_with_tool_call() -> Vec<u8> {
    [
        frame("assistantResponseEvent", json!({ "content": "Checking" })),
        frame(
            "toolUseEvent",
            json!({ "toolUseId": "tu_1", "name": "get_weather", "input": "" }),
        ),
        frame(
            "toolUseEvent",
            json!({ "toolUseId": "tu_1", "name": "get_weather", "input": "{\"city\":\"Paris\"}", "stop": true }),
        ),
    ]
    .concat()
}

// ── Test adapter: forwards to the fake upstream, decodes as Kiro ──────────────

struct KiroLikeAdapter {
    base_url: String,
}

impl ProviderAdapter for KiroLikeAdapter {
    fn id(&self) -> &str {
        "kiro-like"
    }

    fn display_name(&self) -> &str {
        "Kiro-like test provider"
    }

    fn prepare(
        &self,
        _operation: &Operation,
        _config: &ConnectionConfig,
        _credential: &Credential,
    ) -> Result<PreparedRequest, ProviderError> {
        Ok(PreparedRequest {
            url: format!("{}/v1/messages", self.base_url),
            headers: http::HeaderMap::new(),
            body: bytes::Bytes::new(),
            is_streaming: true,
        })
    }

    fn stream_decoder(&self) -> Option<Box<dyn ConversationStreamDecoder>> {
        Some(Box::new(KiroStreamDecoder::new()))
    }
}

fn make_pipeline(base_url: String) -> Arc<PipelineState> {
    let conn_id = ConnectionId("kiro-like".into());
    let config = ConnectionConfig {
        id: conn_id.clone(),
        provider: ProviderKind::Plugin {
            id: "kiro-like".into(),
        },
        auth: AuthKind::ApiKey {
            env_var: "VKDG_STREAM_XLATE_KEY".into(),
        },
        models: vec!["claude-*".into()],
        max_concurrent: 10,
        weight: 1,
        tags: vec![],
        capabilities: CapabilitySet::default(),
    };
    let route = RouteConfig {
        id: RouteId("kiro-like".into()),
        match_models: vec!["claude-*".into()],
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
        provider_registry: {
            let mut r = ProviderRegistry::empty();
            r.register(Arc::new(KiroLikeAdapter { base_url }));
            Arc::new(r)
        },
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
        relay_enabled: false,
        request_log: None,
    })
}

async fn client_stream(api_type: ApiType) -> String {
    std::env::set_var("VKDG_STREAM_XLATE_KEY", "test-token");

    let fake = FakeUpstream::spawn(FakeUpstreamBehavior::RawStream {
        content_type: "application/vnd.amazon.eventstream".into(),
        body: kiro_stream_with_tool_call(),
    })
    .await;

    let pipeline = make_pipeline(fake.base_url.clone());
    let envelope = RequestEnvelope {
        request_id: RequestId::new(),
        client_id: ClientId("xlate-test".into()),
        tenant_id: TenantId("default".into()),
        session_key: None,
        api_type,
        model_requested: "claude-sonnet-4.5".into(),
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
            content: MessageContent::Text("weather in Paris?".into()),
        }],
        tools: vec![],
        max_tokens: Some(64),
        temperature: None,
        stream: true,
        system: None,
        required_capabilities: CapabilitySet::default(),
    });

    let response = run_conversation_pipeline(pipeline, PipelineCtx::new(envelope), op).await;
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("collect body");
    String::from_utf8(bytes.to_vec()).expect("utf-8 body")
}

// ── Anthropic dialect ─────────────────────────────────────────────────────────

/// Refutes: `message_start` sent as `data: {}` (no message object), which every
/// Anthropic client rejects.
#[tokio::test]
async fn kiro_stream_is_translated_to_valid_anthropic_messages() {
    let body = client_stream(ApiType::AnthropicMessages).await;

    let message_start = body
        .split("\n\n")
        .find(|f| f.contains("event: message_start"))
        .unwrap_or_else(|| panic!("no message_start frame in:\n{body}"));
    let payload: Value = serde_json::from_str(
        message_start
            .split("data: ")
            .nth(1)
            .expect("message_start data line"),
    )
    .expect("message_start json");
    assert_eq!(payload["message"]["role"], "assistant", "{payload}");
    assert_eq!(payload["message"]["type"], "message", "{payload}");
    assert_eq!(
        payload["message"]["model"], "claude-sonnet-4.5",
        "message_start must echo the requested model: {payload}"
    );

    // Text and tool_use are separate blocks, so indices must differ.
    assert!(
        body.contains(r#""content_block":{"text":"","type":"text"}"#)
            || body.contains(r#""type":"text","text":"""#),
        "missing text content_block_start in:\n{body}"
    );
    assert!(
        body.contains(r#""name":"get_weather""#),
        "tool call must survive translation:\n{body}"
    );
    assert!(
        body.contains(r#""index":1"#),
        "second content block must use index 1, not a hardcoded 0:\n{body}"
    );
    assert!(
        body.contains("input_json_delta"),
        "tool input must stream as input_json_delta:\n{body}"
    );

    // Kiro sends no stop event: termination comes from decoder.finish().
    let stop_pos = body
        .find("event: message_stop")
        .unwrap_or_else(|| panic!("stream must terminate with message_stop:\n{body}"));
    let delta_pos = body
        .find("event: message_delta")
        .unwrap_or_else(|| panic!("stream must send message_delta:\n{body}"));
    assert!(
        delta_pos < stop_pos,
        "message_delta must precede message_stop:\n{body}"
    );
    assert!(
        body.contains(r#""stop_reason":"tool_use""#),
        "a stream ending in a tool call must report stop_reason tool_use:\n{body}"
    );
    assert!(
        body[..stop_pos].contains("content_block_stop"),
        "open blocks must close before message_stop:\n{body}"
    );
}

// ── OpenAI dialect ────────────────────────────────────────────────────────────

/// Refutes: the same events encoded as Anthropic frames for an OpenAI client, or
/// a stream that never sends `[DONE]`.
#[tokio::test]
async fn kiro_stream_is_translated_to_valid_openai_chunks() {
    let body = client_stream(ApiType::OpenAiChatCompletions).await;

    assert!(
        !body.contains("event: message_start"),
        "OpenAI clients must not receive Anthropic frames:\n{body}"
    );
    assert!(
        body.contains(r#""object":"chat.completion.chunk""#),
        "missing chat.completion.chunk payloads:\n{body}"
    );
    assert!(
        body.contains(r#""name":"get_weather""#),
        "tool call must survive translation:\n{body}"
    );
    assert!(
        body.contains(r#""finish_reason":"tool_calls""#),
        "a stream ending in a tool call must finish with tool_calls:\n{body}"
    );
    assert!(
        body.trim_end().ends_with("data: [DONE]"),
        "OpenAI streams must terminate with [DONE]:\n{body}"
    );
}
