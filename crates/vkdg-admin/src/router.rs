use crate::handlers::requests::RequestLog;
use crate::session::SessionStore;
use axum::{
    routing::{delete, get, post, put},
    Router,
};
use std::future::Future;
use std::sync::Arc;
use std::time::Instant;
use vkdg_config::{ConfigRx, ConfigTx, GatewayStore};
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
    /// Fires a smoke request through a specific connection and returns latency and status.
    /// `None` = no data plane, the endpoint answers 503.
    pub connection_tester: Option<ConnectionTester>,
    /// Persistent store for connections/routes; `None` = read-only (no CRUD).
    pub gateway_store: Option<Arc<GatewayStore>>,
    /// Sender to push a new snapshot after every store write.
    pub config_tx: Option<ConfigTx>,
}

/// Sends one smoke request (`"Hello"`, `max_tokens=1`) through the named connection
/// and returns `(latency_ms, ok, error)`. Spawned as a blocking task if needed.
pub type ConnectionTester = Arc<
    dyn Fn(String) -> std::pin::Pin<Box<dyn Future<Output = ConnectionTestResult> + Send>>
        + Send
        + Sync,
>;

#[derive(Clone, serde::Serialize)]
pub struct ConnectionTestResult {
    /// Round-trip time in milliseconds.
    pub latency_ms: u64,
    /// `true` when the provider returned a response (any 2xx or a provider-level error
    /// such as "context too long" still counts — the connection itself is alive).
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
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
            get(crate::handlers::connections::list_connections)
                .post(crate::handlers::connections::create_connection),
        )
        .route(
            "/admin/v1/connections/{id}",
            get(crate::handlers::connections::get_connection)
                .put(crate::handlers::connections::update_connection)
                .patch(crate::handlers::connections::patch_connection)
                .delete(crate::handlers::connections::delete_connection),
        )
        .route(
            "/admin/v1/connections/{id}/test",
            axum::routing::post(crate::handlers::connections::test_connection),
        )
        .route(
            "/admin/v1/connections/{id}/reset-cooldown",
            axum::routing::post(crate::handlers::connections::reset_connection_cooldown),
        )
        .route(
            "/admin/v1/connections/{id}/models/sync",
            post(crate::handlers::model_sync::sync_models),
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
            get(crate::handlers::routes::list_routes).post(crate::handlers::routes::create_route),
        )
        .route(
            "/admin/v1/routes/preview",
            get(crate::handlers::routes::preview_route),
        )
        .route(
            "/admin/v1/routes/{id}",
            put(crate::handlers::routes::update_route)
                .delete(crate::handlers::routes::delete_route),
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
        .route(
            "/admin/v1/accounts/{id}/connection",
            post(crate::handlers::oauth::enable_account),
        )
        .route(
            "/admin/v1/config/export",
            get(crate::handlers::config::export_config),
        )
        .route("/admin/v1/stats", get(crate::handlers::stats::get_stats))
        .route("/metrics", get(crate::handlers::stats::get_metrics))
        .route(
            "/admin/v1/catalog/{provider}",
            get(crate::handlers::catalog::get_catalog).put(crate::handlers::catalog::put_catalog),
        )
        .route(
            "/admin/v1/catalog/{provider}/import-url",
            post(crate::handlers::catalog::import_catalog_url),
        )
        .with_state(state)
}

#[cfg(test)]
mod auth_tests {
    use super::*;
    use axum::body::Body;
    use http::{Method, Request, StatusCode};
    use tower::ServiceExt;
    use vkdg_config::ConfigSnapshot;

    fn state() -> AdminState {
        let (_tx, rx) = tokio::sync::watch::channel(Arc::new(ConfigSnapshot::default_empty()));
        let dir = std::env::temp_dir().join(format!("vkdg-authz-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        AdminState {
            sessions: SessionStore::new("t".into()),
            config_rx: rx,
            started_at: Arc::new(Instant::now()),
            key_store: Arc::new(vkdg_governance::VirtualKeyStore::in_memory().unwrap()),
            request_log: RequestLog::new(),
            combos: None,
            catalog: None,
            logins: None,
            reload_plugins: None,
            connection_tester: None,
            // A real store, so a missing auth check would reach it and succeed.
            gateway_store: Some(Arc::new(GatewayStore::open(&dir.join("gw.db")).unwrap())),
            config_tx: None,
        }
    }

    // Refutes: a handler that forgets `get_session` (each handler checks it
    // itself; there is no router-wide layer). Every route not in PUBLIC must
    // answer 401 with no cookie, including ones that would otherwise succeed.
    #[tokio::test]
    async fn every_non_public_route_requires_a_session() {
        const PUBLIC: &[(&str, &str)] = &[
            ("GET", "/admin/v1/system"),
            ("POST", "/admin/v1/session"),
            ("DELETE", "/admin/v1/session"),
            ("GET", "/admin/v1/setup"),
            ("GET", "/metrics"),
        ];
        let routes: &[(&str, &str, &str)] = &[
            ("GET", "/admin/v1/session/me", ""),
            (
                "POST",
                "/admin/v1/setup",
                r#"{"password":"xxxxxxxxxxxxxxxx"}"#,
            ),
            ("GET", "/admin/v1/connections", ""),
            ("GET", "/admin/v1/keys", ""),
            ("GET", "/admin/v1/routes", ""),
            ("GET", "/admin/v1/requests", ""),
            ("GET", "/admin/v1/plugins", ""),
            ("DELETE", "/admin/v1/plugins/x", ""),
            ("GET", "/admin/v1/stats", ""),
            ("GET", "/admin/v1/config/export", ""),
            ("GET", "/admin/v1/catalog/codex", ""),
            ("PUT", "/admin/v1/catalog/codex", r#"{"models":[]}"#),
            (
                "POST",
                "/admin/v1/catalog/codex/import-url",
                r#"{"url":"http://127.0.0.1:1/"}"#,
            ),
        ];
        let app = build_admin_router(state());
        let mut open = Vec::new();
        for (m, path, body) in routes {
            assert!(!PUBLIC.contains(&(*m, *path)));
            let req = Request::builder()
                .method(Method::from_bytes(m.as_bytes()).unwrap())
                .uri(*path)
                .header("content-type", "application/json")
                .body(Body::from(*body))
                .unwrap();
            let status = app.clone().oneshot(req).await.unwrap().status();
            if status != StatusCode::UNAUTHORIZED {
                open.push(format!("{m} {path} -> {status}"));
            }
        }
        assert!(
            open.is_empty(),
            "reachable without a session:\n{}",
            open.join("\n")
        );
    }
}
