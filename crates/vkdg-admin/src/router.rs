use std::sync::Arc;
use std::time::Instant;
use axum::{
    routing::{delete, get, post},
    Router,
};
use vkdg_config::ConfigRx;
use crate::session::SessionStore;

#[derive(Clone)]
pub struct AdminState {
    pub sessions: Arc<SessionStore>,
    pub config_rx: ConfigRx,
    pub started_at: Arc<Instant>,
}

pub fn build_admin_router(state: AdminState) -> Router {
    Router::new()
        .route("/admin/v1/system", get(crate::handlers::system::get_system))
        .route("/admin/v1/session", post(crate::handlers::session::login))
        .route("/admin/v1/session", delete(crate::handlers::session::logout))
        .route("/admin/v1/session/me", get(crate::handlers::session::me))
        .route(
            "/admin/v1/connections",
            get(crate::handlers::connections::list_connections),
        )
        .route(
            "/admin/v1/connections/{id}",
            get(crate::handlers::connections::get_connection),
        )
        .with_state(state)
}
