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
