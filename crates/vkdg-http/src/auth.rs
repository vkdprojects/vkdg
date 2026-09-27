//! Data-plane client authentication.
//!
//! Every `/v1/*` route is wrapped once by [`require_api_key`]. It accepts the
//! key as `x-api-key` (Anthropic clients) or `Authorization: Bearer` (OpenAI
//! clients), checks the endpoint's scope, and puts the caller's identity in the
//! request extensions as [`ClientIdentity`] for the ingress handlers.
//!
//! It fails closed: no store configured, no key, an unknown or revoked key, and
//! a storage error all reject the request. The only way to serve without keys is
//! [`DataAuth::disabled`], an explicit operator opt-out.

use std::sync::Arc;

use axum::{
    extract::{Request, State},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use http::{HeaderMap, StatusCode};
use serde_json::json;
use vkdg_core::VkdgError;
use vkdg_governance::{KeyScope, VirtualKeyStore};

/// Who is calling, as established by [`require_api_key`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientIdentity {
    /// Virtual key id; becomes the envelope's `client_id`.
    pub key_id: String,
    pub tenant_id: String,
    /// Client address from the socket, or from `X-Forwarded-For` only when the
    /// socket peer is a trusted proxy. See [`crate::resolve_client_ip`].
    pub client_ip: Option<std::net::IpAddr>,
    /// Model patterns the key may call; empty = every model.
    pub allowed_models: Arc<[String]>,
}

impl ClientIdentity {
    /// Identity used only when auth is explicitly disabled.
    fn anonymous(client_ip: Option<std::net::IpAddr>) -> Self {
        Self {
            key_id: "anonymous".into(),
            tenant_id: "default".into(),
            client_ip,
            allowed_models: Arc::from([]),
        }
    }

    /// Check the requested model against the key's list. Ingress calls this once
    /// the body is decoded: the model is not known before that.
    pub fn check_model(&self, model: &str) -> Result<(), VkdgError> {
        if self.allowed_models.is_empty()
            || vkdg_core::glob::matches_any(&self.allowed_models, model)
        {
            Ok(())
        } else {
            Err(VkdgError::Unauthorized)
        }
    }
}

/// Data-plane auth policy, shared by every `/v1/*` route.
#[derive(Clone)]
pub struct DataAuth {
    store: Option<Arc<VirtualKeyStore>>,
    /// Proxies whose `X-Forwarded-For` is believed.
    trusted_proxies: Arc<[vkdg_core::net::IpNet]>,
}

impl DataAuth {
    /// Require a valid key from `store` on every request.
    pub fn required(store: Arc<VirtualKeyStore>) -> Self {
        Self {
            store: Some(store),
            trusted_proxies: Arc::from([]),
        }
    }

    /// Serve without client keys. Operator opt-out only; never a default.
    pub fn disabled() -> Self {
        Self {
            store: None,
            trusted_proxies: Arc::from([]),
        }
    }

    /// Believe `X-Forwarded-For` from these peers (e.g. the local nginx).
    #[must_use]
    pub fn with_trusted_proxies(mut self, proxies: Vec<vkdg_core::net::IpNet>) -> Self {
        self.trusted_proxies = proxies.into();
        self
    }

    pub fn is_disabled(&self) -> bool {
        self.store.is_none()
    }
}

/// Axum middleware: authenticate the client or reject the request.
pub async fn require_api_key(
    State(auth): State<DataAuth>,
    mut req: Request,
    next: Next,
) -> Response {
    let path = req.uri().path().to_owned();
    // Present when served with `into_make_service_with_connect_info`.
    let peer = req
        .extensions()
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
        .map(|c| c.0.ip());
    let client_ip = crate::resolve_client_ip(peer, req.headers(), &auth.trusted_proxies);
    let Some(store) = &auth.store else {
        req.extensions_mut()
            .insert(ClientIdentity::anonymous(client_ip));
        return next.run(req).await;
    };

    let Some(token) = presented_token(req.headers()) else {
        return reject(
            &path,
            StatusCode::UNAUTHORIZED,
            "missing API key: send `x-api-key: <key>` or `Authorization: Bearer <key>`. \
             Create one with `vkdg keys create <name>` or in the console under Keys",
        );
    };

    let key = match store.authenticate(token) {
        Ok(Some(key)) => key,
        Ok(None) => {
            return reject(
                &path,
                StatusCode::UNAUTHORIZED,
                "invalid or revoked API key",
            );
        }
        Err(e) => {
            tracing::error!(error = %e, "key store lookup failed; rejecting request");
            return reject(
                &path,
                StatusCode::SERVICE_UNAVAILABLE,
                "API key store unavailable",
            );
        }
    };

    if !key.permits_ip(client_ip) {
        return reject(
            &path,
            StatusCode::FORBIDDEN,
            "this API key is not allowed from this address",
        );
    }

    let scope = scope_for(&path);
    if !key.allows(scope) {
        return reject(
            &path,
            StatusCode::FORBIDDEN,
            "this API key is not allowed to call this endpoint",
        );
    }

    req.extensions_mut().insert(ClientIdentity {
        key_id: key.id.0,
        tenant_id: key.tenant_id,
        client_ip,
        allowed_models: key.allowed_models.into(),
    });
    next.run(req).await
}

/// The key a client presented. `x-api-key` wins: Anthropic SDKs send it and some
/// also send an unrelated `Authorization` header.
fn presented_token(headers: &HeaderMap) -> Option<&str> {
    if let Some(v) = headers.get("x-api-key").and_then(|v| v.to_str().ok()) {
        let v = v.trim();
        if !v.is_empty() {
            return Some(v);
        }
    }
    let auth = headers.get(http::header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = auth.trim().split_once(' ')?;
    let token = token.trim();
    (scheme.eq_ignore_ascii_case("bearer") && !token.is_empty()).then_some(token)
}

fn scope_for(path: &str) -> KeyScope {
    if path.starts_with("/v1/images") {
        KeyScope::DataImage
    } else {
        KeyScope::DataInference
    }
}

/// An error body in the wire format the client speaks, so SDKs surface the
/// message instead of a parse error.
fn reject(path: &str, status: StatusCode, message: &str) -> Response {
    let kind = match status {
        StatusCode::FORBIDDEN => "permission_error",
        StatusCode::SERVICE_UNAVAILABLE => "api_error",
        _ => "authentication_error",
    };
    let body = if path.starts_with("/v1/messages") {
        json!({ "type": "error", "error": { "type": kind, "message": message } })
    } else {
        let code = match status {
            StatusCode::UNAUTHORIZED => "invalid_api_key",
            StatusCode::FORBIDDEN => "insufficient_scope",
            _ => "unavailable",
        };
        json!({ "error": { "message": message, "type": kind, "code": code } })
    };
    (status, Json(body)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{routing::post, Extension, Router};
    use tower::ServiceExt;

    async fn echo_identity(Extension(id): Extension<ClientIdentity>) -> String {
        format!("{}|{}", id.key_id, id.tenant_id)
    }

    fn app(auth: DataAuth) -> Router {
        Router::new()
            .route("/v1/messages", post(echo_identity))
            .route("/v1/chat/completions", post(echo_identity))
            .route("/v1/images/generations", post(echo_identity))
            .route_layer(axum::middleware::from_fn_with_state(auth, require_api_key))
    }

    async fn call(app: Router, path: &str, headers: &[(&str, &str)]) -> (StatusCode, String) {
        let mut req = http::Request::post(path);
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        let resp = app
            .oneshot(req.body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();
        let status = resp.status();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, String::from_utf8(body.to_vec()).unwrap())
    }

    fn store() -> Arc<VirtualKeyStore> {
        Arc::new(VirtualKeyStore::in_memory().unwrap())
    }

    // The incident: /v1/* answered without any key and spent the live account.
    #[tokio::test]
    async fn keyless_request_is_rejected_on_every_endpoint() {
        let app = app(DataAuth::required(store()));
        for path in [
            "/v1/messages",
            "/v1/chat/completions",
            "/v1/images/generations",
        ] {
            let (status, body) = call(app.clone(), path, &[]).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{path}: {body}");
            assert!(body.contains("vkdg keys create"), "{path}: {body}");
            // Each client SDK must parse the error in its own wire format.
            let v: serde_json::Value = serde_json::from_str(&body).unwrap();
            if path == "/v1/messages" {
                assert_eq!(v["type"], "error", "{body}");
                assert_eq!(v["error"]["type"], "authentication_error", "{body}");
            } else {
                assert_eq!(v["error"]["code"], "invalid_api_key", "{body}");
                assert!(v.get("type").is_none(), "{body}");
            }
        }
    }

    // Plausible wrong impl: an empty store (fresh install) means "auth off".
    #[tokio::test]
    async fn empty_store_still_fails_closed() {
        let (status, _) = call(
            app(DataAuth::required(store())),
            "/v1/messages",
            &[("x-api-key", "vkdg_guess")],
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn both_header_styles_authenticate_and_carry_identity() {
        let store = store();
        let (key, raw) = store
            .create({
                let mut n = vkdg_governance::NewKey::named("ci");
                n.tenant_id = "acme".into();
                n
            })
            .unwrap();
        let app = app(DataAuth::required(store));
        let expected = format!("{}|acme", key.id.0);

        let (s, body) = call(app.clone(), "/v1/messages", &[("x-api-key", &raw)]).await;
        assert_eq!((s, body.as_str()), (StatusCode::OK, expected.as_str()));

        let bearer = format!("Bearer {raw}");
        let (s, body) = call(
            app.clone(),
            "/v1/chat/completions",
            &[("authorization", &bearer)],
        )
        .await;
        assert_eq!((s, body.as_str()), (StatusCode::OK, expected.as_str()));

        // Basic auth or a bare token in Authorization is not a bearer key.
        let (s, _) = call(app, "/v1/chat/completions", &[("authorization", &raw)]).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn revoked_key_is_rejected() {
        let store = store();
        let (key, raw) = store.create(vkdg_governance::NewKey::named("ci")).unwrap();
        store.revoke(&key.id).unwrap();
        let (s, body) = call(
            app(DataAuth::required(store)),
            "/v1/messages",
            &[("x-api-key", &raw)],
        )
        .await;
        assert_eq!(s, StatusCode::UNAUTHORIZED);
        assert!(
            body.contains("\"type\":\"error\""),
            "Anthropic error shape: {body}"
        );
    }

    // Plausible wrong impl: scopes are stored but never checked.
    #[tokio::test]
    async fn key_without_image_scope_cannot_generate_images() {
        let store = store();
        let (_key, raw) = store
            .create({
                let mut n = vkdg_governance::NewKey::named("chat-only");
                n.scopes = vec![KeyScope::DataInference];
                n
            })
            .unwrap();
        let app = app(DataAuth::required(store));
        let bearer = format!("Bearer {raw}");
        let (s, body) = call(
            app.clone(),
            "/v1/images/generations",
            &[("authorization", &bearer)],
        )
        .await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{body}");
        assert!(
            body.contains("insufficient_scope"),
            "OpenAI error shape: {body}"
        );
        let (s, _) = call(app, "/v1/chat/completions", &[("authorization", &bearer)]).await;
        assert_eq!(s, StatusCode::OK);
    }

    #[tokio::test]
    async fn explicit_opt_out_serves_anonymously() {
        let (s, body) = call(app(DataAuth::disabled()), "/v1/messages", &[]).await;
        assert_eq!((s, body.as_str()), (StatusCode::OK, "anonymous|default"));
    }

    // Plausible wrong impl: per-key IP list stored but never enforced.
    #[tokio::test]
    async fn key_used_outside_its_ip_list_is_forbidden() {
        let store = store();
        let mut spec = vkdg_governance::NewKey::named("office");
        spec.allowed_ips = vec!["10.0.0.0/8".parse().unwrap()];
        let (_k, raw) = store.create(spec).unwrap();
        // In-process calls have no socket address: the list cannot be satisfied.
        let (s, body) = call(
            app(DataAuth::required(store)),
            "/v1/messages",
            &[("x-api-key", &raw)],
        )
        .await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{body}");
    }

    #[test]
    fn model_outside_the_key_list_is_rejected() {
        let id = ClientIdentity {
            key_id: "k".into(),
            tenant_id: "t".into(),
            client_ip: None,
            allowed_models: Arc::from(["claude-*".to_owned()]),
        };
        assert!(id.check_model("claude-sonnet-4.5").is_ok());
        assert!(matches!(
            id.check_model("gpt-4o"),
            Err(VkdgError::Unauthorized)
        ));
    }
}
