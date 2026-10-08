use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use bytes::Bytes;
use futures::{Stream, StreamExt};
use vkdg_core::{ApiType, RequestId, VkdgError};
use vkdg_operations::{ConversationEvent, StopReason, StreamContext};
use vkdg_provider_sdk::ConversationStreamDecoder;

use super::{decode_stream_to_sse, ByteStream};

/// A decoder that replays scripted events: one batch per `feed`, then `tail` at EOF.
struct Scripted {
    feeds: VecDeque<Vec<ConversationEvent>>,
    tail: Vec<ConversationEvent>,
    pct: Option<f64>,
}

use std::collections::VecDeque;

impl Scripted {
    fn new(feeds: Vec<Vec<ConversationEvent>>, tail: Vec<ConversationEvent>) -> Self {
        Self {
            feeds: feeds.into(),
            tail,
            pct: None,
        }
    }
}

impl ConversationStreamDecoder for Scripted {
    fn feed(&mut self, _chunk: Bytes) -> Vec<ConversationEvent> {
        self.feeds.pop_front().unwrap_or_default()
    }
    fn finish(&mut self) -> Vec<ConversationEvent> {
        std::mem::take(&mut self.tail)
    }
    fn context_usage_pct(&self) -> Option<f64> {
        self.pct
    }
}

fn started() -> ConversationEvent {
    ConversationEvent::Started {
        request_id: RequestId::new(),
    }
}

fn text(t: &str) -> ConversationEvent {
    ConversationEvent::OutputDelta {
        delta: t.into(),
        index: 0,
    }
}

fn upstream_chunks(n: usize) -> ByteStream {
    Box::pin(futures::stream::iter(
        (0..n).map(|_| Ok(Bytes::from_static(b"x"))),
    ))
}

async fn run(body: ByteStream, decoder: Scripted, api_type: &ApiType) -> (String, Vec<Vec<u8>>) {
    let rid = RequestId::new();
    let ctx = StreamContext {
        model: "m",
        request_id: &rid,
    };
    let chunks: Vec<Vec<u8>> = decode_stream_to_sse(body, Box::new(decoder), api_type, &ctx)
        .map(|r| r.map(|b| b.to_vec()).unwrap_or_default())
        .collect()
        .await;
    (
        String::from_utf8_lossy(&chunks.concat()).into_owned(),
        chunks,
    )
}

// Refutes: skipping `encoder.finish()` whenever `decoder.finish()` yields events.
// A decoder whose EOF flush emits content but no `Completed` left the OpenAI client
// without finish_reason and `[DONE]`.
#[tokio::test]
async fn decoder_flush_events_do_not_skip_the_dialect_close() {
    let decoder = Scripted::new(vec![vec![started(), text("hi")]], vec![text("tail")]);
    let (out, _) = run(upstream_chunks(1), decoder, &ApiType::OpenAiChatCompletions).await;
    assert!(out.contains("\"content\":\"tail\""), "{out}");
    assert!(out.contains("\"finish_reason\":\"stop\""), "{out}");
    assert!(out.ends_with("data: [DONE]\n\n"), "{out}");
}

// Refutes: the Kiro context-usage tail disappearing when the decoder flush has events.
#[tokio::test]
async fn context_usage_tail_survives_decoder_flush_events() {
    let mut decoder = Scripted::new(vec![vec![started(), text("hi")]], vec![text("tail")]);
    decoder.pct = Some(12.5);
    let (out, _) = run(upstream_chunks(1), decoder, &ApiType::AnthropicMessages).await;
    assert!(out.contains("message_stop"), "{out}");
    assert!(
        out.ends_with("data: {\"type\":\"vkdg_context_usage\",\"pct\":12.5}\n\n"),
        "{out}"
    );
}

// Refutes: a decoder `Failed` after commit that keeps reading upstream or leaves the
// client without its dialect's error event (and, for Anthropic, with a second event after it).
#[tokio::test]
async fn failed_after_commit_ends_the_client_stream_with_its_dialect_error_and_stops_reading() {
    let pulled = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&pulled);
    let body: ByteStream = Box::pin(futures::stream::iter(0..10).map(move |_| {
        counter.fetch_add(1, Ordering::SeqCst);
        Ok(Bytes::from_static(b"x"))
    }));
    let failure = ConversationEvent::Failed {
        error: VkdgError::UpstreamError {
            code: 529,
            message: "Overloaded".into(),
            retry_after: None,
        },
    };
    let decoder = Scripted::new(
        vec![vec![started(), text("hi")], vec![failure]],
        vec![ConversationEvent::Completed {
            stop_reason: StopReason::EndTurn,
        }],
    );
    let (out, _) = run(body, decoder, &ApiType::AnthropicMessages).await;
    assert!(out.contains("\"text\":\"hi\""), "{out}");
    assert!(out.contains("event: error\n"), "{out}");
    // 529 is Anthropic's "overloaded"; the shared error table maps it to 429 for clients.
    assert!(out.contains("rate_limit_error"), "{out}");
    assert!(
        !out.contains("message_stop"),
        "nothing may follow the error: {out}"
    );
    assert_eq!(
        pulled.load(Ordering::SeqCst),
        2,
        "upstream read after the failure"
    );
}

// Refutes: a relay that buffers ahead of a slow client. The client takes one item;
// upstream may have been polled for that item and no more.
#[tokio::test]
async fn relay_reads_upstream_only_as_fast_as_the_client_consumes() {
    let pulled = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&pulled);
    let body: ByteStream = Box::pin(futures::stream::iter(0..1000).map(move |_| {
        counter.fetch_add(1, Ordering::SeqCst);
        Ok(Bytes::from_static(b"x"))
    }));
    let feeds = (0..1000).map(|_| vec![text("a")]).collect();
    let rid = RequestId::new();
    let ctx = StreamContext {
        model: "m",
        request_id: &rid,
    };
    let mut relay = decode_stream_to_sse(
        body,
        Box::new(Scripted::new(feeds, vec![])),
        &ApiType::OpenAiChatCompletions,
        &ctx,
    );
    let _first = relay.next().await;
    assert!(
        pulled.load(Ordering::SeqCst) <= 2,
        "pulled {} chunks for one consumed item",
        pulled.load(Ordering::SeqCst)
    );
}

#[allow(dead_code)]
fn assert_send<S: Stream + Send>(_: &S) {}

// ── guard_first_event ─────────────────────────────────────────────────────────

use std::io;
use std::task::Poll;

use super::guard_first_event;

/// Adapter whose decoder is built fresh per call from `script`, like a real one.
struct ScriptedAdapter {
    script: Option<Box<dyn Fn() -> Scripted + Send + Sync>>,
}

fn adapter(script: impl Fn() -> Scripted + Send + Sync + 'static) -> ScriptedAdapter {
    ScriptedAdapter {
        script: Some(Box::new(script)),
    }
}

impl vkdg_provider_sdk::ProviderAdapter for ScriptedAdapter {
    fn id(&self) -> &'static str {
        "scripted"
    }
    fn display_name(&self) -> &'static str {
        "Scripted"
    }
    fn prepare(
        &self,
        _operation: &vkdg_operations::Operation,
        _config: &vkdg_connections::ConnectionConfig,
        _credential: &vkdg_provider_sdk::Credential,
    ) -> Result<vkdg_provider_sdk::PreparedRequest, vkdg_provider_sdk::ProviderError> {
        Err(vkdg_provider_sdk::ProviderError::UnsupportedOperation)
    }
    fn stream_decoder(&self) -> Option<Box<dyn ConversationStreamDecoder>> {
        self.script
            .as_ref()
            .map(|script| Box::new(script()) as Box<dyn ConversationStreamDecoder>)
    }
}

fn failed(code: u16, retry_after: Option<u32>) -> ConversationEvent {
    ConversationEvent::Failed {
        error: VkdgError::UpstreamError {
            code,
            message: format!("upstream said {code}"),
            retry_after,
        },
    }
}

/// An upstream that yields `items`, counts every poll and panics if polled again
/// after it returned `None` (a stream must not be polled past its end).
fn strict(items: Vec<io::Result<Bytes>>, polls: &Arc<AtomicUsize>) -> ByteStream {
    let polls = Arc::clone(polls);
    let mut items: VecDeque<_> = items.into();
    let mut ended = false;
    Box::pin(futures::stream::poll_fn(move |_| {
        polls.fetch_add(1, Ordering::SeqCst);
        assert!(!ended, "upstream polled after its end");
        if let Some(item) = items.pop_front() {
            Poll::Ready(Some(item))
        } else {
            ended = true;
            Poll::Ready(None)
        }
    }))
}

fn chunks(parts: &[&'static str]) -> Vec<io::Result<Bytes>> {
    parts
        .iter()
        .map(|p| Ok(Bytes::from_static(p.as_bytes())))
        .collect()
}

async fn collect(body: ByteStream) -> Vec<Result<Vec<u8>, String>> {
    body.map(|r| r.map(|b| b.to_vec()).map_err(|e| e.to_string()))
        .collect()
        .await
}

// Refutes: a look-ahead that stops at the first decoded batch even when it only held
// `Started` (a quota frame can follow metering frames), that buffers the whole stream,
// or that loses/reorders/duplicates the chunks it consumed. The client must receive
// every upstream byte once, in order, and upstream is read only up to the first
// substantive event.
#[tokio::test]
async fn look_ahead_stops_at_the_first_substantive_event_and_replays_every_chunk_in_order() {
    let adapter = adapter(|| {
        Scripted::new(
            vec![
                vec![started()],
                vec![],
                vec![text("hi")],
                vec![text("more")],
            ],
            vec![],
        )
    });
    let polls = Arc::new(AtomicUsize::new(0));
    let body = strict(chunks(&["a", "b", "c", "d", "e"]), &polls);

    let guarded = guard_first_event(&adapter, body)
        .await
        .expect("normal stream");
    assert_eq!(
        polls.load(Ordering::SeqCst),
        3,
        "reads chunks a, b, c (Started-only and empty batches do not count) and no more"
    );
    let all = collect(guarded).await;
    let bytes: Vec<u8> = all.into_iter().flat_map(|c| c.unwrap()).collect();
    assert_eq!(bytes, b"abcde", "every upstream byte, once, in order");
}

// Refutes: a look-ahead that lets an out-of-credits / throttled first event through
// (client gets a 200 with an error frame, no cooldown, no failover), or that returns
// the error with the wrong status, message or retry_after.
#[tokio::test]
async fn a_failover_status_as_first_event_is_returned_before_any_byte_is_replayed() {
    for (code, retry_after) in [(402, None), (429, Some(7)), (529, None)] {
        let adapter = adapter(move || {
            Scripted::new(vec![vec![started(), failed(code, retry_after)]], vec![])
        });
        let polls = Arc::new(AtomicUsize::new(0));
        let body = strict(chunks(&["x", "y"]), &polls);
        match guard_first_event(&adapter, body).await {
            Err(VkdgError::UpstreamError {
                code: got,
                message,
                retry_after: got_wait,
            }) => {
                assert_eq!(got, code);
                assert_eq!(message, format!("upstream said {code}"));
                assert_eq!(got_wait, retry_after);
            }
            Err(other) => panic!("expected the upstream error, got {other:?}"),
            Ok(_) => panic!("{code} as first event must fail the look-ahead"),
        }
        assert_eq!(
            polls.load(Ordering::SeqCst),
            1,
            "stopped at the failing chunk"
        );
    }
}

// Refutes: treating every first-event `Failed` as an account failure (a 400 bad request
// would cool a healthy connection and be replayed on a sibling), and examining a `Failed`
// that follows a normal event (the client already holds the response: retry is unsafe).
#[tokio::test]
async fn only_a_failover_failure_that_is_the_first_substantive_event_fails_the_look_ahead() {
    let bad_request = adapter(|| Scripted::new(vec![vec![started(), failed(400, None)]], vec![]));
    let after_text = adapter(|| {
        Scripted::new(
            vec![vec![started(), text("partial"), failed(429, None)]],
            vec![],
        )
    });
    for adapter in [bad_request, after_text] {
        let polls = Arc::new(AtomicUsize::new(0));
        let body = strict(chunks(&["x", "y"]), &polls);
        let guarded = guard_first_event(&adapter, body)
            .await
            .expect("not an account failure: the client gets the normal stream");
        let bytes: Vec<u8> = collect(guarded)
            .await
            .into_iter()
            .flat_map(|c| c.unwrap())
            .collect();
        assert_eq!(bytes, b"xy");
    }
}

// Refutes: an upstream read error before any event being swallowed or turned into a
// failover, or the stream being polled again after it failed. Today's behaviour is that
// the client's stream carries the error; it must stay so, after the bytes already read.
#[tokio::test]
async fn an_io_error_before_the_first_event_reaches_the_client_after_the_bytes_read() {
    let adapter = adapter(|| Scripted::new(vec![vec![]], vec![]));
    let polls = Arc::new(AtomicUsize::new(0));
    let mut items = chunks(&["a"]);
    items.push(Err(io::Error::other("reset")));
    let body = strict(items, &polls);

    let guarded = guard_first_event(&adapter, body)
        .await
        .expect("not a failover");
    let all = collect(guarded).await;
    assert_eq!(all, vec![Ok(b"a".to_vec()), Err("reset".to_owned())]);
    assert_eq!(polls.load(Ordering::SeqCst), 2, "no poll after the error");
}

// Refutes: polling the upstream again after it ended during the look-ahead (strict
// stream panics), and losing the decoder's end-of-stream failure for the relay.
#[tokio::test]
async fn a_stream_that_ends_before_any_event_is_replayed_without_polling_past_the_end() {
    let adapter = adapter(|| Scripted::new(vec![vec![]], vec![failed(502, None)]));
    let polls = Arc::new(AtomicUsize::new(0));
    let body = strict(chunks(&["a"]), &polls);

    let guarded = guard_first_event(&adapter, body)
        .await
        .expect("502 is not a failover");
    assert_eq!(collect(guarded).await, vec![Ok(b"a".to_vec())]);
    assert_eq!(polls.load(Ordering::SeqCst), 2, "chunk, then the end, once");
}

// Refutes: look-ahead on passthrough providers, which would delay their first byte
// and read an upstream nobody asked to inspect.
#[tokio::test]
async fn a_provider_without_a_decoder_is_not_looked_ahead() {
    let adapter = ScriptedAdapter { script: None };
    let polls = Arc::new(AtomicUsize::new(0));
    let body = strict(chunks(&["a", "b"]), &polls);

    let guarded = guard_first_event(&adapter, body)
        .await
        .expect("passthrough");
    assert_eq!(
        polls.load(Ordering::SeqCst),
        0,
        "nothing read before the client asks"
    );
    drop(guarded);
}
