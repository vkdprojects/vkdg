//! `POST /admin/v1/connections/{id}/models/sync`: replace a connection's model
//! patterns with the concrete ids its provider reports for the connection's account.
//!
//! Connections ship with route patterns (`claude-*`) that `/v1/models` cannot list.
//! This asks the provider which models the account may call and stores exactly
//! those ids, so clients see them and eligibility can match them.

use axum::{
    extract::{Path, State},
    response::{IntoResponse, Response},
    Json,
};
use http::{HeaderMap, StatusCode};
use vkdg_connections::{AuthKind, ConnectionConfig, Credential};

use crate::{
    error::{AdminError, AdminErrorResponse},
    handlers::{connections::rebuild_and_push, oauth::LoginService, session::get_session},
    router::AdminState,
};

/// Longest upstream explanation echoed back to the console.
const MAX_REASON_CHARS: usize = 200;

fn fail(status: StatusCode, code: &str, message: impl Into<String>) -> Response {
    AdminErrorResponse(status, AdminError::new(code, message)).into_response()
}

fn sync_failed(message: impl AsRef<str>) -> Response {
    let reason: String = message.as_ref().chars().take(MAX_REASON_CHARS).collect();
    fail(StatusCode::BAD_GATEWAY, "model_sync_failed", reason)
}

/// The account's own credential, refreshed when it is about to expire.
async fn credential_for(svc: &LoginService, conn: &ConnectionConfig) -> Result<Credential, String> {
    if let Some(credentials) = &svc.credentials {
        return credentials
            .get_token(conn)
            .await
            .map_err(|e| format!("no usable credential for connection {}: {e}", conn.id.0));
    }
    match &conn.auth {
        AuthKind::Account { account_id } => match svc.store.get(account_id) {
            Ok(Some(account)) => Ok(account.credential()),
            Ok(None) => Err(format!("account {account_id} not found")),
            Err(e) => Err(format!("account store: {e}")),
        },
        _ => Err("no credential manager is configured".to_owned()),
    }
}

/// Discovered ids as the connection's model list: trimmed, concrete, sorted, unique.
fn concrete_sorted(discovered: Vec<String>) -> Vec<String> {
    let mut models: Vec<String> = discovered
        .into_iter()
        .map(|m| m.trim().to_owned())
        .filter(|m| !m.is_empty() && !m.contains(['*', '?']))
        .collect();
    models.sort();
    models.dedup();
    models
}

pub async fn sync_models(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if get_session(&state, &headers).is_none() {
        return AdminErrorResponse(StatusCode::UNAUTHORIZED, AdminError::unauthorized())
            .into_response();
    }
    let snapshot = state.config_rx.borrow().clone();
    let Some(conn) = snapshot.connections.iter().find(|c| c.id.0 == id) else {
        return AdminErrorResponse(StatusCode::NOT_FOUND, AdminError::not_found(&id))
            .into_response();
    };
    let Some(store) = &state.gateway_store else {
        return fail(
            StatusCode::SERVICE_UNAVAILABLE,
            "no_store",
            "gateway store not available",
        );
    };
    let Some(svc) = state.logins.clone() else {
        return fail(
            StatusCode::SERVICE_UNAVAILABLE,
            "accounts_disabled",
            "provider registry is not configured",
        );
    };
    let provider = conn.provider.adapter_id();
    let Some(adapter) = svc.registry.get(provider) else {
        return fail(
            StatusCode::UNPROCESSABLE_ENTITY,
            "provider_unavailable",
            format!("provider '{provider}' is not installed"),
        );
    };
    let Some(catalog) = adapter.model_catalog() else {
        return fail(
            StatusCode::UNPROCESSABLE_ENTITY,
            "no_model_catalog",
            format!(
                "{} cannot list models; edit the connection's models by hand",
                adapter.display_name()
            ),
        );
    };

    let credential = match credential_for(&svc, conn).await {
        Ok(c) => c,
        Err(reason) => return sync_failed(reason),
    };
    let discovered = match catalog.list_models(conn, &credential).await {
        Ok(models) => concrete_sorted(models),
        Err(e) => return sync_failed(format!("{} refused the model list: {e}", adapter.id())),
    };
    // An empty answer is never trusted: it would leave the connection with nothing
    // to serve, and a transient upstream quirk would silently unroute the account.
    if discovered.is_empty() {
        return sync_failed(format!(
            "{} reported no models; the connection was left unchanged",
            adapter.id()
        ));
    }

    let (defs, _) = match store.load() {
        Ok(v) => v,
        Err(e) => {
            return fail(
                StatusCode::INTERNAL_SERVER_ERROR,
                "store_error",
                e.to_string(),
            )
        }
    };
    let Some(old) = defs.into_iter().find(|c| c.id == id) else {
        return AdminErrorResponse(StatusCode::NOT_FOUND, AdminError::not_found(&id))
            .into_response();
    };
    let mut updated = old.clone();
    updated.models.clone_from(&discovered);
    if let Err(e) = store.upsert_connection(&updated) {
        return fail(
            StatusCode::INTERNAL_SERVER_ERROR,
            "store_error",
            e.to_string(),
        );
    }
    if let Err(e) = rebuild_and_push(&state) {
        // Keep the stored def the snapshot accepted.
        let _ = store.upsert_connection(&old);
        return AdminErrorResponse(StatusCode::UNPROCESSABLE_ENTITY, e).into_response();
    }
    let count = discovered.len();
    Json(serde_json::json!({ "models": discovered, "count": count })).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handlers::requests::RequestLog;
    use crate::session::SessionStore;
    use futures::future::BoxFuture;
    use parking_lot::Mutex;
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::time::Instant;
    use vkdg_config::schema::{AuthDef, ConnectionDef};
    use vkdg_config::{ConfigSnapshot, GatewayStore};
    use vkdg_connections::{Account, AccountStore, TokenPair};
    use vkdg_operations::Operation;
    use vkdg_provider_sdk::{
        ModelCatalog, PreparedRequest, ProviderAdapter, ProviderError, ProviderRegistry,
    };

    /// What the fake upstream answers.
    enum Script {
        Models(Vec<&'static str>),
        Fail(&'static str),
    }

    /// Provider with a catalog; records the token each listing used.
    struct WithCatalog {
        script: Script,
        seen_tokens: Arc<Mutex<Vec<String>>>,
    }

    impl ProviderAdapter for WithCatalog {
        fn id(&self) -> &'static str {
            "fake"
        }
        fn display_name(&self) -> &'static str {
            "Fake"
        }
        fn prepare(
            &self,
            _: &Operation,
            _: &ConnectionConfig,
            _: &Credential,
        ) -> Result<PreparedRequest, ProviderError> {
            Err(ProviderError::UnsupportedOperation)
        }
        fn model_catalog(&self) -> Option<&dyn ModelCatalog> {
            Some(self)
        }
    }

    impl ModelCatalog for WithCatalog {
        fn list_models<'a>(
            &'a self,
            _: &'a ConnectionConfig,
            credential: &'a Credential,
        ) -> BoxFuture<'a, Result<Vec<String>, ProviderError>> {
            Box::pin(async move {
                self.seen_tokens.lock().push(credential.token.clone());
                match &self.script {
                    Script::Models(m) => Ok(m.iter().map(|s| (*s).to_owned()).collect()),
                    Script::Fail(why) => Err(ProviderError::Http((*why).to_owned())),
                }
            })
        }
    }

    /// Provider that cannot list models.
    struct NoCatalog;

    impl ProviderAdapter for NoCatalog {
        fn id(&self) -> &'static str {
            "plain"
        }
        fn display_name(&self) -> &'static str {
            "Plain"
        }
        fn prepare(
            &self,
            _: &Operation,
            _: &ConnectionConfig,
            _: &Credential,
        ) -> Result<PreparedRequest, ProviderError> {
            Err(ProviderError::UnsupportedOperation)
        }
    }

    struct Fixture {
        state: AdminState,
        store: Arc<GatewayStore>,
        seen_tokens: Arc<Mutex<Vec<String>>>,
    }

    /// Two account connections (`c-fake` with a catalog, `c-plain` without), each
    /// declaring only the glob `claude-*` plus non-default weight/limits/tags that
    /// a sync must leave alone.
    fn fixture(script: Script) -> Fixture {
        let seen_tokens = Arc::new(Mutex::new(Vec::new()));
        let mut registry = ProviderRegistry::empty();
        registry.register(Arc::new(WithCatalog {
            script,
            seen_tokens: Arc::clone(&seen_tokens),
        }));
        registry.register(Arc::new(NoCatalog));

        let accounts = Arc::new(AccountStore::in_memory().unwrap());
        let mut account_ids = HashMap::new();
        for (provider, token) in [("fake", "acct-token"), ("plain", "other-token")] {
            let account = Account::from_token_pair(
                provider,
                provider,
                TokenPair {
                    access_token: token.into(),
                    refresh_token: None,
                    expires_in_secs: None,
                    extra: HashMap::new(),
                },
            );
            accounts.upsert(&account).unwrap();
            account_ids.insert(provider, account.id);
        }

        let store = Arc::new(GatewayStore::in_memory().unwrap());
        for (id, provider) in [("c-fake", "fake"), ("c-plain", "plain")] {
            store
                .upsert_connection(&ConnectionDef {
                    id: id.into(),
                    provider: provider.into(),
                    base_url: None,
                    endpoint: None,
                    auth: AuthDef::Account {
                        account: account_ids[provider].clone(),
                    },
                    models: vec!["claude-*".into()],
                    max_concurrent: Some(7),
                    weight: Some(3),
                    tags: vec!["keep".into()],
                })
                .unwrap();
        }

        let (tx, rx) = tokio::sync::watch::channel(Arc::new(ConfigSnapshot::default_empty()));
        let state = AdminState {
            sessions: SessionStore::new("tok".into()),
            config_rx: rx,
            started_at: Arc::new(Instant::now()),
            key_store: Arc::new(vkdg_governance::VirtualKeyStore::in_memory().unwrap()),
            request_log: RequestLog::new(),
            combos: None,
            reload_plugins: None,
            connection_tester: None,
            catalog: None,
            logins: Some(LoginService::new(Arc::new(registry), accounts, None)),
            gateway_store: Some(Arc::clone(&store)),
            config_tx: Some(tx),
        };
        rebuild_and_push(&state).unwrap();
        Fixture {
            state,
            store,
            seen_tokens,
        }
    }

    fn authed(state: &AdminState) -> HeaderMap {
        let session = state.sessions.bootstrap_login().unwrap();
        let mut h = HeaderMap::new();
        h.insert(
            http::header::COOKIE,
            http::HeaderValue::from_str(&format!("vkdg_session={}", session.session_id)).unwrap(),
        );
        h
    }

    async fn sync(f: &Fixture, id: &str) -> (StatusCode, serde_json::Value) {
        let resp = sync_models(
            State(f.state.clone()),
            authed(&f.state),
            Path(id.to_owned()),
        )
        .await;
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or_default())
    }

    fn stored(f: &Fixture, id: &str) -> ConnectionDef {
        f.store
            .load()
            .unwrap()
            .0
            .into_iter()
            .find(|c| c.id == id)
            .unwrap()
    }

    fn live_models(f: &Fixture, id: &str) -> Vec<String> {
        f.state
            .config_rx
            .borrow()
            .connections
            .iter()
            .find(|c| c.id.0 == id)
            .unwrap()
            .models
            .clone()
    }

    // Wrong impls: appends to the declared glob instead of replacing; skips
    // sorting/dedup/blank filtering; writes the store but not the live snapshot
    // (clients keep seeing the old list); rebuilds the def and loses weight,
    // limits, endpoint, tags or auth; lists with some other account's token.
    #[tokio::test]
    async fn sync_replaces_models_everywhere_and_touches_nothing_else() {
        let f = fixture(Script::Models(vec![
            "claude-sonnet-4.6",
            "auto",
            "claude-haiku-4.5",
            "auto",
            "  ",
            "claude-*",
        ]));
        let (status, v) = sync(&f, "c-fake").await;

        assert_eq!(status, StatusCode::OK, "{v}");
        let expected = ["auto", "claude-haiku-4.5", "claude-sonnet-4.6"];
        assert_eq!(v["models"], serde_json::json!(expected));
        assert_eq!(v["count"], 3);

        let def = stored(&f, "c-fake");
        assert_eq!(def.models, expected);
        assert_eq!(live_models(&f, "c-fake"), expected);
        assert_eq!((def.weight, def.max_concurrent), (Some(3), Some(7)));
        assert_eq!(def.endpoint, None);
        assert_eq!(def.tags, ["keep"]);
        assert!(matches!(&def.auth, AuthDef::Account { .. }));

        assert_eq!(*f.seen_tokens.lock(), ["acct-token"]);
        // The sibling connection is not part of this sync.
        assert_eq!(stored(&f, "c-plain").models, ["claude-*"]);
    }

    // Wrong impls: an empty (or all-blank/glob) answer wipes the connection's
    // models, leaving it unroutable; an upstream error still writes something.
    #[tokio::test]
    async fn empty_or_failed_discovery_leaves_the_connection_unchanged() {
        for script in [
            Script::Models(vec![]),
            Script::Models(vec![" ", "*"]),
            Script::Fail("403 forbidden"),
        ] {
            let f = fixture(script);
            let version = f.state.config_rx.borrow().version;
            let (status, v) = sync(&f, "c-fake").await;

            assert_eq!(status, StatusCode::BAD_GATEWAY, "{v}");
            assert_eq!(v["code"], "model_sync_failed", "{v}");
            assert_eq!(stored(&f, "c-fake").models, ["claude-*"]);
            assert_eq!(live_models(&f, "c-fake"), ["claude-*"]);
            assert_eq!(
                f.state.config_rx.borrow().version,
                version,
                "no snapshot push"
            );
        }
    }

    // Wrong impls: a provider that cannot list models is treated as "empty" (502)
    // or silently succeeds; the connection is modified anyway.
    #[tokio::test]
    async fn provider_without_a_catalog_is_422_and_unchanged() {
        let f = fixture(Script::Models(vec!["x"]));
        let (status, v) = sync(&f, "c-plain").await;

        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
        assert_eq!(v["code"], "no_model_catalog", "{v}");
        assert_eq!(stored(&f, "c-plain").models, ["claude-*"]);
        assert!(f.seen_tokens.lock().is_empty());
    }

    // Wrong impls: unknown id 500s or creates a connection; no session reaches
    // the provider with an account credential.
    #[tokio::test]
    async fn unknown_connection_is_404_and_a_session_is_required() {
        let f = fixture(Script::Models(vec!["x"]));
        let (status, v) = sync(&f, "nope").await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{v}");

        let resp = sync_models(
            State(f.state.clone()),
            HeaderMap::new(),
            Path("c-fake".into()),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        assert!(f.seen_tokens.lock().is_empty());
        assert_eq!(stored(&f, "c-fake").models, ["claude-*"]);
    }
}
