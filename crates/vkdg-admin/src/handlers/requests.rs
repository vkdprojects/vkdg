use crate::{
    error::{AdminError, AdminErrorResponse},
    handlers::session::get_session,
    router::AdminState,
};
use axum::{
    extract::{Path, Query, State},
    response::{IntoResponse, Response},
    Json,
};
use http::{HeaderMap, StatusCode};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExcludedInfo {
    pub id: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionInfo {
    pub route_id: Option<String>,
    pub attempt_count: u32,
    pub candidates_excluded: Vec<ExcludedInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestRecord {
    pub request_id: String,
    pub model: String,
    pub api_type: String,
    pub status: String,
    pub connection_id: Option<String>,
    /// Client API key id (never the key itself); `anonymous` when auth is off.
    #[serde(default)]
    pub key_id: Option<String>,
    pub started_at_ms: i64,
    pub duration_ms: Option<i64>,
    pub decision: Option<DecisionInfo>,
    /// Tokens the response reported, read from the body the client received.
    /// Absent until the body finishes, or when the provider reported none.
    #[serde(default)]
    pub input_tokens: Option<u64>,
    #[serde(default)]
    pub output_tokens: Option<u64>,
    /// Cost in microdollars from the provider's list price; absent when the
    /// provider declares no price for the model (subscription, free tier).
    #[serde(default)]
    pub cost_microdollars: Option<u64>,
    /// Model stop reason: `end_turn`, `max_tokens`, `tool_use`, `stop_sequence`.
    /// Absent until the stream ends. Key for diagnosing empty responses.
    #[serde(default)]
    pub stop_reason: Option<String>,
    /// Error message when status is `failed`. Never contains prompt text.
    #[serde(default)]
    pub error_message: Option<String>,
    /// Whether extended reasoning (thinking) was requested by the client.
    #[serde(default)]
    pub thinking_requested: Option<bool>,
    /// Number of messages in the conversation (proxy for context depth).
    #[serde(default)]
    pub message_count: Option<u32>,
    /// Pipeline phase timestamps: [(`phase_name`, `unix_ms`), ...].
    /// Populated at request start; absent when the record was written by an older build.
    #[serde(default)]
    pub state_transitions: Option<Vec<(String, i64)>>,
    /// Prompt cache tokens: read from and written to the provider's cache.
    /// Absent when the provider reported no cache activity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<u64>,
    /// Kiro context window usage as a percentage (0–100+). Only set when the
    /// Kiro provider reported a `contextUsageEvent`. Absent for all other providers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_usage_pct: Option<f64>,
}

/// Status of a row whose response body is still being sent.
pub const STATUS_PENDING: &str = "pending";

/// Request history for the admin API, persisted in SQLite so it survives a
/// restart. Holds metadata only: never prompts, responses, or credentials
/// (invariant 3 applies here as it does to `DecisionRecord`).
pub struct RequestLog {
    conn: Mutex<rusqlite::Connection>,
    retain: usize,
}

/// Rows kept by default; older ones are pruned as new ones arrive.
pub const DEFAULT_RETAIN: usize = 10_000;

impl RequestLog {
    /// In-memory log (tests, and when no data directory is available).
    pub fn new() -> Arc<Self> {
        let conn = rusqlite::Connection::open_in_memory().expect("in-memory sqlite");
        Arc::new(Self::init(conn, DEFAULT_RETAIN).expect("request log schema"))
    }

    /// Open (or create) the log at `path`, keeping the newest `retain` rows.
    pub fn open(path: &std::path::Path, retain: usize) -> Result<Arc<Self>, String> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        let conn =
            rusqlite::Connection::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::init(conn, retain).map(Arc::new)
    }

    fn init(conn: rusqlite::Connection, retain: usize) -> Result<Self, String> {
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA busy_timeout = 2000;
             CREATE TABLE IF NOT EXISTS requests (
                 seq         INTEGER PRIMARY KEY AUTOINCREMENT,
                 request_id  TEXT NOT NULL UNIQUE,
                 status      TEXT NOT NULL,
                 started_at  INTEGER NOT NULL,
                 record      TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS requests_status ON requests (status, seq);",
        )
        .map_err(|e| format!("request log init: {e}"))?;
        Ok(Self {
            conn: Mutex::new(conn),
            retain: retain.max(1),
        })
    }

    pub fn push(&self, r: &RequestRecord) {
        let Ok(json) = serde_json::to_string(r) else {
            return;
        };
        let conn = self.conn.lock();
        let written = conn
            .execute(
                "INSERT OR REPLACE INTO requests (request_id, status, started_at, record)
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![r.request_id, r.status, r.started_at_ms, json],
            )
            .and_then(|_| {
                // Keep the newest `retain` rows. `seq` only grows, so this is an
                // index range delete, not a scan.
                conn.execute(
                    "DELETE FROM requests WHERE seq <= (SELECT MAX(seq) FROM requests) - ?1",
                    [i64::try_from(self.retain).unwrap_or(i64::MAX)],
                )
            });
        if let Err(e) = written {
            // Logging must never fail a request; the operator sees it here.
            eprintln!("request log write failed: {e}");
        }
    }

    /// Settle a row once its response body is done: record the tokens it
    /// reported and, for a row still `pending`, the final status (`completed`
    /// when the body ran to the end, `cancelled` when the client hung up) and
    /// the duration including the stream. Unknown ids are ignored.
    pub fn finish(
        &self,
        id: &str,
        tokens: vkdg_core::pricing::BilledTokens,
        body_ended: bool,
        price: Option<&vkdg_core::pricing::ModelPrice>,
        stop_reason: Option<String>,
        context_usage_pct: Option<f64>,
    ) {
        let conn = self.conn.lock();
        let Ok(json) = conn.query_row(
            "SELECT record FROM requests WHERE request_id = ?1",
            [id],
            |r| r.get::<_, String>(0),
        ) else {
            return;
        };
        let Ok(mut r) = serde_json::from_str::<RequestRecord>(&json) else {
            return;
        };
        if tokens.input + tokens.output > 0 {
            r.input_tokens = Some(tokens.input);
            r.output_tokens = Some(tokens.output);
            r.cost_microdollars = price.map(|p| p.cost_microdollars(tokens));
        }
        if tokens.cache_read > 0 {
            r.cache_read_tokens = Some(tokens.cache_read);
        }
        if tokens.cache_write > 0 {
            r.cache_write_tokens = Some(tokens.cache_write);
        }
        if let Some(sr) = stop_reason {
            r.stop_reason = Some(sr);
        }
        if let Some(pct) = context_usage_pct {
            r.context_usage_pct = Some(pct);
        }
        if r.status == STATUS_PENDING {
            r.status = if body_ended { "completed" } else { "cancelled" }.into();
            r.duration_ms = Some(chrono::Utc::now().timestamp_millis() - r.started_at_ms);
        }
        let Ok(json) = serde_json::to_string(&r) else {
            return;
        };
        if let Err(e) = conn.execute(
            "UPDATE requests SET status = ?2, record = ?3 WHERE request_id = ?1",
            rusqlite::params![id, r.status, json],
        ) {
            eprintln!("request log update failed: {e}");
        }
    }

    /// Newest first, optionally filtered by status.
    pub fn list(&self, limit: usize, status_filter: Option<&str>) -> (Vec<RequestRecord>, bool) {
        let conn = self.conn.lock();
        let fetch = i64::try_from(limit.saturating_add(1)).unwrap_or(i64::MAX);
        let rows: rusqlite::Result<Vec<String>> = match status_filter {
            Some(status) => conn
                .prepare("SELECT record FROM requests WHERE status = ?1 ORDER BY seq DESC LIMIT ?2")
                .and_then(|mut st| {
                    st.query_map(rusqlite::params![status, fetch], |r| r.get(0))?
                        .collect()
                }),
            None => conn
                .prepare("SELECT record FROM requests ORDER BY seq DESC LIMIT ?1")
                .and_then(|mut st| st.query_map([fetch], |r| r.get(0))?.collect()),
        };
        let mut records: Vec<RequestRecord> = rows
            .unwrap_or_default()
            .iter()
            .filter_map(|j| serde_json::from_str(j).ok())
            .collect();
        let has_more = records.len() > limit;
        records.truncate(limit);
        (records, has_more)
    }

    pub fn get(&self, id: &str) -> Option<RequestRecord> {
        let conn = self.conn.lock();
        let json: String = conn
            .query_row(
                "SELECT record FROM requests WHERE request_id = ?1",
                [id],
                |r| r.get(0),
            )
            .ok()?;
        serde_json::from_str(&json).ok()
    }
}

#[derive(Serialize)]
struct RequestList {
    items: Vec<RequestRecord>,
    has_more: bool,
    cursor: Option<String>,
}

#[derive(Deserialize)]
pub struct ListQuery {
    pub limit: Option<usize>,
    pub status: Option<String>,
}

pub async fn list_requests(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Query(q): Query<ListQuery>,
) -> Response {
    if get_session(&state, &headers).is_none() {
        return AdminErrorResponse(StatusCode::UNAUTHORIZED, AdminError::unauthorized())
            .into_response();
    }
    let limit = q.limit.unwrap_or(50).min(200);
    let (items, has_more) = state.request_log.list(limit, q.status.as_deref());
    Json(RequestList {
        items,
        has_more,
        cursor: None,
    })
    .into_response()
}

pub async fn get_request(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if get_session(&state, &headers).is_none() {
        return AdminErrorResponse(StatusCode::UNAUTHORIZED, AdminError::unauthorized())
            .into_response();
    }
    match state.request_log.get(&id) {
        Some(r) => Json(r).into_response(),
        None => {
            AdminErrorResponse(StatusCode::NOT_FOUND, AdminError::not_found(&id)).into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::SessionStore;
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
            request_log: RequestLog::new(),
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

    #[tokio::test]
    async fn list_requests_requires_session() {
        let state = make_state();
        let resp = list_requests(
            State(state),
            HeaderMap::new(),
            Query(ListQuery {
                limit: None,
                status: None,
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn list_requests_returns_records() {
        let state = make_state();
        state.request_log.push(&RequestRecord {
            request_id: "req-1".into(),
            model: "claude-3-opus".into(),
            api_type: "messages".into(),
            status: "success".into(),
            connection_id: Some("conn-a".into()),
            key_id: None,
            started_at_ms: 1000,
            duration_ms: Some(42),
            decision: None,
            input_tokens: None,
            output_tokens: None,
            cost_microdollars: None,
            stop_reason: None,
            error_message: None,
            thinking_requested: None,
            message_count: None,
            state_transitions: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
            context_usage_pct: None,
        });
        state.request_log.push(&RequestRecord {
            request_id: "req-2".into(),
            model: "claude-3-sonnet".into(),
            api_type: "messages".into(),
            status: "error".into(),
            connection_id: None,
            key_id: None,
            started_at_ms: 2000,
            duration_ms: None,
            decision: None,
            input_tokens: None,
            output_tokens: None,
            cost_microdollars: None,
            stop_reason: None,
            error_message: None,
            thinking_requested: None,
            message_count: None,
            state_transitions: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
            context_usage_pct: None,
        });
        let headers = authed_headers(&state);
        let resp = list_requests(
            State(state),
            headers,
            Query(ListQuery {
                limit: Some(10),
                status: None,
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let val: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(val["items"].as_array().unwrap().len(), 2);
    }

    fn record(id: &str, status: &str) -> RequestRecord {
        RequestRecord {
            request_id: id.into(),
            model: "m".into(),
            api_type: "anthropic".into(),
            status: status.into(),
            connection_id: None,
            key_id: Some("key-1".into()),
            started_at_ms: 0,
            duration_ms: Some(1),
            decision: None,
            input_tokens: None,
            output_tokens: None,
            cost_microdollars: None,
            stop_reason: None,
            error_message: None,
            thinking_requested: None,
            message_count: None,
            state_transitions: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
            context_usage_pct: None,
        }
    }

    // The log lived in memory: every restart or deploy wiped request history.
    #[test]
    fn history_survives_reopen_newest_first_and_is_pruned() {
        let dir = std::env::temp_dir().join(format!("vkdg-reqlog-{}", uuid::Uuid::new_v4()));
        let path = dir.join("requests.db");
        {
            let log = RequestLog::open(&path, 3).unwrap();
            for (i, st) in ["completed", "failed", "completed", "completed"]
                .iter()
                .enumerate()
            {
                log.push(&record(&format!("r{i}"), st));
            }
        }
        let log = RequestLog::open(&path, 3).unwrap();
        let (items, more) = log.list(10, None);
        let ids: Vec<&str> = items.iter().map(|r| r.request_id.as_str()).collect();
        assert_eq!(ids, ["r3", "r2", "r1"], "newest first, oldest pruned");
        assert!(!more);
        let (failed, _) = log.list(10, Some("failed"));
        assert_eq!(failed.len(), 1);
        assert_eq!(log.get("r1").unwrap().key_id.as_deref(), Some("key-1"));
        assert!(log.get("r0").is_none());
        let (one, more) = log.list(1, None);
        assert_eq!((one.len(), more), (1, true));
        let _ = std::fs::remove_dir_all(dir);
    }

    // Streaming rows were written when headers left, so they said `completed`
    // with no tokens even for a stream the client abandoned halfway.
    #[test]
    fn finish_settles_pending_rows_only() {
        let log = RequestLog::new();
        log.push(&record("streamed", STATUS_PENDING));
        log.push(&record("dropped", STATUS_PENDING));
        log.push(&record("failed", "failed"));
        use vkdg_core::pricing::{BilledTokens, ModelPrice};
        let t = |input, output| BilledTokens {
            input,
            output,
            ..BilledTokens::default()
        };
        let price = ModelPrice::new("m", 3_000_000, 15_000_000);
        log.finish("streamed", t(11, 4), true, Some(&price), None, None);
        log.finish("dropped", t(11, 1), false, None, None, None);
        log.finish("failed", t(0, 0), true, Some(&price), None, None);
        log.finish("unknown", t(1, 1), true, None, None, None);

        let r = log.get("streamed").unwrap();
        assert_eq!(r.status, "completed");
        assert_eq!((r.input_tokens, r.output_tokens), (Some(11), Some(4)));
        // 11 * $3/M + 4 * $15/M = 33 + 60 µ$.
        assert_eq!(r.cost_microdollars, Some(93));
        assert!(r.duration_ms.unwrap() > 1, "duration covers the stream");
        let d = log.get("dropped").unwrap();
        assert_eq!(d.status, "cancelled");
        assert_eq!(d.cost_microdollars, None, "no price is unknown, not $0");
        let f = log.get("failed").unwrap();
        assert_eq!((f.status.as_str(), f.input_tokens), ("failed", None));
        assert_eq!(f.duration_ms, Some(1), "a settled row keeps its duration");
        let (done, _) = log.list(10, Some("completed"));
        assert_eq!(done.len(), 1, "status column follows the record");
    }
}
