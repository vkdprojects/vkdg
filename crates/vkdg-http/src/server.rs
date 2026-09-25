use axum::{
    Router,
    response::{IntoResponse, Json},
    routing::{get, post},
};
use http::StatusCode;
use serde_json::json;
use tokio::net::TcpListener;
use tracing::info;

use vkdg_core::VkdgError;

use crate::app_state::AppState;
use crate::frontdoor::ServerConfig;

/// POST /v1/messages
///
/// Checks the admission guard first; returns 503 when at capacity.
/// When a full pipeline is wired, delegates to it; otherwise 501.
async fn handle_messages(
    axum::extract::State(state): axum::extract::State<AppState>,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    // Admission check — must happen before any pipeline work.
    if let Err(e) = state.front_door.admission.acquire().await {
        let code = match &e {
            VkdgError::AdmissionRejected { .. } => StatusCode::SERVICE_UNAVAILABLE,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        return (
            code,
            Json(json!({"error": {"type": "admission_rejected", "message": e.to_string()}})),
        ).into_response();
    }

    // Pipeline not wired yet — return 501.
    let _ = body;
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(json!({"error": {"type": "not_implemented", "message": "pipeline not wired"}})),
    ).into_response()
}

async fn stub_501() -> impl IntoResponse {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(json!({"error": {"type": "not_implemented", "message": "handler not yet wired"}})),
    )
}

async fn health() -> impl IntoResponse {
    Json(json!({"status": "ok"}))
}

async fn info() -> impl IntoResponse {
    Json(json!({"version": "0.1.0"}))
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/v1/messages", post(handle_messages))
        .route("/v1/chat/completions", post(stub_501))
        .route("/health", get(health))
        .route("/vkdg/v1/info", get(info))
        .with_state(state)
}

pub async fn serve(config: ServerConfig, router: Router) -> anyhow::Result<()> {
    let addr = config.listen_addr.clone();
    let listener = TcpListener::bind(&addr).await?;
    info!(addr = %addr, "vkdg listening");

    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

async fn shutdown_signal() {
    use tokio::signal;

    let ctrl_c = async {
        signal::ctrl_c().await.expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_state::AppState;
    use crate::frontdoor::ServerConfig;
    use axum::body::to_bytes;
    use http::{Method, Request};
    use tower::ServiceExt;

    fn app_with_limit(limit: usize) -> Router {
        build_router(AppState::new(ServerConfig {
            max_concurrent_requests: limit,
            ..ServerConfig::default()
        }))
    }

    fn messages_request() -> Request<axum::body::Body> {
        Request::builder()
            .method(Method::POST)
            .uri("/v1/messages")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                r#"{"model":"claude-3-5-sonnet-20241022","max_tokens":1,"messages":[]}"#,
            ))
            .unwrap()
    }

    // Plausible wrong impl: admission check is placed after pipeline dispatch,
    // so a full semaphore only surfaces inside the pipeline, not at the HTTP boundary.
    // This test defeats it: limit=0 must return 503 before any pipeline work runs.
    #[tokio::test]
    async fn admission_exhausted_returns_503_not_501() {
        let app = app_with_limit(0);
        let resp = app.oneshot(messages_request()).await.unwrap();
        assert_eq!(
            resp.status().as_u16(),
            503,
            "admission guard at limit=0 must return 503 SERVICE_UNAVAILABLE"
        );
    }

    // Plausible wrong impl: admission check returns 503 on ALL requests regardless
    // of semaphore state — guard logic is inverted.
    // This test defeats it: limit=1 must allow through (pipeline absent → 501, not 503).
    #[tokio::test]
    async fn admission_available_passes_through() {
        let app = app_with_limit(1);
        let resp = app.oneshot(messages_request()).await.unwrap();
        let status = resp.status().as_u16();
        assert_ne!(
            status, 503,
            "with capacity available, must not return 503 — got {status}"
        );
    }

    // Plausible wrong impl: /health is accidentally routed through admission check
    // and returns 503 when limit=0 instead of 200.
    // This test defeats it: /health must always return 200 regardless of admission.
    #[tokio::test]
    async fn health_bypasses_admission_guard() {
        let app = app_with_limit(0);
        let req = Request::builder()
            .method(Method::GET)
            .uri("/health")
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status().as_u16(), 200, "/health must not be blocked by admission");
    }
}
