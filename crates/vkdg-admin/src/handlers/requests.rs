use axum::{
    extract::{Path, Query, State},
    response::{IntoResponse, Response},
    Json,
};
use http::{HeaderMap, StatusCode};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use parking_lot::RwLock;
use crate::{
    error::{AdminError, AdminErrorResponse},
    handlers::session::get_session,
    router::AdminState,
};

#[derive(Debug, Clone, Serialize)]
pub struct ExcludedInfo {
    pub id: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DecisionInfo {
    pub route_id: Option<String>,
    pub attempt_count: u32,
    pub candidates_excluded: Vec<ExcludedInfo>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RequestRecord {
    pub request_id: String,
    pub model: String,
    pub api_type: String,
    pub status: String,
    pub connection_id: Option<String>,
    pub started_at_ms: i64,
    pub duration_ms: Option<i64>,
    pub decision: Option<DecisionInfo>,
}

pub struct RequestLog {
    records: RwLock<Vec<RequestRecord>>,
}

impl RequestLog {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { records: RwLock::new(Vec::new()) })
    }

    pub fn push(&self, r: RequestRecord) {
        let mut v = self.records.write();
        v.push(r);
        if v.len() > 1000 {
            v.remove(0);
        }
    }

    pub fn list(&self, limit: usize, status_filter: Option<&str>) -> (Vec<RequestRecord>, bool) {
        let v = self.records.read();
        let filtered: Vec<RequestRecord> = v
            .iter()
            .rev()
            .filter(|r| status_filter.is_none_or(|s| r.status == s))
            .cloned()
            .collect();
        let has_more = filtered.len() > limit;
        (filtered.into_iter().take(limit).collect(), has_more)
    }

    pub fn get(&self, id: &str) -> Option<RequestRecord> {
        self.records.read().iter().find(|r| r.request_id == id).cloned()
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
    Json(RequestList { items, has_more, cursor: None }).into_response()
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
        None => AdminErrorResponse(StatusCode::NOT_FOUND, AdminError::not_found(&id)).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Instant;
    use axum::extract::State;
    use http::HeaderMap;
    use tokio::sync::watch;
    use vkdg_config::ConfigSnapshot;
    use crate::session::{KeyStore, SessionStore};

    fn make_state() -> AdminState {
        let snap = ConfigSnapshot::default_empty();
        let (_tx, rx) = watch::channel(Arc::new(snap));
        AdminState {
            sessions: SessionStore::new("tok".into()),
            config_rx: rx,
            started_at: Arc::new(Instant::now()),
            key_store: KeyStore::new(),
            request_log: RequestLog::new(),
        }
    }

    fn authed_headers(state: &AdminState) -> HeaderMap {
        let session = state.sessions.bootstrap_login().unwrap();
        let mut h = HeaderMap::new();
        let val = format!("vkdg_session={}", session.session_id);
        h.insert(http::header::COOKIE, http::HeaderValue::from_str(&val).unwrap());
        h
    }

    #[tokio::test]
    async fn list_requests_requires_session() {
        let state = make_state();
        let resp = list_requests(
            State(state),
            HeaderMap::new(),
            Query(ListQuery { limit: None, status: None }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn list_requests_returns_records() {
        let state = make_state();
        state.request_log.push(RequestRecord {
            request_id: "req-1".into(),
            model: "claude-3-opus".into(),
            api_type: "messages".into(),
            status: "success".into(),
            connection_id: Some("conn-a".into()),
            started_at_ms: 1000,
            duration_ms: Some(42),
            decision: None,
        });
        state.request_log.push(RequestRecord {
            request_id: "req-2".into(),
            model: "claude-3-sonnet".into(),
            api_type: "messages".into(),
            status: "error".into(),
            connection_id: None,
            started_at_ms: 2000,
            duration_ms: None,
            decision: None,
        });
        let headers = authed_headers(&state);
        let resp = list_requests(
            State(state),
            headers,
            Query(ListQuery { limit: Some(10), status: None }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let val: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(val["items"].as_array().unwrap().len(), 2);
    }
}
