use crate::handlers::requests::RequestLog;
use crate::session::{KeyStore, SessionStore};
use axum::{
    routing::{delete, get, post},
    Router,
};
use std::sync::Arc;
use std::time::Instant;
use vkdg_combos::ComboResolver;
use vkdg_config::ConfigRx;
use vkdg_connections::ConnectionCatalog;

#[derive(Clone)]
pub struct AdminState {
    pub sessions: Arc<SessionStore>,
    pub config_rx: ConfigRx,
    pub started_at: Arc<Instant>,
    pub key_store: Arc<KeyStore>,
    pub request_log: Arc<RequestLog>,
    pub combo_resolver: Option<Arc<ComboResolver>>,
    pub catalog: Option<Arc<ConnectionCatalog>>,
}

pub fn build_admin_router(state: AdminState) -> Router {
    Router::new()
        .route("/admin/v1/system", get(crate::handlers::system::get_system))
        .route("/admin/v1/session", post(crate::handlers::session::login))
        .route(
            "/admin/v1/session",
            delete(crate::handlers::session::logout),
        )
        .route("/admin/v1/session/me", get(crate::handlers::session::me))
        .route(
            "/admin/v1/connections",
            get(crate::handlers::connections::list_connections),
        )
        .route(
            "/admin/v1/connections/{id}",
            get(crate::handlers::connections::get_connection),
        )
        .route(
            "/admin/v1/keys",
            get(crate::handlers::keys::list_keys).post(crate::handlers::keys::create_key),
        )
        .route(
            "/admin/v1/keys/{id}",
            delete(crate::handlers::keys::revoke_key),
        )
        .route(
            "/admin/v1/routes",
            get(crate::handlers::routes::list_routes),
        )
        .route(
            "/admin/v1/routes/preview",
            get(crate::handlers::routes::preview_route),
        )
        .route(
            "/admin/v1/requests",
            get(crate::handlers::requests::list_requests),
        )
        .route(
            "/admin/v1/requests/{id}",
            get(crate::handlers::requests::get_request),
        )
        .route(
            "/admin/v1/combos",
            get(crate::handlers::routes::list_combos),
        )
        .with_state(state)
}
