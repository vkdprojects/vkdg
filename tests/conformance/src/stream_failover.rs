//! Account failures that Kiro reports INSIDE an HTTP 200 event stream.
//!
//! Kiro answers 200 and sends "throttled" or "out of credits" as a frame of the
//! stream. The pipeline looks ahead to the first substantive event before it
//! commits the response: a throttling/quota first frame cools the connection and
//! moves the request to a sibling, before any byte reaches the client. Every
//! other shape (normal first event, a failure after a normal event, a failure that
//! is not an account failure) behaves as it always did.
//!
//! The fake upstreams emit real AWS event-stream frames (built with the official
//! encoder, as the Kiro plugin's own stream tests do) and decode them with the real
//! `KiroStreamDecoder`; only the transport is fake.

use std::collections::VecDeque;
use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use aws_smithy_eventstream::frame::write_message_to;
use aws_smithy_types::event_stream::{Header, HeaderValue as AwsValue, Message as AwsMessage};
use axum::body::Body;
use axum::extract::State;
use axum::response::Response;
use axum::routing::post;
use axum::Router as AxumRouter;
use bytes::Bytes;
use http_body::Frame;
use serde_json::{json, Value};
use vkdg_connections::{
    AuthKind, ConnectionCatalog, ConnectionConfig, Credential, CredentialManager, ProviderKind,
};
use vkdg_core::pipeline::PipelineCtx;
use vkdg_core::{ApiType, ClientId, ConnectionId, RequestEnvelope, RequestId, TenantId};
use vkdg_http::pipeline::run_conversation_pipeline;
use vkdg_http::upstream::HttpClient;
use vkdg_http::{AdmissionGuard, PipelineState};
use vkdg_observe::DecisionRecordExporter;
use vkdg_operations::{
    CapabilitySet, ConversationRequest, Message, MessageContent, Operation, Role,
};
use vkdg_provider_kiro::stream_decoder::KiroStreamDecoder;
use vkdg_provider_sdk::{
    ConversationStreamDecoder, PreparedRequest, ProviderAdapter, ProviderError, ProviderRegistry,
};
use vkdg_routing::{PluginHooks, RouteConfig, RouteId, Router, StrategyKind};

const MODEL: &str = "claude-sonnet-4.6";
const KEY_ENV: &str = "VKDG_STREAM_FAILOVER_KEY";
/// Credits do not return within minutes: an out-of-credits connection is held for 6 h.
const SIX_HOURS: u64 = 6 * 60 * 60;

// ── Kiro EventStream fixture ──────────────────────────────────────────────────

fn encode(message: &AwsMessage) -> Vec<u8> {
    let mut buf = Vec::new();
    write_message_to(message, &mut buf).expect("encode event stream frame");
    buf
}

fn header(name: &'static str, value: &str) -> Header {
    Header::new(name, AwsValue::String(value.to_owned().into()))
}

fn event(event_type: &str, payload: &Value) -> Vec<u8> {
    encode(
        &AwsMessage::new(payload.to_string())
            .add_header(header(":message-type", "event"))
            .add_header(header(":event-type", event_type))
            .add_header(header(":content-type", "application/json")),
    )
}

fn text(s: &str) -> Vec<u8> {
    event("assistantResponseEvent", &json!({ "content": s }))
}

/// An AWS exception frame (`ThrottlingException`, `ValidationException`, ...).
fn exception(kind: &str, message: &str) -> Vec<u8> {
    encode(
        &AwsMessage::new(json!({ "message": message }).to_string())
            .add_header(header(":message-type", "exception"))
            .add_header(header(":exception-type", kind))
            .add_header(header(":content-type", "application/json")),
    )
}

/// The Kiro data plane's own error frame: the code is in the payload.
fn plane_error(code: &str, message: &str) -> Vec<u8> {
    encode(
        &AwsMessage::new(json!({ "error_code": code, "error_message": message }).to_string())
            .add_header(header(":message-type", "error"))
            .add_header(header(":content-type", "application/json")),
    )
}

/// A frame Kiro sends first on most streams and the client never sees.
fn metering() -> Vec<u8> {
    event("meteringEvent", &json!({ "unit": "credit", "usage": 0.1 }))
}

fn answer(who: &str) -> Vec<u8> {
    [metering(), text(&format!("answer from {who}"))].concat()
}

fn out_of_credits() -> Vec<u8> {
    plane_error("MONTHLY_REQUEST_COUNT", "monthly request count exceeded")
}

// ── Fake Kiro upstream ────────────────────────────────────────────────────────

/// Sends `parts` one at a time with a short pause between them, so the pipeline
/// sees the HTTP body in several reads.
struct Dribble {
    parts: VecDeque<Bytes>,
    pause: Option<Pin<Box<tokio::time::Sleep>>>,
}

impl http_body::Body for Dribble {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        if let Some(pause) = self.pause.as_mut() {
            if pause.as_mut().poll(cx).is_pending() {
                return Poll::Pending;
            }
            self.pause = None;
        }
        match self.parts.pop_front() {
            None => Poll::Ready(None),
            Some(part) => {
                self.pause = Some(Box::pin(tokio::time::sleep(Duration::from_millis(2))));
                Poll::Ready(Some(Ok(Frame::data(part))))
            }
        }
    }
}

struct Shared {
    body: Vec<u8>,
    chunk: usize,
    hits: Arc<AtomicUsize>,
}

async fn serve(State(shared): State<Arc<Shared>>) -> Response {
    shared.hits.fetch_add(1, Ordering::SeqCst);
    let parts = shared
        .body
        .chunks(shared.chunk)
        .map(Bytes::copy_from_slice)
        .collect();
    Response::builder()
        .status(200)
        .header("content-type", "application/vnd.amazon.eventstream")
        .body(Body::new(Dribble { parts, pause: None }))
        .expect("response")
}

struct Upstream {
    url: String,
    hits: Arc<AtomicUsize>,
    _shutdown: tokio::sync::oneshot::Sender<()>,
}

impl Upstream {
    /// Serves `body` to every request, in reads of at most `chunk` bytes.
    async fn spawn(body: Vec<u8>, chunk: usize) -> Self {
        let hits = Arc::new(AtomicUsize::new(0));
        let shared = Arc::new(Shared {
            body,
            chunk,
            hits: Arc::clone(&hits),
        });
        let app = AxumRouter::new()
            .route("/v1/messages", post(serve))
            .with_state(shared);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let url = format!("http://{}", listener.local_addr().expect("addr"));
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = rx.await;
                })
                .await
                .ok();
        });
        Self {
            url,
            hits,
            _shutdown: tx,
        }
    }

    /// One read per request: the whole body at once.
    async fn whole(body: Vec<u8>) -> Self {
        let chunk = body.len();
        Self::spawn(body, chunk).await
    }

    fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }
}

/// Forwards to the connection's `endpoint` and decodes the reply as Kiro does.
struct KiroLike;

impl ProviderAdapter for KiroLike {
    fn id(&self) -> &'static str {
        "kiro-like"
    }

    fn display_name(&self) -> &'static str {
        "Kiro-like test provider"
    }

    fn prepare(
        &self,
        _operation: &Operation,
        config: &ConnectionConfig,
        _credential: &Credential,
    ) -> Result<PreparedRequest, ProviderError> {
        let endpoint = config
            .endpoint
            .as_deref()
            .ok_or(ProviderError::UnsupportedOperation)?;
        Ok(PreparedRequest {
            url: format!("{endpoint}/v1/messages"),
            headers: http::HeaderMap::new(),
            body: Bytes::new(),
            is_streaming: true,
        })
    }

    fn stream_decoder(&self) -> Option<Box<dyn ConversationStreamDecoder>> {
        Some(Box::new(KiroStreamDecoder::new()))
    }
}

// ── Pipeline + client ─────────────────────────────────────────────────────────

/// One Kiro-like connection per `(id, upstream url)`, all serving `claude-*`,
/// behind one route with `strategy`.
fn pipeline(upstreams: &[(&str, &Upstream)], strategy: StrategyKind) -> Arc<PipelineState> {
    std::env::set_var(KEY_ENV, "test-token");
    let configs: Vec<ConnectionConfig> = upstreams
        .iter()
        .map(|(id, upstream)| ConnectionConfig {
            id: ConnectionId((*id).into()),
            provider: ProviderKind::Plugin {
                id: "kiro-like".into(),
            },
            auth: AuthKind::ApiKey {
                env_var: KEY_ENV.into(),
            },
            models: vec!["claude-*".into()],
            max_concurrent: 10,
            weight: 1,
            tags: vec![],
            endpoint: Some(upstream.url.clone()),
            capabilities: CapabilitySet::default(),
        })
        .collect();
    let route = RouteConfig {
        id: RouteId("kiro-pair".into()),
        match_models: vec!["claude-*".into()],
        strategy,
        targets: configs.iter().map(|c| c.id.clone()).collect(),
        plugin_hooks: PluginHooks::default(),
    };
    let mut registry = ProviderRegistry::empty();
    registry.register(Arc::new(KiroLike));
    Arc::new(PipelineState::minimal(
        Arc::new(AdmissionGuard::new(100)),
        Arc::new(Router::new(vec![route])),
        Arc::new(ConnectionCatalog::new(configs)),
        Arc::new(CredentialManager::new()),
        Arc::new(HttpClient::new()),
        Arc::new(DecisionRecordExporter::new()),
        Arc::new(registry),
    ))
}

struct Reply {
    status: u16,
    retry_after: Option<u64>,
    body: String,
}

/// One streaming Anthropic-dialect request. Both the answer and the end of its
/// body are bounded: a pipeline that hangs fails the test instead of the suite.
async fn ask(pipeline: &Arc<PipelineState>) -> Reply {
    let envelope = RequestEnvelope {
        request_id: RequestId::new(),
        client_id: ClientId("stream-failover".into()),
        tenant_id: TenantId("default".into()),
        session_key: None,
        api_type: ApiType::AnthropicMessages,
        model_requested: MODEL.into(),
        deadline: None,
        mode_pack_override: None,
        compression_override: None,
        cache_bypass: false,
        include_think_tags: false,
        client_ip: None,
    };
    let op = Operation::Conversation(ConversationRequest {
        model: "test-model".into(),
        messages: vec![Message {
            role: Role::User,
            content: MessageContent::Text("hello".into()),
        }],
        tools: vec![],
        max_tokens: Some(64),
        temperature: None,
        stream: true,
        system: None,
        required_capabilities: CapabilitySet::default(),
        thinking: None,
        ..Default::default()
    });
    let response = tokio::time::timeout(
        Duration::from_secs(20),
        run_conversation_pipeline(Arc::clone(pipeline), PipelineCtx::new(envelope), op),
    )
    .await
    .expect("the pipeline must answer, not hang");
    let status = response.status().as_u16();
    let retry_after = response
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok());
    let bytes = tokio::time::timeout(
        Duration::from_secs(20),
        axum::body::to_bytes(response.into_body(), 1024 * 1024),
    )
    .await
    .expect("the response body must end")
    .expect("collect body");
    Reply {
        status,
        retry_after,
        body: String::from_utf8_lossy(&bytes).into_owned(),
    }
}

fn is_healthy(pipeline: &PipelineState, id: &str) -> bool {
    let connection = pipeline
        .catalog
        .get(&ConnectionId(id.into()))
        .expect("connection");
    let guard = connection.try_read().expect("uncontended");
    guard.state.is_healthy()
}

// ── Account failure on the first frame: fail over before the first byte ──────

/// Refutes: reading the failure only after the response is committed (the client
/// gets a 200 carrying an error event, the connection is never cooled and keeps
/// being picked), cooling a quota failure like a 429 (back after seconds), and a
/// retry that is not confined to a sibling. Run with the dead account first and
/// second in the route, and with its first frame split over several reads.
#[tokio::test]
async fn out_of_credits_on_the_first_frame_fails_over_and_keeps_that_connection_out() {
    const REQUESTS: usize = 24;
    for (dead_first, dead_chunk) in [(true, usize::MAX), (false, 7)] {
        let dead_body = [metering(), out_of_credits()].concat();
        let dead_chunk = dead_chunk.min(dead_body.len());
        let dead = Upstream::spawn(dead_body, dead_chunk).await;
        let live = Upstream::whole(answer("live")).await;
        let mut targets = vec![("dead", &dead), ("live", &live)];
        if !dead_first {
            targets.reverse();
        }
        let p = pipeline(&targets, StrategyKind::PowerOfTwoChoices);

        for i in 0..REQUESTS {
            let reply = ask(&p).await;
            assert_eq!(
                reply.status, 200,
                "request {i} (dead_first={dead_first}): {}",
                reply.body
            );
            assert!(
                reply.body.contains("answer from live"),
                "request {i} must be served by the live sibling: {}",
                reply.body
            );
            assert!(
                !reply.body.contains("event: error"),
                "the client must never see the failure: {}",
                reply.body
            );
        }

        assert_eq!(
            dead.hits(),
            1,
            "dead_first={dead_first}: probed once, then kept out (0 = never tried, >1 = no cooldown)"
        );
        assert_eq!(live.hits(), REQUESTS, "every request ends on the live one");
        let wait = p
            .catalog
            .secs_until_cooldown_ends(MODEL)
            .expect("the dead connection is cooling");
        assert!(
            u64::from(wait) + 30 >= SIX_HOURS,
            "out of credits holds the connection for ~6 h, got {wait} s"
        );
        assert!(
            is_healthy(&p, "live"),
            "invariant 6: the sibling is untouched"
        );
    }
}

/// Refutes: handling only the Kiro-plane `error_code` shape (a 402) and not the
/// AWS exception shape, or treating `ServiceQuotaExceeded` as a plain failure.
#[tokio::test]
async fn throttling_on_the_first_frame_fails_over_and_cools_the_connection() {
    for kind in ["ThrottlingException", "ServiceQuotaExceededException"] {
        let throttled = Upstream::spawn(exception(kind, "slow down"), 5).await;
        let live = Upstream::whole(answer("live")).await;
        let p = pipeline(
            &[("throttled", &throttled), ("live", &live)],
            StrategyKind::FallbackChain,
        );

        let reply = ask(&p).await;
        assert_eq!(reply.status, 200, "{kind}: {}", reply.body);
        assert!(
            reply.body.contains("answer from live"),
            "{kind}: {}",
            reply.body
        );
        assert!(
            !reply.body.contains("event: error"),
            "{kind}: {}",
            reply.body
        );
        assert_eq!(throttled.hits(), 1, "{kind}");
        assert_eq!(live.hits(), 1, "{kind}");
        assert!(
            !is_healthy(&p, "throttled"),
            "{kind}: the throttled connection must be cooling"
        );
        assert!(is_healthy(&p, "live"), "{kind}");
    }
}

/// Refutes: failing over a quota failure that is not the FIRST substantive event
/// (a client already holding half an answer cannot be moved: invariant 1), and
/// treating every first-event failure as an account failure.
#[tokio::test]
async fn failures_that_are_not_account_failures_on_the_first_frame_are_not_failed_over() {
    // (what Kiro sends, does the client see the failure as an in-stream error event)
    let cases: [(&str, Vec<u8>); 2] = [
        (
            "failure after a normal event",
            [
                text("partial"),
                exception("ThrottlingException", "slow down"),
            ]
            .concat(),
        ),
        (
            "bad request",
            exception("ValidationException", "improperly formed request"),
        ),
    ];
    for (name, body) in cases {
        let broken = Upstream::whole(body).await;
        let live = Upstream::whole(answer("live")).await;
        let p = pipeline(
            &[("broken", &broken), ("live", &live)],
            StrategyKind::FallbackChain,
        );

        let reply = ask(&p).await;
        assert_eq!(
            reply.status, 200,
            "{name}: committed stream: {}",
            reply.body
        );
        assert!(
            reply.body.contains("event: error"),
            "{name}: the client gets the failure as its dialect's error event: {}",
            reply.body
        );
        assert!(
            !reply.body.contains("answer from live"),
            "{name}: must not be replayed on the sibling: {}",
            reply.body
        );
        assert_eq!(broken.hits(), 1, "{name}");
        assert_eq!(live.hits(), 0, "{name}: no retry");
    }
}

// ── Normal streams are unchanged ──────────────────────────────────────────────

/// Refutes: a look-ahead that drops, duplicates or reorders the bytes it read
/// ahead (the first frame is split over many reads here), that retries a healthy
/// stream on a sibling, or that cools a connection that answered normally.
#[tokio::test]
async fn a_normal_stream_reaches_the_client_whole_once_and_is_never_retried() {
    let body = [metering(), text("alpha "), text("beta "), text("gamma")].concat();
    let first = Upstream::spawn(body, 7).await;
    let second = Upstream::whole(answer("second")).await;
    let p = pipeline(
        &[("first", &first), ("second", &second)],
        StrategyKind::FallbackChain,
    );

    for i in 0..5 {
        let reply = ask(&p).await;
        assert_eq!(reply.status, 200, "request {i}: {}", reply.body);
        let alpha = reply.body.find("alpha ").expect("alpha");
        let beta = reply.body.find("beta ").expect("beta");
        let gamma = reply.body.find("gamma").expect("gamma");
        assert!(alpha < beta && beta < gamma, "in order: {}", reply.body);
        for piece in ["alpha ", "beta ", "gamma"] {
            assert_eq!(
                reply.body.matches(piece).count(),
                1,
                "{piece:?} exactly once: {}",
                reply.body
            );
        }
        assert!(reply.body.contains("message_start"), "{}", reply.body);
        assert!(reply.body.contains("message_stop"), "{}", reply.body);
        assert!(!reply.body.contains("event: error"), "{}", reply.body);
    }
    assert_eq!(first.hits(), 5, "one upstream call per request");
    assert_eq!(
        second.hits(),
        0,
        "a healthy stream is never retried elsewhere"
    );
    assert!(is_healthy(&p, "first"));
}

// ── Nothing left to fail over to ──────────────────────────────────────────────

/// Refutes: a hang or a retry loop when every account is dead, an error that
/// hides why (a gateway 502 instead of the upstream's 402 and a wait), and
/// re-probing dead accounts on the next request instead of answering 429 with the
/// time until the first one recovers.
#[tokio::test]
async fn when_every_connection_is_out_of_credits_the_client_gets_the_upstream_error_and_a_wait() {
    let a = Upstream::spawn(out_of_credits(), 9).await;
    let b = Upstream::whole(out_of_credits()).await;
    let p = pipeline(&[("a", &a), ("b", &b)], StrategyKind::FallbackChain);

    let reply = ask(&p).await;
    assert_eq!(
        reply.status, 402,
        "the upstream's own status: {}",
        reply.body
    );
    let wait = reply.retry_after.expect("a retry-after");
    assert!(
        wait + 30 >= SIX_HOURS,
        "the client is told how long the accounts stay out, got {wait}"
    );
    assert_eq!(
        (a.hits(), b.hits()),
        (1, 1),
        "one retry, on the sibling only"
    );

    // Both are cooling: the next request is answered without touching either.
    let again = ask(&p).await;
    assert_eq!(again.status, 429, "{}", again.body);
    assert!(
        again.retry_after.expect("a retry-after") + 30 >= SIX_HOURS,
        "{:?}",
        again.retry_after
    );
    assert_eq!(
        (a.hits(), b.hits()),
        (1, 1),
        "dead accounts are not re-probed"
    );
}

/// Refutes: the throttled-everywhere case answering with no `retry-after`, or
/// with the first attempt's error after the retry already failed.
#[tokio::test]
async fn when_every_connection_is_throttled_the_client_gets_a_429_with_a_wait() {
    let a = Upstream::whole(exception("ThrottlingException", "slow down")).await;
    let b = Upstream::whole(exception("ThrottlingException", "slow down")).await;
    let p = pipeline(&[("a", &a), ("b", &b)], StrategyKind::FallbackChain);

    let reply = ask(&p).await;
    assert_eq!(reply.status, 429, "{}", reply.body);
    assert!(
        reply.retry_after.is_some_and(|s| s >= 1),
        "a retry-after, got {:?}",
        reply.retry_after
    );
    assert_eq!((a.hits(), b.hits()), (1, 1));
}
