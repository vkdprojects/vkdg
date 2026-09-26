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

pub async fn run(fixture_path: &str, base_url: Option<&str>) -> Result<()> {
    let base = base_url
        .map(|s| s.to_string())
        .or_else(|| std::env::var("VKDG_BASE_URL").ok())
        .unwrap_or_else(|| "http://127.0.0.1:8080".into());

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
        if body.len() > 500 { &body[..500] } else { &body }
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
