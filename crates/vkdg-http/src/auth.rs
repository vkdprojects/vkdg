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

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use axum::{
    extract::{Request, State},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use futures::Stream;
use http::{HeaderMap, StatusCode};
use serde_json::json;
use vkdg_core::VkdgError;
use vkdg_governance::{KeyScope, VirtualKeyId, VirtualKeyStore};

use crate::metering::UsageMeter;
use crate::pipeline::PendingLog;

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
    /// Per-key request counters for `requests_per_minute`. In memory on
    /// purpose: a rate limit is about the last minute, not durable history.
    rate: Arc<RateWindows>,
}

/// Fixed one-minute windows per key. Only authenticated key ids are ever
/// inserted, so the map is bounded by the number of keys.
#[derive(Default)]
struct RateWindows(parking_lot::Mutex<HashMap<String, (Instant, u32)>>);

impl RateWindows {
    const WINDOW: Duration = Duration::from_secs(60);

    /// Count one request against `limit`. When the window is full, returns how
    /// long until it reopens.
    fn admit(&self, key_id: &str, limit: u32) -> Result<(), Duration> {
        let now = Instant::now();
        let mut windows = self.0.lock();
        let (start, count) = windows.entry(key_id.to_owned()).or_insert((now, 0));
        if now.duration_since(*start) >= Self::WINDOW {
            (*start, *count) = (now, 0);
        }
        if *count >= limit {
            return Err(Self::WINDOW.saturating_sub(now.duration_since(*start)));
        }
        *count += 1;
        Ok(())
    }
}

impl DataAuth {
    /// Require a valid key from `store` on every request.
    pub fn required(store: Arc<VirtualKeyStore>) -> Self {
        Self {
            store: Some(store),
            trusted_proxies: Arc::from([]),
            rate: Arc::default(),
        }
    }

    /// Serve without client keys. Operator opt-out only; never a default.
    pub fn disabled() -> Self {
        Self {
            store: None,
            trusted_proxies: Arc::from([]),
            rate: Arc::default(),
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
        return metered(next.run(req).await, None, true);
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

    // Budget before the upstream is ever called.
    if let Some(limit) = key.monthly_token_limit {
        match store.usage_this_month(&key.id) {
            Ok(used) if used.tokens() >= limit => {
                return with_retry_after(
                    reject_as(
                        &path,
                        StatusCode::TOO_MANY_REQUESTS,
                        "rate_limit_error",
                        "insufficient_quota",
                        "this API key has used its monthly token budget",
                    ),
                    until_next_month(),
                );
            }
            Ok(_) => {}
            Err(e) => {
                tracing::error!(error = %e, "key usage lookup failed; rejecting request");
                return reject(
                    &path,
                    StatusCode::SERVICE_UNAVAILABLE,
                    "API key store unavailable",
                );
            }
        }
    }
    if let Some(rpm) = key.requests_per_minute {
        if let Err(wait) = auth.rate.admit(&key.id.0, rpm) {
            return with_retry_after(
                reject_as(
                    &path,
                    StatusCode::TOO_MANY_REQUESTS,
                    "rate_limit_error",
                    "rate_limit_exceeded",
                    "this API key is over its requests-per-minute limit",
                ),
                wait,
            );
        }
    }

    let key_id = key.id.clone();
    let log_history = !key.no_log;
    req.extensions_mut().insert(ClientIdentity {
        key_id: key.id.0,
        tenant_id: key.tenant_id,
        client_ip,
        allowed_models: key.allowed_models.into(),
    });
    let response = next.run(req).await;
    metered(response, Some((Arc::clone(store), key_id)), log_history)
}

/// `retry-after` in whole seconds, at least 1, so clients back off instead of
/// retrying at once.
fn with_retry_after(mut response: Response, wait: Duration) -> Response {
    let secs = wait.as_secs() + u64::from(wait.subsec_nanos() > 0);
    if let Ok(v) = http::HeaderValue::from_str(&secs.max(1).to_string()) {
        response.headers_mut().insert(http::header::RETRY_AFTER, v);
    }
    response
}

/// Time until the monthly budget window (calendar month, UTC) resets.
fn until_next_month() -> Duration {
    use chrono::{Datelike, TimeZone, Utc};
    let now = Utc::now();
    let (y, m) = if now.month() == 12 {
        (now.year() + 1, 1)
    } else {
        (now.year(), now.month() + 1)
    };
    Utc.with_ymd_and_hms(y, m, 1, 0, 0, 0)
        .single()
        .and_then(|next| (next - now).to_std().ok())
        .unwrap_or(Duration::from_secs(3600))
}

/// Settle the response once its body is done (at the end of the stream, or
/// when the client hangs up, so a disconnect mid-stream is still charged for
/// what was sent): count its tokens against the key and complete its request
/// history row. The row is written here, when headers leave, and never for a
/// `no_log` key.
fn metered(
    mut response: Response,
    charge: Option<(Arc<VirtualKeyStore>, VirtualKeyId)>,
    log_history: bool,
) -> Response {
    let history = response
        .extensions_mut()
        .remove::<PendingLog>()
        .filter(|_| log_history)
        .map(|p| {
            let id = p.record.request_id.clone();
            p.log.push(p.record);
            (p.log, id, p.price)
        });
    if charge.is_none() && history.is_none() {
        return response;
    }
    let (parts, body) = response.into_parts();
    let stream = Metered {
        inner: body.into_data_stream(),
        meter: UsageMeter::default(),
        settle: Some(Settle { charge, history }),
        ended: false,
    };
    Response::from_parts(parts, axum::body::Body::from_stream(stream))
}

struct Settle {
    charge: Option<(Arc<VirtualKeyStore>, VirtualKeyId)>,
    history: Option<(
        Arc<vkdg_admin::handlers::requests::RequestLog>,
        String,
        Option<vkdg_core::pricing::ModelPrice>,
    )>,
}

struct Metered {
    inner: axum::body::BodyDataStream,
    meter: UsageMeter,
    /// Taken on the first of end-of-stream or drop, so it runs once.
    settle: Option<Settle>,
    /// The body ran to its end (as opposed to the client hanging up).
    ended: bool,
}

impl Metered {
    fn record(&mut self) {
        let Some(Settle { charge, history }) = self.settle.take() else {
            return;
        };
        let usage = self.meter.finish();
        let ended = self.ended;
        let write = move || {
            if let Some((store, key_id)) = charge {
                if let Err(e) = store.record_usage(&key_id, usage.input, usage.output) {
                    tracing::error!(error = %e, key = %key_id.0, "failed to record key usage");
                }
            }
            if let Some((log, id, price)) = history {
                log.finish(&id, usage.billed(), ended, price.as_ref());
            }
        };
        // SQLite is blocking; keep it off the async workers.
        match tokio::runtime::Handle::try_current() {
            Ok(rt) => drop(rt.spawn_blocking(write)),
            Err(_) => write(),
        }
    }
}

impl Stream for Metered {
    type Item = Result<bytes::Bytes, axum::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let polled = Pin::new(&mut self.inner).poll_next(cx);
        match &polled {
            Poll::Ready(Some(Ok(chunk))) => self.meter.feed(chunk),
            Poll::Ready(None) => {
                self.ended = true;
                self.record();
            }
            _ => {}
        }
        polled
    }
}

impl Drop for Metered {
    fn drop(&mut self) {
        self.record();
    }
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
    let code = match status {
        StatusCode::UNAUTHORIZED => "invalid_api_key",
        StatusCode::FORBIDDEN => "insufficient_scope",
        _ => "unavailable",
    };
    reject_as(path, status, kind, code, message)
}

/// `kind` is the Anthropic `error.type`; `code` the OpenAI `error.code`.
fn reject_as(path: &str, status: StatusCode, kind: &str, code: &str, message: &str) -> Response {
    let body = if path.starts_with("/v1/messages") {
        json!({ "type": "error", "error": { "type": kind, "message": message } })
    } else {
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

    fn limited(
        f: impl FnOnce(&mut vkdg_governance::NewKey),
    ) -> (Arc<VirtualKeyStore>, vkdg_governance::VirtualKey, String) {
        let store = store();
        let mut spec = vkdg_governance::NewKey::named("limited");
        f(&mut spec);
        let (key, raw) = store.create(spec).unwrap();
        (store, key, raw)
    }

    // Plausible wrong impl: requests_per_minute stored but never enforced.
    #[tokio::test]
    async fn requests_over_the_per_minute_limit_get_429() {
        let (store, _k, raw) = limited(|s| s.requests_per_minute = Some(2));
        let app = app(DataAuth::required(store));
        for _ in 0..2 {
            let (s, _) = call(app.clone(), "/v1/messages", &[("x-api-key", &raw)]).await;
            assert_eq!(s, StatusCode::OK);
        }
        let resp = app
            .oneshot(
                http::Request::post("/v1/messages")
                    .header("x-api-key", &raw)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
        // Clients back off on retry-after; without it they hammer the gateway.
        let wait: u64 = resp.headers()["retry-after"]
            .to_str()
            .unwrap()
            .parse()
            .unwrap();
        assert!((1..=60).contains(&wait), "{wait}");
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["error"]["type"], "rate_limit_error", "{v}");
    }

    // The budget is checked before the request reaches the upstream.
    #[tokio::test]
    async fn key_over_its_monthly_budget_is_refused_before_the_handler() {
        let (store, key, raw) = limited(|s| s.monthly_token_limit = Some(100));
        store.record_usage(&key.id, 60, 40).unwrap();
        let (s, body) = call(
            app(DataAuth::required(store)),
            "/v1/chat/completions",
            &[("authorization", &format!("Bearer {raw}"))],
        )
        .await;
        assert_eq!(s, StatusCode::TOO_MANY_REQUESTS, "{body}");
        assert!(body.contains("insufficient_quota"), "{body}");
    }

    // Streaming is most traffic: its usage must be counted from the stream.
    #[tokio::test]
    async fn streamed_usage_is_recorded_against_the_key() {
        async fn sse() -> axum::response::Response {
            let chunks = [
                "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":11}}}\n\n",
                "event: message_delta\ndata: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":4}}\n\n",
            ];
            let stream = futures::stream::iter(
                chunks.map(|c| Ok::<_, std::io::Error>(bytes::Bytes::from(c))),
            );
            axum::response::Response::new(axum::body::Body::from_stream(stream))
        }
        let (store, key, raw) = limited(|_| {});
        let app = Router::new().route("/v1/messages", post(sse)).route_layer(
            axum::middleware::from_fn_with_state(
                DataAuth::required(Arc::clone(&store)),
                require_api_key,
            ),
        );
        let (s, _) = call(app, "/v1/messages", &[("x-api-key", &raw)]).await;
        assert_eq!(s, StatusCode::OK);
        // Recording happens when the body finishes; give the blocking write a moment.
        for _ in 0..50 {
            if store.usage_this_month(&key.id).unwrap().requests > 0 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let u = store.usage_this_month(&key.id).unwrap();
        assert_eq!((u.input_tokens, u.output_tokens, u.requests), (11, 4, 1));
    }

    fn pending(
        log: &Arc<vkdg_admin::handlers::requests::RequestLog>,
        id: &str,
        price: Option<vkdg_core::pricing::ModelPrice>,
    ) -> PendingLog {
        PendingLog {
            log: Arc::clone(log),
            record: vkdg_admin::handlers::requests::RequestRecord {
                request_id: id.into(),
                model: "m".into(),
                api_type: "anthropic".into(),
                status: vkdg_admin::handlers::requests::STATUS_PENDING.into(),
                connection_id: None,
                key_id: None,
                started_at_ms: chrono::Utc::now().timestamp_millis(),
                duration_ms: Some(0),
                decision: None,
                input_tokens: None,
                output_tokens: None,
                cost_microdollars: None,
            },
            price,
        }
    }

    /// A router whose handler streams an Anthropic usage pair and attaches the
    /// history row the way the pipeline does.
    fn logged_app(auth: DataAuth, log: &Arc<vkdg_admin::handlers::requests::RequestLog>) -> Router {
        priced_app(
            auth,
            log,
            Some(vkdg_core::pricing::ModelPrice::new(
                "m", 3_000_000, 15_000_000,
            )),
        )
    }

    fn priced_app(
        auth: DataAuth,
        log: &Arc<vkdg_admin::handlers::requests::RequestLog>,
        price: Option<vkdg_core::pricing::ModelPrice>,
    ) -> Router {
        let log = Arc::clone(log);
        let handler = move || {
            let (log, price) = (Arc::clone(&log), price.clone());
            async move {
                let chunks = [
                    "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":11}}}\n\n",
                    "event: message_delta\ndata: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":4}}\n\n",
                ];
                let stream = futures::stream::iter(
                    chunks.map(|c| Ok::<_, std::io::Error>(bytes::Bytes::from(c))),
                );
                let mut r = axum::response::Response::new(axum::body::Body::from_stream(stream));
                r.extensions_mut().insert(pending(&log, "req-1", price));
                r
            }
        };
        Router::new()
            .route("/v1/messages", post(handler))
            .route_layer(axum::middleware::from_fn_with_state(auth, require_api_key))
    }

    async fn settled(
        log: &vkdg_admin::handlers::requests::RequestLog,
        id: &str,
    ) -> Option<vkdg_admin::handlers::requests::RequestRecord> {
        for _ in 0..50 {
            match log.get(id) {
                Some(r) if r.status != vkdg_admin::handlers::requests::STATUS_PENDING => {
                    return Some(r)
                }
                _ => tokio::time::sleep(std::time::Duration::from_millis(10)).await,
            }
        }
        log.get(id)
    }

    // The history row was written when headers left: a stream always read
    // `completed` with no tokens.
    #[tokio::test]
    async fn streamed_history_row_gets_tokens_and_final_status() {
        let log = vkdg_admin::handlers::requests::RequestLog::new();
        let (store, _key, raw) = limited(|_| {});
        let (s, _) = call(
            logged_app(DataAuth::required(store), &log),
            "/v1/messages",
            &[("x-api-key", &raw)],
        )
        .await;
        assert_eq!(s, StatusCode::OK);
        let r = settled(&log, "req-1").await.expect("row written");
        assert_eq!(r.status, "completed");
        assert_eq!((r.input_tokens, r.output_tokens), (Some(11), Some(4)));
        assert_eq!(
            r.cost_microdollars,
            Some(93),
            "priced from the attached list price"
        );
    }

    // Auth off has no key to charge, but history must still settle.
    #[tokio::test]
    async fn anonymous_history_row_is_settled_too() {
        let log = vkdg_admin::handlers::requests::RequestLog::new();
        let (s, _) = call(logged_app(DataAuth::disabled(), &log), "/v1/messages", &[]).await;
        assert_eq!(s, StatusCode::OK);
        let r = settled(&log, "req-1").await.expect("row written");
        assert_eq!((r.status.as_str(), r.output_tokens), ("completed", Some(4)));
    }

    // A provider with no list price (Kiro, Claude Code) must read as unknown,
    // never as a $0 request.
    #[tokio::test]
    async fn unpriced_row_has_tokens_but_no_cost() {
        let log = vkdg_admin::handlers::requests::RequestLog::new();
        let (s, _) = call(
            priced_app(DataAuth::disabled(), &log, None),
            "/v1/messages",
            &[],
        )
        .await;
        assert_eq!(s, StatusCode::OK);
        let r = settled(&log, "req-1").await.expect("row written");
        assert_eq!((r.input_tokens, r.cost_microdollars), (Some(11), None));
    }

    #[tokio::test]
    async fn no_log_key_leaves_no_history_but_is_still_charged() {
        let log = vkdg_admin::handlers::requests::RequestLog::new();
        let (store, key, raw) = limited(|s| s.no_log = true);
        let (s, _) = call(
            logged_app(DataAuth::required(Arc::clone(&store)), &log),
            "/v1/messages",
            &[("x-api-key", &raw)],
        )
        .await;
        assert_eq!(s, StatusCode::OK);
        for _ in 0..50 {
            if store.usage_this_month(&key.id).unwrap().requests > 0 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert_eq!(store.usage_this_month(&key.id).unwrap().output_tokens, 4);
        assert!(log.get("req-1").is_none(), "no row, not even briefly");
    }

    // A client that hangs up mid-stream must not show as completed.
    #[tokio::test]
    async fn abandoned_stream_is_recorded_as_cancelled() {
        let log = vkdg_admin::handlers::requests::RequestLog::new();
        let (store, _key, raw) = limited(|_| {});
        let resp = logged_app(DataAuth::required(store), &log)
            .oneshot(
                http::Request::post("/v1/messages")
                    .header("x-api-key", &raw)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            log.get("req-1").map(|r| r.status),
            Some(vkdg_admin::handlers::requests::STATUS_PENDING.to_string()),
            "row exists while the body is in flight"
        );
        drop(resp);
        let r = settled(&log, "req-1").await.expect("row written");
        assert_eq!(r.status, "cancelled");
    }
}
