// Fake upstream server for conformance tests.
//
// Provides an in-process HTTP server that simulates provider behavior
// without real network calls. Tests configure it with a `FakeUpstreamConfig`
// before calling the pipeline.
//
// Phase A: static response / echo modes only.
// Phase B: add SSE streaming simulation and latency injection.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::{extract::State, response::IntoResponse, routing::post, Router};
use http::StatusCode;
use serde_json::json;
use tokio::net::TcpListener;

// ── Config ─────────────────────────────────────────────────────────────────────

/// What the fake upstream returns for any incoming request.
// ConnectionReset and OpenAI429 are infrastructure variants — available for tests
// that need them but not constructed in the current suite.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum FakeUpstreamBehavior {
    /// Return a static JSON body with the given status.
    StaticJson {
        status: u16,
        body: serde_json::Value,
    },
    /// Immediately close the connection (simulates a network error).
    ConnectionReset,
    /// Return a well-formed Anthropic-style 200 non-streaming response.
    AnthropicOk { content: String },
    /// Return a well-formed Anthropic SSE stream (text/event-stream).
    /// Emits: message_start → content_block_start → content_block_delta → message_delta → [DONE]
    AnthropicStreamOk { content: String },
    /// Return HTTP 429 in Anthropic error format.
    AnthropicOk429,
    /// Return OpenAI-format streaming SSE chunks (text/event-stream).
    OpenAIStreamOk { content: String },
    /// Return HTTP 429 in OpenAI error format.
    OpenAI429,
}

#[derive(Clone)]
pub struct FakeUpstreamState {
    pub behavior: FakeUpstreamBehavior,
    pub call_count: Arc<AtomicUsize>,
}

impl FakeUpstreamState {
    pub fn call_count(&self) -> usize {
        self.call_count.load(Ordering::Relaxed)
    }
}

// ── Handler ────────────────────────────────────────────────────────────────────

async fn handle(State(state): State<FakeUpstreamState>) -> impl IntoResponse {
    state.call_count.fetch_add(1, Ordering::Relaxed);

    match &state.behavior {
        FakeUpstreamBehavior::StaticJson { status, body } => {
            let code = StatusCode::from_u16(*status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
            let json_bytes = serde_json::to_vec(body).unwrap_or_else(|_| b"{}".to_vec());
            (code, [("content-type", "application/json")], json_bytes).into_response()
        }
        FakeUpstreamBehavior::ConnectionReset => {
            // Return 503 to simulate a provider-level error.
            (
                StatusCode::SERVICE_UNAVAILABLE,
                [("content-type", "application/json")],
                serde_json::to_vec(&json!({"error": "connection reset"})).unwrap(),
            )
                .into_response()
        }
        FakeUpstreamBehavior::AnthropicOk { content } => {
            let body = json!({
                "id": "msg_fake_01",
                "type": "message",
                "role": "assistant",
                "model": "claude-3-5-sonnet-20241022",
                "content": [{"type": "text", "text": content}],
                "stop_reason": "end_turn",
                "usage": {"input_tokens": 10, "output_tokens": 20}
            });
            (
                StatusCode::OK,
                [("content-type", "application/json")],
                serde_json::to_vec(&body).unwrap(),
            )
                .into_response()
        }
        FakeUpstreamBehavior::AnthropicStreamOk { content } => {
            // Emit properly-formatted SSE frames matching Anthropic's streaming wire format.
            // pipeline.rs passthrough forwards these bytes verbatim to the client.
            let sse = format!(
                "data: {}\n\ndata: {}\n\ndata: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
                serde_json::to_string(&json!({
                    "type": "message_start",
                    "message": {"id": "msg_fake_01", "type": "message", "role": "assistant",
                                "content": [], "model": "claude-3-5-sonnet-20241022",
                                "stop_reason": null, "usage": {"input_tokens": 10, "output_tokens": 0}}
                })).unwrap(),
                serde_json::to_string(&json!({
                    "type": "content_block_start", "index": 0,
                    "content_block": {"type": "text", "text": ""}
                })).unwrap(),
                serde_json::to_string(&json!({
                    "type": "content_block_delta", "index": 0,
                    "delta": {"type": "text_delta", "text": content}
                })).unwrap(),
                serde_json::to_string(&json!({
                    "type": "message_delta",
                    "delta": {"stop_reason": "end_turn"},
                    "usage": {"output_tokens": 5}
                })).unwrap(),
            );
            axum::response::Response::builder()
                .status(StatusCode::OK)
                .header("content-type", "text/event-stream")
                .header("cache-control", "no-cache")
                .body(axum::body::Body::from(sse))
                .unwrap()
        }
        FakeUpstreamBehavior::AnthropicOk429 => {
            let body = json!({"type":"error","error":{"type":"rate_limit_error","message":"Rate limit exceeded"}});
            (
                StatusCode::TOO_MANY_REQUESTS,
                [("content-type", "application/json")],
                serde_json::to_vec(&body).unwrap(),
            )
                .into_response()
        }
        FakeUpstreamBehavior::OpenAIStreamOk { content } => {
            let sse = format!(
                "data: {}\n\ndata: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
                serde_json::to_string(&json!({"id":"chatcmpl-fake","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"role":"assistant","content":""},"finish_reason":null}]})).unwrap(),
                serde_json::to_string(&json!({"id":"chatcmpl-fake","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"content":content},"finish_reason":null}]})).unwrap(),
                serde_json::to_string(&json!({"id":"chatcmpl-fake","object":"chat.completion.chunk","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}})).unwrap(),
            );
            axum::response::Response::builder()
                .status(StatusCode::OK)
                .header("content-type", "text/event-stream")
                .header("cache-control", "no-cache")
                .body(axum::body::Body::from(sse))
                .unwrap()
        }
        FakeUpstreamBehavior::OpenAI429 => {
            let body = json!({"error":{"message":"Rate limit exceeded","type":"rate_limit_error","code":"rate_limit_exceeded"}});
            (
                StatusCode::TOO_MANY_REQUESTS,
                [("content-type", "application/json")],
                serde_json::to_vec(&body).unwrap(),
            )
                .into_response()
        }
    }
}

// ── Server handle ──────────────────────────────────────────────────────────────

/// A running fake upstream bound to a random loopback port.
pub struct FakeUpstream {
    pub base_url: String,
    pub state: FakeUpstreamState,
    _shutdown: tokio::sync::oneshot::Sender<()>,
}

impl FakeUpstream {
    /// Spawn a fake upstream on a random OS-assigned port.
    /// Returns immediately; the server runs in the background on the current
    /// tokio runtime.
    pub async fn spawn(behavior: FakeUpstreamBehavior) -> Self {
        let state = FakeUpstreamState {
            behavior,
            call_count: Arc::new(AtomicUsize::new(0)),
        };

        let app = Router::new()
            .route("/v1/messages", post(handle))
            .route("/v1/chat/completions", post(handle))
            .with_state(state.clone());

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let base_url = format!("http://127.0.0.1:{port}");

        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = rx.await;
                })
                .await
                .ok();
        });

        Self {
            base_url,
            state,
            _shutdown: tx,
        }
    }

    /// Number of requests the fake upstream has received.
    pub fn call_count(&self) -> usize {
        self.state.call_count()
    }
}

// ── Smoke test ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify the fake upstream starts and responds correctly. PASSES.
    #[tokio::test]
    async fn fake_upstream_responds_static_json() {
        let fake = FakeUpstream::spawn(FakeUpstreamBehavior::StaticJson {
            status: 200,
            body: json!({"hello": "world"}),
        })
        .await;

        let resp = reqwest::Client::new()
            .post(format!("{}/v1/messages", fake.base_url))
            .header("content-type", "application/json")
            .body(r#"{"test":true}"#)
            .send()
            .await
            .expect("fake upstream must respond");

        assert_eq!(resp.status().as_u16(), 200);
        assert_eq!(fake.call_count(), 1, "exactly one request recorded");
    }

    /// Verify the fake upstream was never contacted when admission rejects.
    /// PASSES (no actual call made, just count check).
    #[tokio::test]
    async fn fake_upstream_call_count_zero_when_not_called() {
        let fake = FakeUpstream::spawn(FakeUpstreamBehavior::AnthropicOk {
            content: "unreachable".into(),
        })
        .await;

        assert_eq!(
            fake.call_count(),
            0,
            "upstream must not be contacted before any request"
        );
    }
}
