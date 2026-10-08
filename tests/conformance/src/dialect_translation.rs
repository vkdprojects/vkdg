// Dialect translation through the whole pipeline: a client speaking one dialect
// (Anthropic Messages or OpenAI Chat Completions) over a provider speaking the other,
// stream and non-stream, against a fake upstream serving hand-written fixtures.
//
// Expected values come from the Anthropic and OpenAI wire references, never from the
// translators themselves. Byte-level and every-split coverage of the decoders and
// encoders lives next to them (vkdg-provider-sdk `dialect`, vkdg-operations); this
// file proves they are wired into the pipeline and that same-dialect traffic is untouched.

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
use vkdg_operations::{
    CapabilitySet, ConversationRequest, Message, MessageContent, Operation, Role,
};
use vkdg_provider_anthropic::AnthropicAdapter;
use vkdg_provider_openai::OpenAIAdapter;
use vkdg_provider_sdk::ProviderRegistry;
use vkdg_routing::{PluginHooks, RouteConfig, RouteId, Router, StrategyKind};

use crate::fake_upstream::{FakeUpstream, FakeUpstreamBehavior};

const ANTHROPIC_CLIENT: ApiType = ApiType::AnthropicMessages;
const OPENAI_CLIENT: ApiType = ApiType::OpenAiChatCompletions;

// ── Harness ───────────────────────────────────────────────────────────────────

/// Which dialect the fake upstream (and so the connection's adapter) speaks.
#[derive(Clone, Copy)]
enum Upstream {
    Anthropic,
    OpenAi,
}

struct Reply {
    status: u16,
    content_type: String,
    body: String,
    upstream_calls: usize,
}

fn pipeline(base_url: String, upstream: Upstream) -> Arc<PipelineState> {
    std::env::set_var("VKDG_SMOKE_KEY", "test-token");
    let id = ConnectionId("translate".into());
    let provider = match upstream {
        Upstream::Anthropic => ProviderKind::AnthropicCompat { base_url },
        Upstream::OpenAi => ProviderKind::Custom { base_url },
    };
    let config = ConnectionConfig {
        id: id.clone(),
        provider,
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
        id: RouteId("translate".into()),
        match_models: vec!["claude-*".into()],
        strategy: StrategyKind::RoundRobin,
        targets: vec![id],
        plugin_hooks: PluginHooks::default(),
    };
    let mut registry = ProviderRegistry::empty();
    registry.register(Arc::new(AnthropicAdapter));
    registry.register(Arc::new(OpenAIAdapter));
    Arc::new(PipelineState::minimal(
        Arc::new(AdmissionGuard::new(10)),
        Arc::new(Router::new(vec![route])),
        Arc::new(ConnectionCatalog::new(vec![config])),
        Arc::new(CredentialManager::new()),
        Arc::new(HttpClient::new()),
        Arc::new(DecisionRecordExporter::new()),
        Arc::new(registry),
    ))
}

async fn call(
    behavior: FakeUpstreamBehavior,
    upstream: Upstream,
    client: ApiType,
    stream: bool,
) -> Reply {
    let fake = FakeUpstream::spawn(behavior).await;
    let envelope = RequestEnvelope {
        request_id: RequestId::new(),
        client_id: ClientId("translate".into()),
        tenant_id: TenantId("default".into()),
        session_key: None,
        api_type: client,
        model_requested: "claude-3-5-haiku-20241022".into(),
        deadline: None,
        mode_pack_override: None,
        compression_override: None,
        cache_bypass: true,
        include_think_tags: false,
        client_ip: None,
    };
    let op = Operation::Conversation(ConversationRequest {
        model: "claude-3-5-haiku-20241022".into(),
        messages: vec![Message {
            role: Role::User,
            content: MessageContent::Text("ping".into()),
        }],
        max_tokens: Some(64),
        stream,
        ..Default::default()
    });
    let response = run_conversation_pipeline(
        pipeline(fake.base_url.clone(), upstream),
        PipelineCtx::new(envelope),
        op,
    )
    .await;
    let status = response.status().as_u16();
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let body = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    Reply {
        status,
        content_type,
        body: String::from_utf8_lossy(&body).into_owned(),
        upstream_calls: fake.call_count(),
    }
}

/// `(event name, data)` of every SSE frame in `body`.
fn frames(body: &str) -> Vec<(Option<String>, String)> {
    body.split("\n\n")
        .filter(|raw| !raw.trim().is_empty())
        .map(|raw| {
            let mut event = None;
            let mut data = String::new();
            for line in raw.lines() {
                if let Some(v) = line.strip_prefix("event: ") {
                    event = Some(v.to_owned());
                } else if let Some(v) = line.strip_prefix("data: ") {
                    data.push_str(v);
                }
            }
            (event, data)
        })
        .collect()
}

/// The JSON data of every frame except `[DONE]`.
fn chunks(body: &str) -> Vec<Value> {
    frames(body)
        .into_iter()
        .filter(|(_, data)| data != "[DONE]")
        .map(|(_, data)| serde_json::from_str(&data).unwrap_or_else(|e| panic!("{e}: {data}")))
        .collect()
}

fn sse(frames: &[(&str, Value)]) -> Vec<u8> {
    use std::fmt::Write as _;
    let mut out = String::new();
    for (event, data) in frames {
        let _ = write!(out, "event: {event}\ndata: {data}\n\n");
    }
    out.into_bytes()
}

fn raw_sse(body: Vec<u8>) -> FakeUpstreamBehavior {
    FakeUpstreamBehavior::RawStream {
        content_type: "text/event-stream".into(),
        body,
    }
}

// ── Fixtures ──────────────────────────────────────────────────────────────────

fn anthropic_stream() -> Vec<u8> {
    sse(&[
        (
            "message_start",
            json!({"type":"message_start","message":{"id":"msg_1","type":"message","role":"assistant",
                "model":"claude-3-5-haiku-20241022","content":[],"stop_reason":null,"stop_sequence":null,
                "usage":{"input_tokens":25,"cache_read_input_tokens":10,"output_tokens":1}}}),
        ),
        (
            "content_block_start",
            json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}),
        ),
        (
            "content_block_delta",
            json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"hm"}}),
        ),
        (
            "content_block_stop",
            json!({"type":"content_block_stop","index":0}),
        ),
        (
            "content_block_start",
            json!({"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}),
        ),
        (
            "content_block_delta",
            json!({"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"Hel"}}),
        ),
        (
            "content_block_delta",
            json!({"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"lo"}}),
        ),
        (
            "content_block_stop",
            json!({"type":"content_block_stop","index":1}),
        ),
        (
            "content_block_start",
            json!({"type":"content_block_start","index":2,"content_block":
                {"type":"tool_use","id":"toolu_1","name":"get_weather","input":{}}}),
        ),
        (
            "content_block_delta",
            json!({"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{\"city\":"}}),
        ),
        (
            "content_block_delta",
            json!({"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"\"Paris\"}"}}),
        ),
        (
            "content_block_stop",
            json!({"type":"content_block_stop","index":2}),
        ),
        (
            "message_delta",
            json!({"type":"message_delta","delta":{"stop_reason":"tool_use","stop_sequence":null},
                "usage":{"output_tokens":40}}),
        ),
        ("message_stop", json!({"type":"message_stop"})),
    ])
}

fn anthropic_message() -> Value {
    json!({"id":"msg_1","type":"message","role":"assistant","model":"claude-3-5-haiku-20241022",
        "content":[
            {"type":"thinking","thinking":"hm","signature":"SIG"},
            {"type":"text","text":"Hello"},
            {"type":"tool_use","id":"toolu_1","name":"get_weather","input":{"city":"Paris"}}],
        "stop_reason":"tool_use","stop_sequence":null,
        "usage":{"input_tokens":25,"cache_read_input_tokens":10,"output_tokens":40}})
}

fn openai_completion() -> Value {
    json!({"id":"chatcmpl-1","object":"chat.completion","created":1,"model":"gpt-4o",
        "choices":[{"index":0,"message":{"role":"assistant","content":"Hello",
            "reasoning_content":"hm",
            "tool_calls":[{"id":"call_1","type":"function",
                "function":{"name":"get_weather","arguments":"{\"city\":\"Paris\"}"}}]},
            "finish_reason":"tool_calls"}],
        "usage":{"prompt_tokens":35,"completion_tokens":40,"total_tokens":75,
                 "prompt_tokens_details":{"cached_tokens":10}}})
}

// ── Anthropic upstream, OpenAI client (the reported bug) ──────────────────────

// Refutes: an OpenAI client receiving the Anthropic body untranslated (no `choices`),
// cached tokens missing from prompt_tokens, reasoning in `content`, or arguments re-serialized.
#[tokio::test]
async fn openai_client_over_anthropic_upstream_gets_a_chat_completion() {
    let reply = call(
        FakeUpstreamBehavior::StaticJson {
            status: 200,
            body: anthropic_message(),
        },
        Upstream::Anthropic,
        OPENAI_CLIENT,
        false,
    )
    .await;
    assert_eq!(reply.status, 200, "{}", reply.body);
    assert!(reply.content_type.contains("application/json"));
    let body: Value = serde_json::from_str(&reply.body).unwrap();
    assert_eq!(body["object"], "chat.completion", "{body}");
    let message = &body["choices"][0]["message"];
    assert_eq!(message["role"], "assistant");
    assert_eq!(message["content"], "Hello");
    assert_eq!(message["reasoning_content"], "hm");
    assert_eq!(message["tool_calls"][0]["id"], "toolu_1");
    assert_eq!(message["tool_calls"][0]["function"]["name"], "get_weather");
    assert_eq!(
        message["tool_calls"][0]["function"]["arguments"],
        "{\"city\":\"Paris\"}"
    );
    assert_eq!(body["choices"][0]["finish_reason"], "tool_calls");
    assert_eq!(
        body["usage"]["prompt_tokens"], 35,
        "input 25 + cache read 10"
    );
    assert_eq!(body["usage"]["completion_tokens"], 40);
    assert_eq!(body["usage"]["total_tokens"], 75);
    assert_eq!(body["usage"]["prompt_tokens_details"]["cached_tokens"], 10);
    assert!(
        body.get("content").is_none(),
        "Anthropic fields leaked: {body}"
    );
    assert_eq!(reply.upstream_calls, 1);
}

// Refutes: chunks that are not OpenAI's (event lines, Anthropic payloads), a missing role
// chunk, tool-call fragments repeating id/name, finish_reason/usage out of order, or no [DONE].
#[tokio::test]
async fn openai_client_over_anthropic_stream_gets_openai_chunks() {
    let reply = call(
        raw_sse(anthropic_stream()),
        Upstream::Anthropic,
        OPENAI_CLIENT,
        true,
    )
    .await;
    assert_eq!(reply.status, 200, "{}", reply.body);
    assert!(reply.content_type.contains("text/event-stream"));
    assert!(
        frames(&reply.body).iter().all(|(event, _)| event.is_none()),
        "OpenAI SSE has no `event:` lines: {}",
        reply.body
    );
    assert_eq!(frames(&reply.body).last().unwrap().1, "[DONE]");

    let all = chunks(&reply.body);
    let delta = |i: usize| &all[i]["choices"][0]["delta"];
    assert_eq!(delta(0), &json!({"role":"assistant","content":""}));
    assert_eq!(delta(1), &json!({"reasoning_content":"hm"}));
    assert_eq!(delta(2), &json!({"content":"Hel"}));
    assert_eq!(delta(3), &json!({"content":"lo"}));
    assert_eq!(
        delta(4),
        &json!({"tool_calls":[{"index":0,"id":"toolu_1","type":"function",
            "function":{"name":"get_weather","arguments":""}}]})
    );
    assert_eq!(
        delta(5),
        &json!({"tool_calls":[{"index":0,"function":{"arguments":"{\"city\":"}}]})
    );
    assert_eq!(
        delta(6),
        &json!({"tool_calls":[{"index":0,"function":{"arguments":"\"Paris\"}"}}]})
    );
    assert_eq!(all[7]["choices"][0]["finish_reason"], "tool_calls");
    let usage = &all[8];
    assert_eq!(usage["choices"], json!([]));
    assert_eq!(usage["usage"]["prompt_tokens"], 35);
    assert_eq!(usage["usage"]["completion_tokens"], 40);
    assert_eq!(usage["usage"]["prompt_tokens_details"]["cached_tokens"], 10);
    assert_eq!(all.len(), 9, "{}", reply.body);
}

// ── OpenAI upstream, Anthropic client ─────────────────────────────────────────

// Refutes: an Anthropic client receiving `choices`, tool arguments left as a string instead
// of an input object, cached tokens counted in input_tokens, or a wrong stop_reason.
#[tokio::test]
async fn anthropic_client_over_openai_upstream_gets_a_message() {
    let reply = call(
        FakeUpstreamBehavior::StaticJson {
            status: 200,
            body: openai_completion(),
        },
        Upstream::OpenAi,
        ANTHROPIC_CLIENT,
        false,
    )
    .await;
    assert_eq!(reply.status, 200, "{}", reply.body);
    let body: Value = serde_json::from_str(&reply.body).unwrap();
    assert_eq!(body["type"], "message", "{body}");
    assert_eq!(body["role"], "assistant");
    assert_eq!(
        body["content"],
        json!([
            {"type":"thinking","thinking":"hm","signature":""},
            {"type":"text","text":"Hello"},
            {"type":"tool_use","id":"call_1","name":"get_weather","input":{"city":"Paris"}},
        ])
    );
    assert_eq!(body["stop_reason"], "tool_use");
    assert_eq!(
        body["usage"]["input_tokens"], 25,
        "prompt 35 minus cached 10"
    );
    assert_eq!(body["usage"]["cache_read_input_tokens"], 10);
    assert_eq!(body["usage"]["output_tokens"], 40);
    assert!(
        body.get("choices").is_none(),
        "OpenAI fields leaked: {body}"
    );
}

// ── Same dialect: untouched ───────────────────────────────────────────────────

// Refutes: parsing and re-serializing a body whose dialect already matches (key order,
// number formatting and unknown fields must survive byte for byte).
#[tokio::test]
async fn same_dialect_non_stream_body_is_byte_identical() {
    let fixture = r#"{"id":"msg_9","type":"message","unknown_future_field":{"b":1,"a":[1.0,2]},"role":"assistant","content":[{"type":"text","text":"x"}],"stop_reason":"end_turn","usage":{"input_tokens":1,"output_tokens":1}}"#;
    let upstream = FakeUpstreamBehavior::StaticJson {
        status: 200,
        body: serde_json::from_str(fixture).unwrap(),
    };
    let reply = call(upstream, Upstream::Anthropic, ANTHROPIC_CLIENT, false).await;
    let expected = serde_json::to_string(&serde_json::from_str::<Value>(fixture).unwrap()).unwrap();
    assert_eq!(reply.body, expected);
}

// Refutes: the relay touching an Anthropic stream that an Anthropic client asked for.
#[tokio::test]
async fn same_dialect_stream_is_byte_identical() {
    let fixture = anthropic_stream();
    let reply = call(
        raw_sse(fixture.clone()),
        Upstream::Anthropic,
        ANTHROPIC_CLIENT,
        true,
    )
    .await;
    assert_eq!(reply.body.as_bytes(), fixture.as_slice());
}

// ── Failures ──────────────────────────────────────────────────────────────────

// Refutes: a foreign or garbage 2xx body reaching the client as if it were a valid answer,
// or the failure being rendered in the wrong dialect.
#[tokio::test]
async fn unreadable_upstream_body_is_a_502_in_the_clients_dialect() {
    for (client, upstream, body) in [
        (
            OPENAI_CLIENT,
            Upstream::Anthropic,
            json!({"unexpected": "shape"}),
        ),
        (
            ANTHROPIC_CLIENT,
            Upstream::OpenAi,
            json!({"unexpected": "shape"}),
        ),
    ] {
        let reply = call(
            FakeUpstreamBehavior::StaticJson { status: 200, body },
            upstream,
            client.clone(),
            false,
        )
        .await;
        assert_eq!(reply.status, 502, "{}", reply.body);
        let error: Value = serde_json::from_str(&reply.body).unwrap();
        match client {
            ApiType::OpenAiChatCompletions => {
                assert!(error.get("type").is_none(), "{error}");
                assert_eq!(error["error"]["type"], "server_error", "{error}");
            }
            _ => assert_eq!(error["type"], "error", "{error}"),
        }
    }
}

// Refutes: an upstream error after bytes were committed being retried, swallowed, or ending
// the stream without the client dialect's terminator (OpenAI SDKs wait for [DONE]).
#[tokio::test]
async fn upstream_error_mid_stream_ends_an_openai_client_with_an_error_object_and_done() {
    let body = sse(&[
        (
            "message_start",
            json!({"type":"message_start","message":{"id":"m","usage":{"input_tokens":1,"output_tokens":1}}}),
        ),
        (
            "content_block_start",
            json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
        ),
        (
            "content_block_delta",
            json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"partial"}}),
        ),
        (
            "error",
            json!({"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}),
        ),
        (
            "content_block_delta",
            json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"late"}}),
        ),
    ]);
    let reply = call(raw_sse(body), Upstream::Anthropic, OPENAI_CLIENT, true).await;
    assert_eq!(reply.status, 200, "the response was already committed");
    let all = frames(&reply.body);
    assert_eq!(all.last().unwrap().1, "[DONE]");
    let error: Value = serde_json::from_str(&all[all.len() - 2].1).unwrap();
    assert_eq!(error["error"]["type"], "rate_limit_error", "{error}");
    assert!(reply.body.contains("partial"));
    assert!(
        !reply.body.contains("late"),
        "content after the error: {}",
        reply.body
    );
    assert_eq!(reply.upstream_calls, 1, "no retry after commit");
}
