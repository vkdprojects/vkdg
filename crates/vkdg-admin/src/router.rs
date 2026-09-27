use crate::handlers::requests::RequestLog;
use crate::session::SessionStore;
use axum::{
    routing::{delete, get, post},
    Router,
};
use std::sync::Arc;
use std::time::Instant;
use vkdg_config::ConfigRx;
use vkdg_connections::ConnectionCatalog;

#[derive(Clone)]
pub struct AdminState {
    pub sessions: Arc<SessionStore>,
    pub config_rx: ConfigRx,
    pub started_at: Arc<Instant>,
    /// Data-plane API keys. The same store instance `/v1/*` authenticates against.
    pub key_store: Arc<vkdg_governance::VirtualKeyStore>,
    pub request_log: Arc<RequestLog>,
    /// Combo edits; `None` when the gateway runs without a data plane.
    pub combos: Option<Arc<vkdg_combos::ComboService>>,
    pub catalog: Option<Arc<ConnectionCatalog>>,
    /// Provider login + account store; `None` disables `/oauth` and `/accounts`.
    pub logins: Option<Arc<crate::handlers::oauth::LoginService>>,
    /// Reload installed plugins into the running data plane after an install
    /// or removal. `None` = plugin changes apply at the next restart.
    pub reload_plugins: Option<PluginReload>,
}

/// Rebuilds the live provider and hook registries from the plugin directory.
pub type PluginReload = Arc<dyn Fn() + Send + Sync>;

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
            "/admin/v1/setup",
            get(crate::handlers::session::setup_status)
                .post(crate::handlers::session::setup_password),
        )
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
            delete(crate::handlers::keys::revoke_key).patch(crate::handlers::keys::update_key),
        )
        .route(
            "/admin/v1/keys/{id}/regenerate",
            post(crate::handlers::keys::regenerate_key),
        )
        .route(
            "/admin/v1/keys/{id}/disable",
            post(crate::handlers::keys::disable_key),
        )
        .route(
            "/admin/v1/keys/{id}/enable",
            post(crate::handlers::keys::enable_key),
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
            get(crate::handlers::combos::list_combos).post(crate::handlers::combos::create_combo),
        )
        .route(
            "/admin/v1/combos/{id}",
            axum::routing::put(crate::handlers::combos::update_combo)
                .delete(crate::handlers::combos::delete_combo),
        )
        .route(
            "/admin/v1/providers/oauth",
            get(crate::handlers::oauth::list_oauth_providers),
        )
        .route(
            "/admin/v1/providers/{id}/login-methods",
            get(crate::handlers::oauth::list_login_methods),
        )
        .route(
            "/admin/v1/oauth/{provider}/start",
            post(crate::handlers::oauth::start_login),
        )
        .route(
            "/admin/v1/oauth/{provider}/poll",
            post(crate::handlers::oauth::poll_login),
        )
        .route(
            "/admin/v1/oauth/{provider}/import",
            post(crate::handlers::oauth::import_token),
        )
        .route(
            "/admin/v1/plugins",
            get(crate::handlers::plugins::list_plugins)
                .post(crate::handlers::plugins::install_plugin),
        )
        .route(
            "/admin/v1/plugins/{name}",
            delete(crate::handlers::plugins::remove_plugin),
        )
        .route(
            "/admin/v1/accounts",
            get(crate::handlers::oauth::list_accounts),
        )
        .route(
            "/admin/v1/accounts/{id}",
            delete(crate::handlers::oauth::delete_account),
        )
        .with_state(state)
}
