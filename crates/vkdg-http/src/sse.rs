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
}

impl SseParser {
    pub fn new() -> Self {
        Self { buf: Vec::new() }
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

            if let Some(event) = parse_event_bytes(&event_bytes) {
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
            return Some(EventEnd { start: i, end: i + 2 });
        }
        // Check for \r\n\r\n
        if buf[i] == b'\r'
            && i + 3 < len
            && buf[i + 1] == b'\n'
            && buf[i + 2] == b'\r'
            && buf[i + 3] == b'\n'
        {
            return Some(EventEnd { start: i, end: i + 4 });
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
            assert_eq!(all.len(), 1, "split at byte {split} produced wrong event count");
            assert_eq!(
                all[0].data,
                "{\"type\":\"output_delta\",\"index\":0,\"delta\":\"hi\"}",
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
            events[0].data,
            "line1\nline2",
            "multi-line data: fields must be joined with \\n"
        );
    }
}
