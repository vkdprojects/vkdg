use crate::{
    error::{AdminError, AdminErrorResponse},
    handlers::session::get_session,
    router::AdminState,
};
use axum::{
    extract::State,
    response::{IntoResponse, Response},
    Json,
};
use http::{HeaderMap, StatusCode};
use serde::Serialize;
use std::collections::HashMap;

// ── /admin/v1/stats ──────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct ErrorEntry {
    pub message: String,
    pub count: u64,
}

#[derive(Serialize)]
pub struct StatsResponse {
    /// Number of records sampled (up to 200).
    pub total: usize,
    /// Records with status `completed`.
    pub ok: usize,
    /// Records with status `failed`.
    pub failed: usize,
    /// Fraction of completed / total; 0.0 when no records exist.
    pub success_rate: f64,
    pub p50_ms: Option<i64>,
    pub p95_ms: Option<i64>,
    pub p99_ms: Option<i64>,
    /// Count per `stop_reason` string (e.g. `end_turn`, `max_tokens`).
    pub stop_reasons: HashMap<String, u64>,
    /// Top error messages by frequency, descending.
    pub errors: Vec<ErrorEntry>,
}

/// Ceiling-based percentile over a sorted slice.
fn percentile(sorted: &[i64], p: f64) -> Option<i64> {
    if sorted.is_empty() {
        return None;
    }
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    let idx = {
        let n = sorted.len();
        let raw = ((p / 100.0) * n as f64).ceil();
        (raw as usize).min(n)
    };
    Some(sorted[idx.saturating_sub(1).min(sorted.len() - 1)])
}

pub async fn get_stats(State(state): State<AdminState>, headers: HeaderMap) -> Response {
    if get_session(&state, &headers).is_none() {
        return AdminErrorResponse(StatusCode::UNAUTHORIZED, AdminError::unauthorized())
            .into_response();
    }

    let (records, _) = state.request_log.list(200, None);
    let total = records.len();
    let ok = records.iter().filter(|r| r.status == "completed").count();
    let failed = records.iter().filter(|r| r.status == "failed").count();
    let success_rate = if total > 0 {
        #[allow(clippy::cast_precision_loss)]
        {
            ok as f64 / total as f64
        }
    } else {
        0.0
    };

    let mut durations: Vec<i64> = records
        .iter()
        .filter_map(|r| r.duration_ms)
        .filter(|&d| d >= 0)
        .collect();
    durations.sort_unstable();

    let p50_ms = percentile(&durations, 50.0);
    let p95_ms = percentile(&durations, 95.0);
    let p99_ms = percentile(&durations, 99.0);

    let mut stop_reasons: HashMap<String, u64> = HashMap::new();
    for r in &records {
        if let Some(sr) = &r.stop_reason {
            *stop_reasons.entry(sr.clone()).or_insert(0) += 1;
        }
    }

    let mut error_map: HashMap<String, u64> = HashMap::new();
    for r in &records {
        if r.status == "failed" {
            if let Some(msg) = &r.error_message {
                *error_map.entry(msg.clone()).or_insert(0) += 1;
            }
        }
    }
    let mut errors: Vec<ErrorEntry> = error_map
        .into_iter()
        .map(|(message, count)| ErrorEntry { message, count })
        .collect();
    errors.sort_by_key(|e: &ErrorEntry| std::cmp::Reverse(e.count));

    Json(StatsResponse {
        total,
        ok,
        failed,
        success_rate,
        p50_ms,
        p95_ms,
        p99_ms,
        stop_reasons,
        errors,
    })
    .into_response()
}

// ── /metrics (Prometheus) ─────────────────────────────────────────────────────

pub async fn get_metrics(State(state): State<AdminState>) -> Response {
    use vkdg_connections::ConnectionState as S;

    let (records, _) = state.request_log.list(200, None);

    // Tally request counts by (status, stop_reason).
    let mut completed_by_reason: HashMap<String, u64> = HashMap::new();
    let mut failed_count: u64 = 0;
    let mut cancelled_count: u64 = 0;
    let mut durations: Vec<i64> = Vec::new();

    for r in &records {
        match r.status.as_str() {
            "completed" => {
                let reason = r.stop_reason.as_deref().unwrap_or("unknown").to_string();
                *completed_by_reason.entry(reason).or_insert(0) += 1;
                if let Some(d) = r.duration_ms {
                    if d >= 0 {
                        durations.push(d);
                    }
                }
            }
            "failed" => failed_count += 1,
            "cancelled" => cancelled_count += 1,
            _ => {}
        }
    }

    durations.sort_unstable();
    let p50 = percentile(&durations, 50.0).unwrap_or(0);
    let p95 = percentile(&durations, 95.0).unwrap_or(0);
    let p99 = percentile(&durations, 99.0).unwrap_or(0);

    // Connection health counts from the live catalog.
    let (connections_active, connections_healthy) = match &state.catalog {
        None => (0u64, 0u64),
        Some(catalog) => {
            let ids = catalog.connection_ids();
            let total_conns = ids.len() as u64;
            let mut healthy_count: u64 = 0;
            for id in &ids {
                if let Some(conn) = catalog.get(id) {
                    if let Ok(guard) = conn.try_read() {
                        if matches!(&guard.state, S::Healthy) {
                            healthy_count += 1;
                        }
                    }
                }
            }
            (total_conns, healthy_count)
        }
    };

    // Build Prometheus exposition format.
    let mut out = String::with_capacity(512);

    let mut sorted_reasons: Vec<(&String, &u64)> = completed_by_reason.iter().collect();
    sorted_reasons.sort_by_key(|(k, _)| k.as_str());
    for (reason, count) in sorted_reasons {
        use std::fmt::Write as _;
        let _ = writeln!(
            out,
            "vkdg_requests_total{{status=\"completed\",stop_reason=\"{reason}\"}} {count}"
        );
    }
    use std::fmt::Write as _;
    let _ = writeln!(
        out,
        "vkdg_requests_total{{status=\"failed\"}} {failed_count}"
    );
    let _ = writeln!(
        out,
        "vkdg_requests_total{{status=\"cancelled\"}} {cancelled_count}"
    );
    let _ = writeln!(out, "vkdg_request_duration_p50_ms {p50}");
    let _ = writeln!(out, "vkdg_request_duration_p95_ms {p95}");
    let _ = writeln!(out, "vkdg_request_duration_p99_ms {p99}");
    let _ = writeln!(out, "vkdg_connections_active {connections_active}");
    let _ = writeln!(out, "vkdg_connections_healthy {connections_healthy}");

    (
        StatusCode::OK,
        [(
            http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        out,
    )
        .into_response()
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{handlers::requests::RequestRecord, router::AdminState, session::SessionStore};
    use axum::extract::State;
    use http::HeaderMap;
    use std::sync::Arc;
    use std::time::Instant;
    use tokio::sync::watch;
    use vkdg_config::ConfigSnapshot;

    fn make_state() -> AdminState {
        let snap = ConfigSnapshot::default_empty();
        let (_tx, rx) = watch::channel(Arc::new(snap));
        AdminState {
            sessions: SessionStore::new("tok".into()),
            config_rx: rx,
            started_at: Arc::new(Instant::now()),
            key_store: Arc::new(vkdg_governance::VirtualKeyStore::in_memory().unwrap()),
            request_log: crate::handlers::requests::RequestLog::new(),
            combos: None,
            reload_plugins: None,
            connection_tester: None,
            catalog: None,
            logins: None,
            gateway_store: None,
            config_tx: None,
        }
    }

    fn authed_headers(state: &AdminState) -> HeaderMap {
        let session = state.sessions.bootstrap_login().unwrap();
        let mut h = HeaderMap::new();
        let val = format!("vkdg_session={}", session.session_id);
        h.insert(
            http::header::COOKIE,
            http::HeaderValue::from_str(&val).unwrap(),
        );
        h
    }

    fn make_record(
        id: &str,
        status: &str,
        duration_ms: Option<i64>,
        stop_reason: Option<&str>,
    ) -> RequestRecord {
        RequestRecord {
            request_id: id.into(),
            model: "m".into(),
            api_type: "anthropic".into(),
            status: status.into(),
            connection_id: None,
            key_id: None,
            started_at_ms: 0,
            duration_ms,
            decision: None,
            input_tokens: None,
            output_tokens: None,
            cost_microdollars: None,
            stop_reason: stop_reason.map(Into::into),
            error_message: None,
            thinking_requested: None,
            message_count: None,
            state_transitions: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
            context_usage_pct: None,
        }
    }

    #[tokio::test]
    async fn stats_requires_session() {
        let state = make_state();
        let resp = get_stats(State(state), HeaderMap::new()).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn stats_empty_log() {
        let state = make_state();
        let headers = authed_headers(&state);
        let resp = get_stats(State(state), headers).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let val: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(val["total"], 0);
        assert_eq!(val["ok"], 0);
        assert_eq!(val["failed"], 0);
        assert_eq!(val["success_rate"], 0.0);
        assert!(val["p50_ms"].is_null());
    }

    #[tokio::test]
    async fn stats_aggregates_records() {
        let state = make_state();
        state
            .request_log
            .push(&make_record("r1", "completed", Some(100), Some("end_turn")));
        state
            .request_log
            .push(&make_record("r2", "completed", Some(200), Some("end_turn")));
        state.request_log.push(&make_record(
            "r3",
            "completed",
            Some(300),
            Some("max_tokens"),
        ));
        state
            .request_log
            .push(&make_record("r4", "failed", None, None));

        let headers = authed_headers(&state);
        let resp = get_stats(State(state), headers).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let val: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(val["total"], 4);
        assert_eq!(val["ok"], 3);
        assert_eq!(val["failed"], 1);
        // success_rate = 3/4 = 0.75
        let sr = val["success_rate"].as_f64().unwrap();
        assert!((sr - 0.75).abs() < 1e-9, "expected 0.75, got {sr}");
        // p50 of [100, 200, 300] = 200
        assert_eq!(val["p50_ms"], 200);
        // stop_reasons
        assert_eq!(val["stop_reasons"]["end_turn"], 2);
        assert_eq!(val["stop_reasons"]["max_tokens"], 1);
    }

    #[tokio::test]
    async fn metrics_no_auth_required() {
        let state = make_state();
        state
            .request_log
            .push(&make_record("r1", "completed", Some(50), Some("end_turn")));
        state
            .request_log
            .push(&make_record("r2", "failed", None, None));

        // No auth headers — should succeed.
        let resp = get_metrics(State(state)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let ct = resp.headers().get(http::header::CONTENT_TYPE).unwrap();
        assert!(ct.to_str().unwrap().starts_with("text/plain"));
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let body = std::str::from_utf8(&bytes).unwrap();
        assert!(
            body.contains("vkdg_requests_total{status=\"completed\""),
            "missing completed line"
        );
        assert!(
            body.contains("vkdg_requests_total{status=\"failed\"} 1"),
            "missing failed count"
        );
        assert!(body.contains("vkdg_connections_active 0"));
        assert!(body.contains("vkdg_connections_healthy 0"));
    }

    #[test]
    fn percentile_empty() {
        assert_eq!(percentile(&[], 50.0), None);
    }

    #[test]
    fn percentile_single() {
        assert_eq!(percentile(&[42], 50.0), Some(42));
        assert_eq!(percentile(&[42], 99.0), Some(42));
    }

    #[test]
    fn percentile_multiple() {
        let sorted = vec![10, 20, 30, 40, 50, 60, 70, 80, 90, 100];
        // p50: ceil(0.5 * 10) = 5 → index 4 → 50
        assert_eq!(percentile(&sorted, 50.0), Some(50));
        // p95: ceil(0.95 * 10) = 10 → index 9 → 100
        assert_eq!(percentile(&sorted, 95.0), Some(100));
        // p99: ceil(0.99 * 10) = 10 → index 9 → 100
        assert_eq!(percentile(&sorted, 99.0), Some(100));
    }
}
