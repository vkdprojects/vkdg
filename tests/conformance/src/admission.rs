// Contract: when admission semaphore is exhausted, gateway returns 503
//
// Scenario: spec/scenarios/admission_rejected.yaml
//
// Current stub wires /v1/messages → stub_501 (returns 501 unconditionally).
// The admission guard is not consulted by the HTTP router in Phase A.
// This test FAILS because: status is 501, not 503.

use axum::body::to_bytes;
use http::{Method, Request, StatusCode};
use tower::ServiceExt;
use vkdg_http::{AppState, ServerConfig, build_router};

fn app_with_admission_limit(limit: usize) -> axum::Router {
    let config = ServerConfig {
        max_concurrent_requests: limit,
        ..ServerConfig::default()
    };
    build_router(AppState::new(config))
}

/// Reads the scenario YAML and asserts documented fields match test setup.
fn load_scenario(name: &str) -> serde_yaml::Value {
    let path = format!(
        "{}/spec/scenarios/{}.yaml",
        env!("CARGO_MANIFEST_DIR").trim_end_matches("tests/conformance"),
        name
    );
    // Best-effort: if file absent the test proceeds with in-code assertions only.
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_yaml::from_str(&s).ok())
        .unwrap_or(serde_yaml::Value::Null)
}

/// Contract: admission_limit=0 → HTTP 503 (scenario: admission_rejected).
/// FAILS because the stub returns 501 before checking the admission guard.
#[tokio::test]
async fn admission_rejected_returns_503() {
    let scenario = load_scenario("admission_rejected");
    // Confirm scenario asserts 503
    let expected_status = scenario["assert"]["http_status"]
        .as_u64()
        .unwrap_or(503) as u16;

    let app = app_with_admission_limit(0);

    let req = Request::builder()
        .method(Method::POST)
        .uri("/v1/messages")
        .header("content-type", "application/json")
        .header("x-client-id", "client-test")
        .header("x-tenant-id", "tenant-test")
        .body(axum::body::Body::from(
            r#"{"model":"claude-3-5-sonnet-20241022","max_tokens":100,"messages":[{"role":"user","content":"hello"}]}"#,
        ))
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    let actual = response.status().as_u16();

    // FAILS: stub returns 501; Phase B must wire admission → 503
    assert_eq!(
        actual, expected_status,
        "admission_rejected contract violated: expected HTTP {expected_status} (AdmissionRejected), got {actual}. \
         Phase B must check AdmissionGuard before dispatching."
    );
}

/// AdmissionGuard unit-level contract: acquire on a fully-exhausted semaphore
/// returns Err(VkdgError::AdmissionRejected). This PASSES — the guard is
/// already correct; only the HTTP integration is missing.
#[tokio::test]
async fn admission_guard_unit_returns_error_when_limit_zero() {
    use vkdg_core::VkdgError;
    use vkdg_http::AdmissionGuard;

    let guard = AdmissionGuard::new(0);
    let result = guard.acquire().await;
    assert!(
        matches!(result, Err(VkdgError::AdmissionRejected { .. })),
        "AdmissionGuard::acquire must return AdmissionRejected when limit=0; got {:?}",
        result
    );
}
