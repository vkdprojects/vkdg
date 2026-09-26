//! `vkdg replay <fixture-path>` — replay a recorded request against the gateway.
//!
//! Fixture format (YAML):
//! ```yaml
//! request:
//!   method: POST
//!   path: /v1/messages
//!   headers:
//!     content-type: application/json
//!   body: |
//!     {"model": "claude-3-5-haiku-20241022", "messages": [{"role": "user", "content": "ping"}], "max_tokens": 10}
//! expected:
//!   status: 200          # optional; if present, assert
//!   body_contains: null  # optional; check body contains this string
//! ```
//!
//! Sends the request to VKDG_BASE_URL (default: http://127.0.0.1:8080).
//! Pass `self_test: true` (or `--self-test` on the CLI) to start an in-process
//! mock gateway instead.

use anyhow::Result;
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Deserialize)]
struct Fixture {
    request: FixtureRequest,
    expected: Option<FixtureExpected>,
}

#[derive(Deserialize)]
struct FixtureRequest {
    method: String,
    path: String,
    #[serde(default)]
    headers: HashMap<String, String>,
    body: Option<String>,
}

#[derive(Deserialize)]
struct FixtureExpected {
    status: Option<u16>,
    body_contains: Option<String>,
}

// ── Self-test in-process server ───────────────────────────────────────────────

async fn start_self_test_server() -> (String, tokio::sync::oneshot::Sender<()>) {
    use axum::{
        routing::{get, post},
        Json, Router,
    };
    use tokio::net::TcpListener;

    let app = Router::new()
        .route(
            "/health",
            get(|| async { Json(serde_json::json!({"status": "ok"})) }),
        )
        .route(
            "/vkdg/v1/info",
            get(|| async { Json(serde_json::json!({"version": "0.1.0"})) }),
        )
        .route(
            "/v1/messages",
            post(|| async {
                Json(serde_json::json!({
                    "id": "msg_test",
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type": "text", "text": "self-test response"}],
                    "model": "claude-3-5-haiku-20241022",
                    "stop_reason": "end_turn",
                    "usage": {"input_tokens": 5, "output_tokens": 5}
                }))
            }),
        )
        .route(
            "/v1/chat/completions",
            post(|| async {
                Json(serde_json::json!({
                    "id": "chatcmpl-test",
                    "object": "chat.completion",
                    "choices": [{
                        "index": 0,
                        "message": {"role": "assistant", "content": "self-test"},
                        "finish_reason": "stop"
                    }],
                    "usage": {"prompt_tokens": 5, "completion_tokens": 5}
                }))
            }),
        );

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
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
    // Give the server a moment to start accepting connections.
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    (base_url, tx)
}

// ── Public entry point ────────────────────────────────────────────────────────

pub async fn run(fixture_path: &str, base_url: Option<&str>, self_test: bool) -> Result<()> {
    let (base, _shutdown) = if self_test {
        let (url, tx) = start_self_test_server().await;
        (url, Some(tx))
    } else {
        let url = base_url
            .map(|s| s.to_string())
            .or_else(|| std::env::var("VKDG_BASE_URL").ok())
            .unwrap_or_else(|| "http://127.0.0.1:8080".into());
        (url, None)
    };

    let fixture_text = std::fs::read_to_string(fixture_path)
        .map_err(|e| anyhow::anyhow!("cannot read fixture '{}': {}", fixture_path, e))?;
    let fixture: Fixture = serde_yaml::from_str(&fixture_text)
        .map_err(|e| anyhow::anyhow!("invalid fixture YAML: {}", e))?;

    let url = format!("{}{}", base, fixture.request.path);
    let client = reqwest::Client::new();
    let method = reqwest::Method::from_bytes(fixture.request.method.as_bytes())
        .map_err(|e| anyhow::anyhow!("invalid method: {}", e))?;

    let mut req = client.request(method, &url);
    for (k, v) in &fixture.request.headers {
        req = req.header(k.as_str(), v.as_str());
    }
    if let Some(body) = &fixture.request.body {
        req = req.body(body.clone());
    }

    println!(
        "Replaying {} {} -> {}",
        fixture.request.method, fixture.request.path, url
    );

    let resp = req
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("request failed: {}", e))?;

    let status = resp.status().as_u16();
    let body = resp.text().await.unwrap_or_default();

    println!("Status: {}", status);
    println!(
        "Body: {}",
        if body.len() > 500 {
            &body[..500]
        } else {
            &body
        }
    );

    if let Some(expected) = &fixture.expected {
        if let Some(exp_status) = expected.status {
            anyhow::ensure!(
                status == exp_status,
                "status mismatch: expected {}, got {}",
                exp_status,
                status
            );
        }
        if let Some(contains) = &expected.body_contains {
            anyhow::ensure!(
                body.contains(contains.as_str()),
                "body does not contain '{}'\nBody: {}",
                contains,
                body
            );
        }
    }

    println!("OK");
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // Plausible wrong impl: --self-test starts a server that doesn't respond,
    // causing the replay to hang or return a connection error.
    #[tokio::test]
    async fn self_test_replay_health_check_passes() {
        let mut fixture = tempfile::NamedTempFile::new().unwrap();
        std::io::Write::write_all(
            fixture.as_file_mut(),
            b"request:\n  method: GET\n  path: /health\nexpected:\n  status: 200\n  body_contains: 'status'",
        )
        .unwrap();
        let result = run(fixture.path().to_str().unwrap(), None, true).await;
        assert!(result.is_ok(), "self-test replay must pass: {result:?}");
    }

    #[tokio::test]
    async fn self_test_replay_messages_passes() {
        let mut fixture = tempfile::NamedTempFile::new().unwrap();
        std::io::Write::write_all(
            fixture.as_file_mut(),
            b"request:\n  method: POST\n  path: /v1/messages\n  headers:\n    content-type: application/json\n  body: |\n    {\"model\": \"claude-3-5-haiku-20241022\", \"messages\": [{\"role\": \"user\", \"content\": \"ping\"}], \"max_tokens\": 10}\nexpected:\n  status: 200\n  body_contains: '\"role\":\"assistant\"'",
        )
        .unwrap();
        let result = run(fixture.path().to_str().unwrap(), None, true).await;
        assert!(
            result.is_ok(),
            "self-test messages replay must pass: {result:?}"
        );
    }
}
