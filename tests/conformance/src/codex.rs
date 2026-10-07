//! Refutes invalid assistant history and raw Responses SSE reaching Chat clients.
use std::fmt::Write;
use std::future::IntoFuture;
use std::sync::Arc;

use axum::{extract::State, response::IntoResponse, routing::post, Json};
use serde_json::{json, Value};
use vkdg_connections::{
    AuthKind, ConnectionCatalog, ConnectionConfig, CredentialManager, ProviderKind,
};
use vkdg_core::ConnectionId;
use vkdg_http::{upstream::HttpClient, AdmissionGuard, AppState, PipelineState, ServerConfig};
use vkdg_observe::DecisionRecordExporter;
use vkdg_operations::{CapabilitySet, Operation};
use vkdg_provider_codex::CodexAdapter;
use vkdg_provider_sdk::{
    ConversationStreamDecoder, Credential, PreparedRequest, ProviderAdapter, ProviderError,
    ProviderRegistry,
};
use vkdg_routing::{PluginHooks, RouteConfig, RouteId, Router, StrategyKind};

struct LoopbackCodex(String);
impl ProviderAdapter for LoopbackCodex {
    fn id(&self) -> &'static str {
        "codex"
    }
    fn display_name(&self) -> &'static str {
        "Loopback Codex"
    }
    fn prepare(
        &self,
        op: &Operation,
        config: &ConnectionConfig,
        credential: &Credential,
    ) -> Result<PreparedRequest, ProviderError> {
        let mut prepared = CodexAdapter::new().prepare(op, config, credential)?;
        prepared.url.clone_from(&self.0);
        Ok(prepared)
    }
    fn stream_decoder(&self) -> Option<Box<dyn ConversationStreamDecoder>> {
        CodexAdapter::new().stream_decoder()
    }
}

fn response_stream(events: &[Value]) -> String {
    let mut stream = String::new();
    for event in events {
        write!(
            stream,
            "event: {}\ndata: {event}\n\n",
            event["type"].as_str().expect("event type")
        )
        .expect("writing to a String");
    }
    stream
}

async fn strict_upstream(
    State(()): State<()>,
    Json(body): Json<Value>,
) -> axum::response::Response {
    let input = body["input"].as_array().expect("Responses input");
    // A permissive fake would mask the incident. Independently enforce upstream roles.
    for item in input {
        if item["type"] == "message" {
            let expected = if item["role"] == "assistant" {
                "output_text"
            } else {
                "input_text"
            };
            for part in item["content"].as_array().expect("message content") {
                if part["type"] != expected {
                    return (
                        http::StatusCode::BAD_REQUEST,
                        Json(json!({"error":"invalid history content type"})),
                    )
                        .into_response();
                }
            }
        }
    }
    assert!(
        input.len() >= 87,
        "history must survive ingress and conversion"
    );
    assert_eq!(input[84]["role"], "assistant");
    assert_eq!(input[84]["content"][0]["text"], "historical answer 84");
    let result = input
        .iter()
        .find(|item| item["type"] == "function_call_output");
    let events = if let Some(result) = result {
        assert_eq!(result["call_id"], "call_loopback");
        assert_eq!(result["output"], "tool-secret-7d2b");
        let call = input
            .iter()
            .find(|item| item["type"] == "function_call")
            .expect("replayed call");
        assert_eq!(call["call_id"], "call_loopback");
        assert_eq!(call["name"], "inspect");
        assert_eq!(
            serde_json::from_str::<Value>(call["arguments"].as_str().expect("arguments string"))
                .expect("arguments JSON"),
            json!({"path":"synthetic"})
        );
        assert!(
            input
                .iter()
                .any(|item| item["role"] == "assistant"
                    && item["content"][0]["text"] == "checking now"),
            "text beside replayed call must not disappear"
        );
        vec![
            json!({"type":"response.created","response":{"id":"resp_second"}}),
            json!({"type":"response.output_text.delta","output_index":0,"delta":"confirmed tool-secret-7d2b"}),
            json!({"type":"response.output_text.done","output_index":0,"text":"confirmed tool-secret-7d2b"}),
            json!({"type":"response.completed","response":{"status":"completed","usage":{"input_tokens":101,"output_tokens":7}}}),
        ]
    } else {
        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["tools"][0]["name"], "inspect");
        vec![
            json!({"type":"response.created","response":{"id":"resp_first"}}),
            json!({"type":"response.output_text.delta","output_index":0,"delta":"checking now"}),
            json!({"type":"response.output_item.added","output_index":1,"item":{"type":"function_call","id":"fc_loopback","call_id":"call_loopback","name":"inspect","arguments":""}}),
            json!({"type":"response.function_call_arguments.delta","output_index":1,"delta":"{\"path\":"}),
            json!({"type":"response.function_call_arguments.delta","output_index":1,"delta":"\"synthetic\"}"}),
            json!({"type":"response.function_call_arguments.done","output_index":1,"arguments":"{\"path\":\"synthetic\"}"}),
            json!({"type":"response.output_item.done","output_index":1,"item":{"type":"function_call","id":"fc_loopback","call_id":"call_loopback","name":"inspect","arguments":"{\"path\":\"synthetic\"}"}}),
            json!({"type":"response.completed","response":{"status":"completed","usage":{"input_tokens":100,"output_tokens":6}}}),
        ]
    };
    (
        [("content-type", "text/event-stream")],
        response_stream(&events),
    )
        .into_response()
}

fn consume_chat(stream: &str) -> (String, Value, String) {
    let mut text = String::new();
    let mut call = json!({"id":"","type":"function","function":{"name":"","arguments":""}});
    let mut finish = String::new();
    let mut done = false;
    for line in stream
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
    {
        if line == "[DONE]" {
            done = true;
            continue;
        }
        let chunk: Value = serde_json::from_str(line).expect("client chunk JSON");
        assert!(chunk.get("error").is_none(), "stream error: {chunk}");
        let choice = &chunk["choices"][0];
        if let Some(delta) = choice["delta"]["content"].as_str() {
            text.push_str(delta);
        }
        if let Some(calls) = choice["delta"]["tool_calls"].as_array() {
            for fragment in calls {
                for (target, source) in [
                    ("id", &fragment["id"]),
                    ("name", &fragment["function"]["name"]),
                    ("arguments", &fragment["function"]["arguments"]),
                ] {
                    if let Some(value) = source.as_str() {
                        let slot = if target == "id" {
                            &mut call[target]
                        } else {
                            &mut call["function"][target]
                        };
                        let mut accumulated = slot.as_str().expect("string field").to_owned();
                        accumulated.push_str(value);
                        *slot = json!(accumulated);
                    }
                }
            }
        }
        if let Some(reason) = choice["finish_reason"].as_str() {
            reason.clone_into(&mut finish);
        }
    }
    assert!(done, "Chat consumer must receive DONE");
    (text, call, finish)
}

#[tokio::test]
async fn smoke_codex_long_history_and_real_chat_tool_round_trip() {
    let upstream_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind upstream");
    let upstream_url = format!(
        "http://{}/responses",
        upstream_listener.local_addr().expect("upstream address")
    );
    let upstream_task = tokio::spawn(
        axum::serve(
            upstream_listener,
            axum::Router::new()
                .route("/responses", post(strict_upstream))
                .with_state(()),
        )
        .into_future(),
    );
    std::env::set_var("VKDG_CODEX_LOOPBACK_KEY", "synthetic-key");
    let id = ConnectionId("codex-loopback".into());
    let connection = ConnectionConfig {
        id: id.clone(),
        provider: ProviderKind::Plugin { id: "codex".into() },
        auth: AuthKind::ApiKey {
            env_var: "VKDG_CODEX_LOOPBACK_KEY".into(),
        },
        models: vec!["gpt-*".into()],
        max_concurrent: 1,
        weight: 1,
        tags: vec![],
        endpoint: None,
        capabilities: CapabilitySet::default(),
    };
    let mut providers = ProviderRegistry::empty();
    providers.register(Arc::new(LoopbackCodex(upstream_url)));
    let pipeline = Arc::new(PipelineState::minimal(
        Arc::new(AdmissionGuard::new(1)),
        Arc::new(Router::new(vec![RouteConfig {
            id: RouteId("codex-loopback".into()),
            match_models: vec!["gpt-*".into()],
            strategy: StrategyKind::RoundRobin,
            targets: vec![id],
            plugin_hooks: PluginHooks::default(),
        }])),
        Arc::new(ConnectionCatalog::new(vec![connection])),
        Arc::new(CredentialManager::new()),
        Arc::new(HttpClient::new()),
        Arc::new(DecisionRecordExporter::new()),
        Arc::new(providers),
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
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind gateway");
    let url = format!(
        "http://{}/v1/chat/completions",
        listener.local_addr().expect("gateway address")
    );
    let gateway_task = tokio::spawn(axum::serve(listener, gateway).into_future());
    let mut messages = Vec::new();
    for index in 0..86 {
        let assistant = index % 2 == 0;
        let text = if assistant {
            format!("historical answer {index}")
        } else {
            format!("historical question {index}")
        };
        let content = if index % 4 == 0 {
            json!([{ "type":"text", "text":text }])
        } else {
            json!(text)
        };
        messages
            .push(json!({"role":if assistant { "assistant" } else { "user" }, "content":content}));
    }
    messages.push(json!({"role":"user","content":"inspect the synthetic path"}));
    let client = reqwest::Client::new();
    let tools = json!([{ "type":"function", "function":{"name":"inspect","parameters":{"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}} }]);
    let first = client
        .post(&url)
        .json(&json!({"model":"gpt-6-luna:high","messages":messages,"tools":tools,"stream":true}))
        .send()
        .await
        .expect("first HTTP request");
    assert_eq!(first.status(), http::StatusCode::OK);
    let (text, call, finish) = consume_chat(&first.text().await.expect("first stream"));
    assert_eq!(text, "checking now");
    assert_eq!(finish, "tool_calls");
    assert_eq!(call["id"], "call_loopback");
    assert_eq!(call["function"]["name"], "inspect");
    assert_eq!(
        serde_json::from_str::<Value>(
            call["function"]["arguments"]
                .as_str()
                .expect("tool arguments")
        )
        .expect("arguments JSON"),
        json!({"path":"synthetic"})
    );
    messages.push(json!({"role":"assistant","content":text,"tool_calls":[call]}));
    messages
        .push(json!({"role":"tool","tool_call_id":"call_loopback","content":"tool-secret-7d2b"}));
    let second = client
        .post(&url)
        .json(&json!({"model":"gpt-6-luna:high","messages":messages,"tools":tools,"stream":true}))
        .send()
        .await
        .expect("second HTTP request");
    assert_eq!(second.status(), http::StatusCode::OK);
    let (answer, _, finish) = consume_chat(&second.text().await.expect("second stream"));
    assert_eq!(answer, "confirmed tool-secret-7d2b");
    assert_eq!(finish, "stop");
    println!("loopback HTTP: 87/89 history messages, assistant array/text, tool call+result replay, Chat deltas/finish/DONE valid");
    gateway_task.abort();
    upstream_task.abort();
}
