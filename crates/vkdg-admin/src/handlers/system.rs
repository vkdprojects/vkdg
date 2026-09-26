use axum::{extract::State, Json};
use serde::Serialize;
use crate::router::AdminState;

#[derive(Serialize)]
pub struct SystemInfo {
    version: &'static str,
    status: &'static str,
    config_revision: u64,
    uptime_secs: u64,
    connection_count: usize,
    active_requests: u32,
}

pub async fn get_system(State(state): State<AdminState>) -> Json<SystemInfo> {
    let snapshot = state.config_rx.borrow().clone();
    Json(SystemInfo {
        version: env!("CARGO_PKG_VERSION"),
        status: "ok",
        config_revision: snapshot.version,
        uptime_secs: state.started_at.elapsed().as_secs(),
        connection_count: snapshot.connections.len(),
        active_requests: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Instant;
    use axum::extract::State;
    use tokio::sync::watch;
    use vkdg_config::ConfigSnapshot;

    fn make_state() -> AdminState {
        let snap = ConfigSnapshot::default_empty();
        let (_tx, rx) = watch::channel(Arc::new(snap));
        AdminState {
            sessions: crate::session::SessionStore::new("test-token".into()),
            config_rx: rx,
            started_at: Arc::new(Instant::now()),
            key_store: crate::session::KeyStore::new(),
            request_log: crate::handlers::requests::RequestLog::new(),
            combo_resolver: None,
            catalog: None,
        }
    }

    #[tokio::test]
    async fn system_returns_version() {
        let state = make_state();
        let Json(info) = get_system(State(state)).await;
        assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
        assert_eq!(info.status, "ok");
    }
}
