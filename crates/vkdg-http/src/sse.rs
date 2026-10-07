use std::pin::Pin;
use std::time::Duration;

use bytes::Bytes;
use futures::{Stream, StreamExt};

// Incremental SSE parser — the owned-event view of `vkdg_provider_sdk::SseFramer`.
//
// Framing (line endings, partial UTF-8, comments, the per-event size bound) lives
// in exactly one place, the framer. This type only turns its borrowed frames into
// owned events and applies the optional think-tag strip.

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub event_type: Option<String>,
    pub data: String,
}

// ── Parser ────────────────────────────────────────────────────────────────────

pub struct SseParser {
    framer: vkdg_provider_sdk::SseFramer,
    /// Whether to strip <think>...</think> blocks from text delta events.
    /// Default: true (most clients don't want to see reasoning tokens).
    pub strip_think_tags: bool,
}

impl SseParser {
    pub fn new() -> Self {
        Self {
            framer: vkdg_provider_sdk::SseFramer::new(),
            strip_think_tags: true,
        }
    }

    /// Keep think-tag blocks in the output (opt-in by client via X-VKDG-Think-Tags: include).
    #[must_use]
    pub fn with_think_tags(mut self) -> Self {
        self.strip_think_tags = false;
        self
    }

    /// Push a chunk of bytes (any size, any alignment) and return all complete
    /// SSE events that can be extracted from the buffer. An event larger than the
    /// framer's limit is dropped (and logged), never buffered without bound.
    pub fn push(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        let strip = self.strip_think_tags;
        let mut events = Vec::new();
        self.framer.push(chunk, |frame| match frame {
            Ok(frame) => {
                let data = frame.data.into_owned();
                events.push(SseEvent {
                    event_type: frame.event.map(str::to_owned),
                    data: if strip { strip_think_tags(&data) } else { data },
                });
            }
            Err(error) => tracing::warn!(%error, "dropping oversized SSE event"),
        });
        events
    }
}

impl Default for SseParser {
    fn default() -> Self {
        Self::new()
    }
}

/// Remove <think>...</think> blocks from SSE data.
/// Handles complete blocks within a single data line.
/// Blocks spanning multiple SSE events are a known gap (see test documentation).
pub fn strip_think_tags(data: &str) -> String {
    let mut result = data.to_string();
    loop {
        match (result.find("<think>"), result.find("</think>")) {
            (Some(start), Some(end)) if start < end => {
                let end_full = end + "</think>".len();
                result = format!("{}{}", &result[..start], &result[end_full..]);
            }
            _ => break,
        }
    }
    result
}

// ── Stream termination guard ──────────────────────────────────────────────────

/// True when this chunk ends the stream in either client dialect.
///
/// `OpenAI` ends with a `[DONE]` sentinel; Anthropic ends with `message_stop`, or with
/// an `error` event after a failure. Recognising only `[DONE]` made every successful
/// Anthropic response end with a spurious error event.
fn is_terminal_chunk(chunk: &[u8]) -> bool {
    let contains = |needle: &[u8]| chunk.windows(needle.len()).any(|w| w == needle);
    contains(b"[DONE]")
        || contains(b"message_stop")
        || contains(b"response.done")
        || contains(b"response.completed")
        || contains(b"event: error")
}

/// Wrap a byte stream and end it with the client dialect's error frame if it
/// finishes (or errors) without a terminal event.
///
/// Detects upstream TCP closure mid-stream: the client gets a well-formed error
/// frame (Anthropic `event: error`, or `OpenAI`'s error object and `[DONE]`) so it can
/// tell an incomplete response from a complete one.
pub fn with_termination_guard(
    inner: Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>>,
    client: &vkdg_core::ApiType,
) -> Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>> {
    let interrupted = Bytes::from(vkdg_operations::stream_error_frame(
        client,
        &vkdg_core::VkdgError::UpstreamError {
            code: 502,
            message: "upstream stream ended before completion".into(),
            retry_after: None,
        },
    ));
    Box::pin(futures::stream::unfold(
        (inner, false, false),
        move |(mut stream, mut saw_done, mut finished)| {
            let interrupted = interrupted.clone();
            async move {
                if finished {
                    return None;
                }
                match stream.next().await {
                    Some(Ok(chunk)) => {
                        if is_terminal_chunk(&chunk) {
                            saw_done = true;
                        }
                        Some((Ok(chunk), (stream, saw_done, finished)))
                    }
                    None => {
                        finished = true;
                        if saw_done {
                            None
                        } else {
                            tracing::warn!(
                                "upstream closed SSE stream before its terminal event; injecting error frame"
                            );
                            Some((Ok(interrupted), (stream, true, finished)))
                        }
                    }
                    Some(Err(e)) => {
                        finished = true;
                        tracing::warn!(error = %e, "SSE stream error; injecting error frame");
                        Some((Ok(interrupted), (stream, true, finished)))
                    }
                }
            }
        },
    ))
}

/// An SSE comment line. Every SSE parser, in either dialect, skips comments.
pub const COMMENT_PING: &[u8] = b": ping\n\n";
/// Idle gap before a keepalive frame is sent. Well under Claude Code's 300 s
/// stream watchdog and Codex's `stream_idle_timeout_ms`.
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15);

/// Emit `frame` whenever `inner` is idle for `interval` at an event boundary.
///
/// A model thinking for minutes sends no bytes, and clients abort idle sockets.
/// The frame is only sent between events: splicing it into an event that is
/// split across reads would corrupt that event.
pub fn with_heartbeat(
    inner: Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>>,
    interval: Duration,
    frame: &'static [u8],
) -> Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>> {
    Box::pin(futures::stream::unfold(
        (inner, true),
        move |(mut stream, at_boundary)| async move {
            let next = if at_boundary {
                // `next()` is cancel-safe: dropping it on timeout loses no data.
                match tokio::time::timeout(interval, stream.next()).await {
                    Ok(next) => next,
                    Err(_) => return Some((Ok(Bytes::from_static(frame)), (stream, true))),
                }
            } else {
                stream.next().await
            };
            let item = next?;
            let boundary = match &item {
                Ok(c) if c.is_empty() => at_boundary,
                Ok(c) => c.ends_with(b"\n\n") || c.ends_with(b"\r\n\r\n"),
                Err(_) => at_boundary,
            };
            Some((item, (stream, boundary)))
        },
    ))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_chunk_complete_event() {
        let mut p = SseParser::new();
        let events = p.push(b"data: hello\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "hello");
    }

    #[test]
    fn fragmented_at_arbitrary_byte() {
        // Split mid-field-name: "data: {\"ty" | "pe\":\"x\"}\n\n"
        let full = b"data: {\"type\":\"output_delta\",\"index\":0,\"delta\":\"hi\"}\n\n";
        for split in 1..full.len() {
            let mut p = SseParser::new();
            let mut all: Vec<SseEvent> = Vec::new();
            all.extend(p.push(&full[..split]));
            all.extend(p.push(&full[split..]));
            assert_eq!(
                all.len(),
                1,
                "split at byte {split} produced wrong event count"
            );
            assert_eq!(
                all[0].data, "{\"type\":\"output_delta\",\"index\":0,\"delta\":\"hi\"}",
                "split at byte {split}"
            );
        }
    }

    #[test]
    fn done_sentinel() {
        let mut p = SseParser::new();
        let events = p.push(b"data: [DONE]\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "[DONE]");
    }

    #[test]
    fn multiple_events_in_one_chunk() {
        let mut p = SseParser::new();
        let events = p.push(b"data: first\n\ndata: second\n\n");
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].data, "first");
        assert_eq!(events[1].data, "second");
    }

    #[test]
    fn event_with_type_field() {
        let mut p = SseParser::new();
        let events = p.push(b"event: ping\ndata: payload\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, Some("ping".to_string()));
        assert_eq!(events[0].data, "payload");
    }

    // Plausible wrong impl: parser emits an empty SseEvent when it sees \n\n
    // with no data: field (e.g. comment-only or blank block).
    // This test defeats it: events with no data lines must be silently dropped.
    #[test]
    fn comment_only_block_produces_no_event() {
        let mut p = SseParser::new();
        let events = p.push(b": this is a comment\n\n");
        assert_eq!(events.len(), 0, "comment-only block must not emit an event");
    }

    // Plausible wrong impl: parser returns a partial event when the buffer ends
    // without a double-newline (EOF mid-stream), leaking incomplete data.
    // This test defeats it: incomplete events must stay buffered, not emitted.
    #[test]
    fn incomplete_event_stays_buffered() {
        let mut p = SseParser::new();
        let events = p.push(b"data: partial");
        assert_eq!(
            events.len(),
            0,
            "no \\n\\n seen yet — event must remain in buffer, not emitted"
        );
    }

    // Plausible wrong impl: parser completes after the first \n even though SSE
    // requires \n\n (blank line) to end an event — causing split emission.
    // This test defeats it: single \n inside an event must not terminate it.
    #[test]
    fn single_newline_does_not_terminate_event() {
        let mut p = SseParser::new();
        // One \n between fields, NOT a blank line
        let events = p.push(b"event: msg\ndata: body");
        assert_eq!(
            events.len(),
            0,
            "single \\n must not emit — event needs \\n\\n to be complete"
        );
        // Now close it
        let events2 = p.push(b"\n\n");
        assert_eq!(events2.len(), 1);
        assert_eq!(events2[0].data, "body");
        assert_eq!(events2[0].event_type, Some("msg".into()));
    }

    // Plausible wrong impl: parser treats \r\n\r\n (HTTP-style line endings) as
    // two separate single-newline events instead of one event terminator.
    // This test defeats it: \r\n\r\n must be recognised as an event boundary.
    #[test]
    fn crlf_event_delimiter_recognised() {
        let mut p = SseParser::new();
        let events = p.push(b"data: crlf\r\n\r\n");
        assert_eq!(events.len(), 1, "\\r\\n\\r\\n must terminate an SSE event");
        assert_eq!(events[0].data, "crlf");
    }

    // Plausible wrong impl: parser decodes the whole buffer as UTF-8 and panics
    // or drops the event when a multi-byte UTF-8 sequence is split across chunks.
    // This test defeats it: partial UTF-8 at a chunk boundary must be held until
    // the continuation byte arrives.
    #[test]
    fn partial_utf8_sequence_across_chunk_boundary() {
        // "é" = 0xC3 0xA9 (2-byte UTF-8).  Split between the two bytes.
        // Full event: b"data: caf\xc3\xa9\n\n"
        let mut p = SseParser::new();
        // First chunk ends mid-UTF-8: includes 0xC3 but not 0xA9
        let events_a = p.push(b"data: caf\xc3");
        assert_eq!(events_a.len(), 0, "partial UTF-8 — no complete event yet");
        // Second chunk completes the character and the event
        let events_b = p.push(b"\xa9\n\n");
        assert_eq!(events_b.len(), 1, "UTF-8 completed — event must now emit");
        assert_eq!(events_b[0].data, "café");
    }

    // Plausible wrong impl: multi-line data fields (multiple data: lines) are
    // concatenated with nothing, losing the spec-mandated \n separator.
    // This test defeats it: adjacent data: lines must be joined with \n.
    #[test]
    fn multi_line_data_joined_with_newline() {
        let mut p = SseParser::new();
        let events = p.push(b"data: line1\ndata: line2\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].data, "line1\nline2",
            "multi-line data: fields must be joined with \\n"
        );
    }

    // Plausible wrong impl: think tags not stripped, reasoning leaks to client.
    #[test]
    fn strip_complete_think_block() {
        let input = "Before<think>internal reasoning here</think>After";
        assert_eq!(strip_think_tags(input), "BeforeAfter");
    }

    // Plausible wrong impl: nested or empty think blocks cause infinite loop.
    #[test]
    fn strip_think_block_empty() {
        assert_eq!(strip_think_tags("<think></think>answer"), "answer");
    }

    // Plausible wrong impl: stripping modifies content outside think blocks.
    #[test]
    fn non_think_content_unchanged() {
        let input = "The answer is 42. No reasoning here.";
        assert_eq!(strip_think_tags(input), input);
    }

    // Known gap: a <think> block that spans two separate SSE events (first event
    // ends with "<think>partial reasoning" and the next starts with
    // "more reasoning</think>answer") will NOT be filtered — the per-event
    // strip_think_tags() only sees one complete event at a time and has no
    // cross-event state. Clients that need guaranteed filtering of cross-event
    // think blocks must use a higher-level buffer. This test documents the gap:
    #[test]
    fn think_block_spanning_two_data_lines_is_known_gap() {
        // The parser joins multiple data: lines within one SSE event with \n,
        // so a think block split across two data: lines in the *same* event IS
        // filtered (both halves are joined before strip_think_tags runs).
        let mut p = SseParser::new();
        let events = p.push(b"data: hello <think>reasoning\ndata: end</think> world\n\n");
        assert_eq!(events.len(), 1);
        // Multi-line data is joined: "hello <think>reasoning\nend</think> world"
        // strip_think_tags sees the complete block and removes it.
        assert_eq!(events[0].data, "hello  world");
    }

    // Plausible wrong impl: stream closes cleanly with 200 when upstream dies,
    // client cannot detect incomplete response.
    #[tokio::test]
    async fn termination_guard_injects_error_when_done_missing() {
        use futures::stream;

        // Stream that ends without [DONE] — simulates upstream dropping connection.
        let chunks: Vec<Result<Bytes, std::io::Error>> = vec![
            Ok(Bytes::from_static(
                b"data: {\"type\":\"content_block_delta\"}\n\n",
            )),
            Ok(Bytes::from_static(
                b"data: {\"type\":\"content_block_delta\"}\n\n",
            )),
        ];
        let inner: Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>> =
            Box::pin(stream::iter(chunks));
        let guarded: Vec<_> =
            with_termination_guard(inner, &vkdg_core::ApiType::OpenAiChatCompletions)
                .collect()
                .await;

        // Last chunk must contain the error event.
        let last = guarded.last().unwrap().as_ref().unwrap();
        let text = std::str::from_utf8(last).unwrap();
        assert!(
            text.contains("upstream stream ended before completion"),
            "missing [DONE] must produce an error object, got: {text}"
        );
        assert!(
            text.ends_with("data: [DONE]\n\n"),
            "an OpenAI client's error frame ends with [DONE]: {text}"
        );
    }

    // Plausible wrong impl: termination guard adds error event even for complete streams.
    #[tokio::test]
    async fn termination_guard_passes_through_complete_stream() {
        use futures::stream;

        // Stream that ends properly with [DONE].
        let chunks: Vec<Result<Bytes, std::io::Error>> = vec![
            Ok(Bytes::from_static(b"data: {\"delta\":{}}\n\n")),
            Ok(Bytes::from_static(b"data: [DONE]\n\n")),
        ];
        let inner: Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>> =
            Box::pin(stream::iter(chunks));
        let guarded: Vec<_> =
            with_termination_guard(inner, &vkdg_core::ApiType::OpenAiChatCompletions)
                .collect()
                .await;

        // Must have exactly 2 chunks (no extra error event injected).
        assert_eq!(
            guarded.len(),
            2,
            "complete stream must not have extra events injected"
        );
    }

    /// Refutes: treating `[DONE]` as the only valid terminator.
    ///
    /// Anthropic streams end with `message_stop`; there is no `[DONE]` sentinel in
    /// that dialect. Requiring one made every successful Anthropic response carry a
    /// trailing `overloaded_error`, so a client that trusts its own protocol saw a
    /// completed answer as a failure. Found against a live Kiro account.
    #[tokio::test]
    async fn anthropic_message_stop_is_a_valid_terminator() {
        use futures::stream;

        let chunks: Vec<Result<Bytes, std::io::Error>> = vec![
            Ok(Bytes::from_static(
                b"event: content_block_delta\ndata: {\"delta\":{\"text\":\"hi\"}}\n\n",
            )),
            Ok(Bytes::from_static(
                b"event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
            )),
        ];
        let inner: Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>> =
            Box::pin(stream::iter(chunks));
        let guarded: Vec<_> = with_termination_guard(inner, &vkdg_core::ApiType::AnthropicMessages)
            .collect()
            .await;

        let text: String = guarded
            .iter()
            .filter_map(|r| r.as_ref().ok())
            .map(|b| String::from_utf8_lossy(b).to_string())
            .collect();
        assert!(
            !text.contains("event: error"),
            "a stream ending in message_stop is complete: {text}"
        );
        assert_eq!(
            guarded.len(),
            2,
            "no extra frame may follow message_stop: {text}"
        );
    }

    #[tokio::test]
    async fn responses_done_is_a_valid_terminator() {
        use futures::stream;

        let chunks: Vec<Result<Bytes, std::io::Error>> = vec![
            Ok(Bytes::from_static(
                b"event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"hi\"}\n\n",
            )),
            Ok(Bytes::from_static(
                b"event: response.done\ndata: {\"type\":\"response.done\"}\n\n",
            )),
        ];
        let inner: Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>> =
            Box::pin(stream::iter(chunks));
        let guarded: Vec<_> =
            with_termination_guard(inner, &vkdg_core::ApiType::OpenAiChatCompletions)
                .collect()
                .await;

        let text: String = guarded
            .iter()
            .filter_map(|r| r.as_ref().ok())
            .map(|b| String::from_utf8_lossy(b).to_string())
            .collect();
        assert!(
            !text.contains("upstream stream ended before completion"),
            "a stream ending in response.done is complete: {text}"
        );
        assert_eq!(
            guarded.len(),
            2,
            "no extra error frame may follow response.done"
        );
    }

    // Plausible wrong impl: stream error silently terminates without notifying client.
    #[tokio::test]
    async fn termination_guard_injects_error_on_stream_error() {
        use futures::stream;

        let chunks: Vec<Result<Bytes, std::io::Error>> = vec![
            Ok(Bytes::from_static(
                b"data: {\"type\":\"content_block_delta\"}\n\n",
            )),
            Err(std::io::Error::new(
                std::io::ErrorKind::ConnectionReset,
                "reset",
            )),
        ];
        let inner: Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>> =
            Box::pin(stream::iter(chunks));
        let guarded: Vec<_> = with_termination_guard(inner, &vkdg_core::ApiType::AnthropicMessages)
            .collect()
            .await;

        // Guard swallows the Err and replaces it with an Ok error-event chunk.
        let last = guarded.last().unwrap().as_ref().unwrap();
        let text = std::str::from_utf8(last).unwrap();
        assert!(
            text.starts_with("event: error\n")
                && text.contains("upstream stream ended before completion"),
            "an Anthropic client gets an `event: error` frame, got: {text}"
        );
    }

    type ByteTx = futures::channel::mpsc::UnboundedSender<Result<Bytes, std::io::Error>>;

    #[allow(clippy::type_complexity)] // test helper only
    fn heartbeat_over_channel() -> (
        ByteTx,
        Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>>,
    ) {
        let (tx, rx) = futures::channel::mpsc::unbounded();
        (
            tx,
            with_heartbeat(Box::pin(rx), HEARTBEAT_INTERVAL, COMMENT_PING),
        )
    }

    fn send(tx: &ByteTx, s: &'static str) {
        tx.unbounded_send(Ok(Bytes::from_static(s.as_bytes())))
            .unwrap();
    }

    /// Next chunk, failing (not hanging) if none arrives within two intervals.
    async fn next_chunk(
        hb: &mut Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>>,
    ) -> Bytes {
        tokio::time::timeout(2 * HEARTBEAT_INTERVAL, hb.next())
            .await
            .expect("no chunk within two heartbeat intervals")
            .expect("stream ended")
            .expect("stream error")
    }

    // Plausible wrong impl: no keepalive at all, so a model thinking for minutes
    // leaves the client with a silent socket until its idle watchdog aborts.
    #[tokio::test(start_paused = true)]
    async fn idle_upstream_gets_a_ping_after_the_interval() {
        let (tx, mut hb) = heartbeat_over_channel();
        send(&tx, "data: a\n\n");
        assert_eq!(next_chunk(&mut hb).await, "data: a\n\n");

        let start = tokio::time::Instant::now();
        let ping = next_chunk(&mut hb).await;
        assert_eq!(ping.as_ref(), COMMENT_PING);
        assert!(start.elapsed() >= HEARTBEAT_INTERVAL, "ping sent too early");

        send(&tx, "data: b\n\n");
        assert_eq!(next_chunk(&mut hb).await, "data: b\n\n");
    }

    // Plausible wrong impl: pings whenever idle, splicing a frame into the middle
    // of an event split across TCP reads and corrupting it for the client.
    #[tokio::test(start_paused = true)]
    async fn no_ping_inside_a_partial_event() {
        let (tx, mut hb) = heartbeat_over_channel();
        send(&tx, "data: {\"par");
        assert_eq!(next_chunk(&mut hb).await, "data: {\"par");

        let waited = tokio::time::timeout(4 * HEARTBEAT_INTERVAL, hb.next()).await;
        assert!(waited.is_err(), "ping spliced mid-event: {waited:?}");

        send(&tx, "tial\"}\n\n");
        assert_eq!(next_chunk(&mut hb).await, "tial\"}\n\n");
        assert_eq!(next_chunk(&mut hb).await.as_ref(), COMMENT_PING);
    }

    // Plausible wrong impl: a fixed-rate ticker that pings a busy stream, or a
    // wrapper that keeps the response open after upstream finishes.
    #[tokio::test(start_paused = true)]
    async fn busy_stream_gets_no_ping_and_ends_with_upstream() {
        let (tx, hb) = heartbeat_over_channel();
        tokio::spawn(async move {
            for chunk in ["data: 1\n\n", "data: 2\n\n", "data: [DONE]\n\n"] {
                tokio::time::sleep(HEARTBEAT_INTERVAL / 2).await;
                send(&tx, chunk);
            }
        });
        let out: Vec<Bytes> = hb.map(Result::unwrap).collect().await;
        assert_eq!(out, ["data: 1\n\n", "data: 2\n\n", "data: [DONE]\n\n"]);
    }
}
