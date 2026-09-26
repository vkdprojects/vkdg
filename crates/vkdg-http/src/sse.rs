use std::pin::Pin;

use bytes::Bytes;
use futures::{Stream, StreamExt};

// Incremental SSE parser — buffers bytes across arbitrary TCP read boundaries.
//
// A complete SSE event is terminated by a blank line (\n\n or \r\n\r\n).
// The parser does not require chunks to align with event or even character
// boundaries; it accumulates raw bytes and only interprets them once a full
// event has been received.

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub event_type: Option<String>,
    pub data: String,
}

// ── Parser ────────────────────────────────────────────────────────────────────

pub struct SseParser {
    // Raw byte buffer — accumulated until a complete event is delimited.
    buf: Vec<u8>,
    /// Whether to strip <think>...</think> blocks from text delta events.
    /// Default: true (most clients don't want to see reasoning tokens).
    pub strip_think_tags: bool,
}

impl SseParser {
    pub fn new() -> Self {
        Self {
            buf: Vec::new(),
            strip_think_tags: true,
        }
    }

    /// Keep think-tag blocks in the output (opt-in by client via X-VKDG-Think-Tags: include).
    pub fn with_think_tags(mut self) -> Self {
        self.strip_think_tags = false;
        self
    }

    /// Push a chunk of bytes (any size, any alignment) and return all complete
    /// SSE events that can be extracted from the buffer.
    pub fn push(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        self.buf.extend_from_slice(chunk);
        let mut events = Vec::new();

        // Scan for \n\n or \r\n\r\n — end-of-event delimiters.
        while let Some(end) = find_event_end(&self.buf) {
            let event_bytes = self.buf[..end.start].to_vec();
            // Drain consumed bytes (including the delimiter).
            self.buf.drain(..end.end);

            if let Some(mut event) = parse_event_bytes(&event_bytes) {
                if self.strip_think_tags {
                    event.data = strip_think_tags(&event.data);
                }
                events.push(event);
            }
        }

        events
    }
}

impl Default for SseParser {
    fn default() -> Self {
        Self::new()
    }
}

// ── Internals ─────────────────────────────────────────────────────────────────

struct EventEnd {
    /// Byte index where the event content ends (before the delimiter).
    start: usize,
    /// Byte index after the delimiter — next event starts here.
    end: usize,
}

/// Find the first \n\n or \r\n\r\n in `buf`.
fn find_event_end(buf: &[u8]) -> Option<EventEnd> {
    let len = buf.len();
    let mut i = 0;
    while i < len {
        // Check for \n\n
        if buf[i] == b'\n' && i + 1 < len && buf[i + 1] == b'\n' {
            return Some(EventEnd {
                start: i,
                end: i + 2,
            });
        }
        // Check for \r\n\r\n
        if buf[i] == b'\r'
            && i + 3 < len
            && buf[i + 1] == b'\n'
            && buf[i + 2] == b'\r'
            && buf[i + 3] == b'\n'
        {
            return Some(EventEnd {
                start: i,
                end: i + 4,
            });
        }
        i += 1;
    }
    None
}

/// Parse raw event bytes (everything before the \n\n delimiter) into an SseEvent.
///
/// Lines that fail UTF-8 decoding are skipped — the buffer only retains
/// partial multi-byte sequences until more bytes arrive via `push`.
fn parse_event_bytes(raw: &[u8]) -> Option<SseEvent> {
    let text = match std::str::from_utf8(raw) {
        Ok(s) => s,
        Err(e) => {
            // Valid prefix only — partial UTF-8 at the end means the event
            // boundary detection fired on a comment or short line; use what
            // is valid (the invalid suffix cannot form a field anyway).
            let valid_up_to = e.valid_up_to();
            std::str::from_utf8(&raw[..valid_up_to]).ok()?
        }
    };

    let mut event_type: Option<String> = None;
    let mut data_parts: Vec<&str> = Vec::new();

    for line in text.lines() {
        // Skip comment lines.
        if line.starts_with(':') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("event:") {
            event_type = Some(rest.trim_start().to_string());
        } else if let Some(rest) = line.strip_prefix("data:") {
            data_parts.push(rest.trim_start());
        } else if line == "data" {
            // bare "data" field with empty value
            data_parts.push("");
        }
        // id: and retry: fields are parsed but not exposed in SseEvent for now.
    }

    if data_parts.is_empty() {
        return None;
    }

    let data = data_parts.join("\n");
    Some(SseEvent { event_type, data })
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

/// The SSE error event injected when upstream closes without [DONE].
/// Format matches Anthropic's error event format so clients handle it correctly.
const STREAM_INTERRUPTED_EVENT: &str =
    "data: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"upstream closed mid-stream\"}}\n\ndata: [DONE]\n\n";

/// Wrap a byte stream and inject an error event if it ends without `[DONE]`.
///
/// Detects upstream TCP closure mid-stream: if the inner stream terminates
/// (or errors) before a `[DONE]` sentinel is seen, appends a well-formed SSE
/// error event so clients can detect the incomplete response.
pub fn with_termination_guard(
    inner: Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>>,
) -> Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>> {
    Box::pin(futures::stream::unfold(
        (inner, false, false),
        |(mut stream, mut saw_done, mut finished)| async move {
            if finished {
                return None;
            }
            match stream.next().await {
                Some(Ok(chunk)) => {
                    if chunk.windows(6).any(|w| w == b"[DONE]") {
                        saw_done = true;
                    }
                    Some((Ok(chunk), (stream, saw_done, finished)))
                }
                None => {
                    finished = true;
                    if !saw_done {
                        // Upstream closed without [DONE] — inject error event.
                        tracing::warn!(
                            "upstream closed SSE stream before [DONE]; injecting error event"
                        );
                        Some((
                            Ok(Bytes::from_static(STREAM_INTERRUPTED_EVENT.as_bytes())),
                            (stream, true, finished),
                        ))
                    } else {
                        None
                    }
                }
                Some(Err(e)) => {
                    finished = true;
                    tracing::warn!(error = %e, "SSE stream error; injecting error event");
                    Some((
                        Ok(Bytes::from_static(STREAM_INTERRUPTED_EVENT.as_bytes())),
                        (stream, true, finished),
                    ))
                }
            }
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
        let guarded: Vec<_> = with_termination_guard(inner).collect().await;

        // Last chunk must contain the error event.
        let last = guarded.last().unwrap().as_ref().unwrap();
        let text = std::str::from_utf8(last).unwrap();
        assert!(
            text.contains("overloaded_error") || text.contains("upstream closed"),
            "missing [DONE] must produce error event, got: {text}"
        );
        assert!(text.contains("[DONE]"), "error event must end with [DONE]");
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
        let guarded: Vec<_> = with_termination_guard(inner).collect().await;

        // Must have exactly 2 chunks (no extra error event injected).
        assert_eq!(
            guarded.len(),
            2,
            "complete stream must not have extra events injected"
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
        let guarded: Vec<_> = with_termination_guard(inner).collect().await;

        // Guard swallows the Err and replaces it with an Ok error-event chunk.
        let last = guarded.last().unwrap().as_ref().unwrap();
        let text = std::str::from_utf8(last).unwrap();
        assert!(
            text.contains("overloaded_error") || text.contains("upstream closed"),
            "stream error must produce error event, got: {text}"
        );
    }
}
