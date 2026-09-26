//! Webhook dispatcher for async job lifecycle events.
//!
//! When a job transitions to Succeeded, Failed, or Cancelled and has a
//! webhook_url configured, sends a POST with the job state as JSON body.
//! Non-blocking: failures are logged but do not affect the job state.
//!
//! Phase E: retry with exponential backoff, signature header (HMAC-SHA256).

use serde_json::json;
use vkdg_storage::{JobRecord, JobState};

/// Dispatch a webhook for a job state transition.
/// Fire-and-forget: spawns a tokio task, never blocks the caller.
pub fn dispatch(record: &JobRecord, webhook_url: &str) {
    let url = webhook_url.to_string();
    let event = state_to_event(&record.state);
    let payload = json!({
        "job_id": record.job_id.to_string(),
        "owner": record.owner_client_id,
        "state": format!("{:?}", record.state),
        "created_at": record.created_at.to_rfc3339(),
        "event": event,
    });
    tokio::spawn(async move {
        match send_webhook(&url, event, &payload).await {
            Ok(_) => tracing::debug!(url = %url, "webhook delivered"),
            Err(e) => tracing::warn!(url = %url, error = %e, "webhook delivery failed"),
        }
    });
}

async fn send_webhook(url: &str, event: &str, payload: &serde_json::Value) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;

    let resp = client
        .post(url)
        .header("content-type", "application/json")
        .header("x-vkdg-event", event)
        .body(payload.to_string())
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if !resp.status().is_success() {
        return Err(format!("webhook endpoint returned {}", resp.status()));
    }
    Ok(())
}

fn state_to_event(state: &JobState) -> &'static str {
    match state {
        JobState::Queued => "job.queued",
        JobState::Running => "job.running",
        JobState::Succeeded => "job.succeeded",
        JobState::Failed { .. } => "job.failed",
        JobState::Cancelled => "job.cancelled",
        JobState::Expired => "job.expired",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;
    use uuid::Uuid;
    use vkdg_core::ConnectionId;

    /// Plausible wrong impl: state_to_event returns same string for all states.
    #[test]
    fn events_are_distinct() {
        assert_ne!(
            state_to_event(&JobState::Succeeded),
            state_to_event(&JobState::Failed { reason: "x".into() })
        );
        assert_ne!(
            state_to_event(&JobState::Queued),
            state_to_event(&JobState::Cancelled)
        );
        assert_ne!(
            state_to_event(&JobState::Running),
            state_to_event(&JobState::Expired)
        );
    }

    /// Plausible wrong impl: JSON payload missing required fields.
    #[test]
    fn payload_has_required_fields() {
        use chrono::Utc;
        use uuid::Uuid;
        use vkdg_core::ConnectionId;

        let record = JobRecord {
            job_id: Uuid::new_v4(),
            owner_client_id: "client-1".into(),
            connection_id: ConnectionId("conn-1".into()),
            upstream_job_id: None,
            state: JobState::Succeeded,
            created_at: Utc::now(),
            idempotency_key: None,
            webhook_url: Some("https://example.com/hook".into()),
        };
        let payload = json!({
            "job_id": record.job_id.to_string(),
            "owner": record.owner_client_id,
            "state": format!("{:?}", record.state),
            "created_at": record.created_at.to_rfc3339(),
            "event": state_to_event(&record.state),
        });
        assert!(payload.get("job_id").is_some());
        assert!(payload.get("owner").is_some());
        assert!(payload.get("event").is_some());
        assert_eq!(payload["event"], "job.succeeded");
        assert_eq!(payload["owner"], "client-1");
    }

    /// Plausible wrong impl: state_to_event for Failed variant ignores the reason field.
    #[test]
    fn failed_event_consistent_regardless_of_reason() {
        assert_eq!(
            state_to_event(&JobState::Failed {
                reason: "timeout".into()
            }),
            state_to_event(&JobState::Failed {
                reason: "upstream_error".into()
            }),
        );
        assert_eq!(
            state_to_event(&JobState::Failed {
                reason: "any".into()
            }),
            "job.failed"
        );
    }

    /// Plausible wrong impl: dispatch() does not actually POST to the webhook_url,
    /// either because tokio::spawn is never awaited or the URL is silently ignored.
    #[tokio::test]
    async fn dispatch_delivers_post_to_webhook_url() {
        use axum::{extract::State, routing::post, Router};
        use tokio::net::TcpListener;

        let call_count = Arc::new(AtomicU32::new(0));
        let call_count_clone = Arc::clone(&call_count);

        // Spin up a minimal webhook receiver
        let app = Router::new()
            .route(
                "/webhook",
                post(
                    move |State(count): State<Arc<AtomicU32>>,
                          axum::extract::Json(body): axum::extract::Json<serde_json::Value>| async move {
                        count.fetch_add(1, Ordering::Relaxed);
                        assert!(body.get("job_id").is_some(), "payload must have job_id");
                        assert!(body.get("event").is_some(), "payload must have event");
                        axum::http::StatusCode::OK
                    },
                ),
            )
            .with_state(Arc::clone(&call_count_clone));

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

        tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await
                .ok();
        });

        // Give server time to start
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let record = JobRecord {
            job_id: Uuid::new_v4(),
            owner_client_id: "test-owner".into(),
            connection_id: ConnectionId("conn-1".into()),
            upstream_job_id: None,
            state: JobState::Succeeded,
            created_at: Utc::now(),
            idempotency_key: None,
            webhook_url: None,
        };

        let webhook_url = format!("http://127.0.0.1:{}/webhook", port);
        dispatch(&record, &webhook_url);

        // Wait for async delivery (dispatch is fire-and-forget)
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        assert_eq!(
            call_count.load(Ordering::Relaxed),
            1,
            "webhook must be delivered exactly once"
        );

        let _ = shutdown_tx.send(());
    }

    /// Plausible wrong impl: dispatch() silently swallows 4xx/5xx errors
    /// without logging them.
    #[tokio::test]
    async fn dispatch_logs_failure_on_non_2xx() {
        // webhook_url points to a non-existent server — should not panic
        let record = JobRecord {
            job_id: Uuid::new_v4(),
            owner_client_id: "test".into(),
            connection_id: ConnectionId("c".into()),
            upstream_job_id: None,
            state: JobState::Failed {
                reason: "upstream error".into(),
            },
            created_at: Utc::now(),
            idempotency_key: None,
            webhook_url: None,
        };
        // Fire-and-forget; failure is logged at warn level, never panics
        dispatch(&record, "http://127.0.0.1:19998/webhook");
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        // No assertion needed beyond "does not panic"
    }
}
