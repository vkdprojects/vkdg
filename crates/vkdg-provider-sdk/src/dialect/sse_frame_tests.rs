use super::sse_frame::{FrameError, SseFramer};
use std::borrow::Cow;

/// Owned snapshot of one framer callback.
type Seen = Result<(Option<String>, String), FrameError>;

fn collect(framer: &mut SseFramer, chunk: &[u8], out: &mut Vec<Seen>) {
    framer.push(chunk, |frame| {
        out.push(frame.map(|f| (f.event.map(str::to_owned), f.data.into_owned())));
    });
}

fn collect_finish(framer: &mut SseFramer, out: &mut Vec<Seen>) {
    framer.finish(|frame| {
        out.push(frame.map(|f| (f.event.map(str::to_owned), f.data.into_owned())));
    });
}

fn run(chunks: &[&[u8]]) -> Vec<Seen> {
    let mut framer = SseFramer::new();
    let mut out = Vec::new();
    for chunk in chunks {
        collect(&mut framer, chunk, &mut out);
    }
    collect_finish(&mut framer, &mut out);
    out
}

#[allow(clippy::unnecessary_wraps)] // mirrors the `Result` shape `Seen` carries on error
fn frame(event: Option<&str>, data: &str) -> Seen {
    Ok((event.map(str::to_owned), data.to_owned()))
}

// Fixture exercising every framing rule at once: CRLF, a multi-byte char, multi-line data,
// comment heartbeat, id/retry fields, event type, bare `data`, double space after the colon.
const FIXTURE: &str = "event: message_start\r\ndata: {\"a\":\"h\u{e9}llo \u{1f600}\"}\r\n\r\n\
: keep-alive\r\n\r\n\
id: 7\r\nretry: 1000\r\ndata: first\r\ndata: second\r\n\r\n\
event: ping\ndata\n\n\
data:  two spaces\n\n";

fn fixture_expected() -> Vec<Seen> {
    vec![
        frame(Some("message_start"), "{\"a\":\"h\u{e9}llo \u{1f600}\"}"),
        frame(None, "first\nsecond"),
        frame(Some("ping"), ""),
        frame(None, " two spaces"),
    ]
}

// Refutes: a framer that only recognises "\n\n" and misses CRLF / lone-CR / mixed terminators.
#[test]
fn every_line_terminator_style_dispatches_the_same_event() {
    for (name, raw) in [
        ("lf", "event: e\ndata: x\n\n"),
        ("crlf", "event: e\r\ndata: x\r\n\r\n"),
        ("cr", "event: e\rdata: x\r\r"),
        ("mixed", "event: e\r\ndata: x\n\r"),
        ("mixed2", "event: e\ndata: x\r\r\n"),
    ] {
        assert_eq!(
            run(&[raw.as_bytes()]),
            vec![frame(Some("e"), "x")],
            "{name}"
        );
    }
}

// Refutes: treating the LF of a CRLF as a second (blank) terminator, which would split one event
// into two or dispatch early.
#[test]
fn crlf_split_between_cr_and_lf_is_one_terminator() {
    let out = run(&[b"data: a\r", b"\ndata: b\r", b"\n\r", b"\n"]);
    assert_eq!(out, vec![frame(None, "a\nb")]);
}

// Refutes: failing to swallow the LF half of a blank-line CRLF that arrives in the next chunk,
// which would turn it into a phantom blank line / corrupt the following event's first line.
#[test]
fn lf_after_blank_line_cr_does_not_leak_into_next_event() {
    let out = run(&[b"data: a\r\n\r", b"\ndata: b\r\n\r\n"]);
    assert_eq!(out, vec![frame(None, "a"), frame(None, "b")]);
}

// Refutes: decoding UTF-8 per chunk (garbling a char cut mid-sequence), or mishandling a split
// at any byte offset.
#[test]
fn every_two_way_split_and_one_byte_chunks_match_the_unsplit_run() {
    let bytes = FIXTURE.as_bytes();
    assert_eq!(run(&[bytes]), fixture_expected());
    for cut in 0..=bytes.len() {
        let a = &bytes[..cut];
        let b = &bytes[cut..];
        assert_eq!(run(&[a, b]), fixture_expected(), "split at {cut}");
    }
    let singles: Vec<&[u8]> = bytes.chunks(1).collect();
    assert_eq!(run(&singles), fixture_expected());
}

// Refutes: `trim_start` on the value (strips more than one space) or keeping the space.
#[test]
fn only_one_leading_space_is_stripped_and_bare_data_is_empty() {
    let out = run(&[b"data:x\n\ndata: x\n\ndata:  x\n\ndata:\n\ndata\n\n"]);
    assert_eq!(
        out,
        vec![
            frame(None, "x"),
            frame(None, "x"),
            frame(None, " x"),
            frame(None, ""),
            frame(None, ""),
        ]
    );
}

// Refutes: a framer that dispatches a frame for comment heartbeats, event-only blocks or
// blank-line runs (the decoder would then see empty data and fail).
#[test]
fn blocks_without_data_lines_yield_nothing() {
    let out = run(&[b"\n\n: ping\n\nevent: only\n\nid: 3\nretry: 5\n\n\r\n\r\n: a\n: b\n\n"]);
    assert_eq!(out, Vec::<Seen>::new());
}

// Refutes: leaking `id:`/`retry:` values into data or event, or letting a comment line inside
// an event drop that event's data.
#[test]
fn id_retry_and_comment_lines_inside_an_event_are_ignored() {
    let out = run(&[b"id: 1\n: note\nretry: 10\nevent: x\ndata: y\nid: 2\n\n"]);
    assert_eq!(out, vec![frame(Some("x"), "y")]);
}

// Refutes: not recognising a field without a colon as the field name (`data` alone), and
// treating an unknown field as data.
#[test]
fn unknown_fields_are_ignored() {
    let out = run(&[b"foo: bar\ndata: y\n\n"]);
    assert_eq!(out, vec![frame(None, "y")]);
}

// Refutes: lossy-decoding per chunk or panicking on invalid UTF-8; the replacement char must
// appear once the event is complete.
#[test]
fn invalid_utf8_is_replaced_not_fatal() {
    let out = run(&[b"data: a\xffb\n\n"]);
    assert_eq!(out, vec![frame(None, "a\u{fffd}b")]);
}

// Refutes: dropping a final event whose blank line never arrived (provider closes right after
// `data: [DONE]`), or flushing nothing when only a partial line is buffered.
#[test]
fn eof_flushes_an_unterminated_trailing_event() {
    assert_eq!(
        run(&[b"data: a\n\ndata: tail\n"]),
        vec![frame(None, "a"), frame(None, "tail")]
    );
    assert_eq!(
        run(&[b"event: e\ndata: [DONE]"]),
        vec![frame(Some("e"), "[DONE]")]
    );
    assert_eq!(run(&[b"data: a\n\n"]), vec![frame(None, "a")]);
    assert_eq!(run(&[b": only a comment"]), Vec::<Seen>::new());
}

// Refutes: an unbounded buffer, and an implementation that reports the oversize error once per
// chunk of the offending event instead of once per event.
#[test]
fn oversized_event_errors_once_then_the_framer_recovers() {
    let limit = 32;
    let mut framer = SseFramer::with_max_event_bytes(limit);
    let mut out = Vec::new();
    let big = format!("data: {}\n\n", "x".repeat(200));
    for chunk in big.as_bytes().chunks(7) {
        collect(&mut framer, chunk, &mut out);
        assert!(
            framer.buffered_bytes() <= limit,
            "buffered {}",
            framer.buffered_bytes()
        );
    }
    collect(&mut framer, b"data: ok\n\n", &mut out);
    collect_finish(&mut framer, &mut out);
    assert_eq!(
        out,
        vec![Err(FrameError::EventTooLarge { limit }), frame(None, "ok")]
    );
}

// Refutes: enforcing the bound only on the buffered tail, so an over-limit event that arrives
// whole in one chunk (or whose last chunk completes it) slips through.
#[test]
fn oversized_complete_event_in_one_chunk_errors_and_neighbours_survive() {
    let limit = 32;
    let mut framer = SseFramer::with_max_event_bytes(limit);
    let mut out = Vec::new();
    let input = format!("data: a\n\ndata: {}\n\ndata: b\n\n", "y".repeat(100));
    collect(&mut framer, input.as_bytes(), &mut out);
    assert_eq!(
        out,
        vec![
            frame(None, "a"),
            Err(FrameError::EventTooLarge { limit }),
            frame(None, "b")
        ]
    );
    assert_eq!(framer.buffered_bytes(), 0);
}

// Refutes: an event split across chunks that crosses the limit only after the final chunk
// (buffered + final part > limit) being accepted.
#[test]
fn event_that_crosses_the_limit_on_its_last_chunk_is_rejected() {
    let limit = 32;
    let mut framer = SseFramer::with_max_event_bytes(limit);
    let mut out = Vec::new();
    collect(&mut framer, b"data: 0123456789012345", &mut out);
    assert_eq!(out, Vec::<Seen>::new());
    collect(&mut framer, b"0123456789\n\ndata: z\n\n", &mut out);
    assert_eq!(
        out,
        vec![Err(FrameError::EventTooLarge { limit }), frame(None, "z")]
    );
}

// Refutes: unbounded growth while discarding; memory stays within the limit however much of a
// never-ending event the upstream sends, and the error is reported exactly once.
#[test]
fn endless_event_never_buffers_beyond_the_limit() {
    let limit = 64;
    let mut framer = SseFramer::with_max_event_bytes(limit);
    let mut out = Vec::new();
    let chunk = [b'z'; 40];
    collect(&mut framer, b"data: ", &mut out);
    for _ in 0..1000 {
        collect(&mut framer, &chunk, &mut out);
        assert!(framer.buffered_bytes() <= limit);
    }
    collect_finish(&mut framer, &mut out);
    assert_eq!(out, vec![Err(FrameError::EventTooLarge { limit })]);
}

// Refutes: copying every chunk into an internal buffer (fast path), or borrowing nothing: whole
// events inside one chunk are framed straight from the slice, only the tail is retained.
#[test]
fn complete_events_are_borrowed_from_the_chunk_and_only_the_tail_is_buffered() {
    let mut framer = SseFramer::new();
    let mut borrowed = Vec::new();
    framer.push(b"data: one\n\ndata: two\n\ndata: par", |f| {
        let f = f.expect("frame");
        borrowed.push(matches!(f.data, Cow::Borrowed(_)));
    });
    assert_eq!(borrowed, vec![true, true]);
    assert_eq!(framer.buffered_bytes(), "data: par".len());
    let mut out = Vec::new();
    collect(&mut framer, b"tial\n\n", &mut out);
    assert_eq!(out, vec![frame(None, "partial")]);
    assert_eq!(framer.buffered_bytes(), 0);
}

// Refutes: multi-line data being returned borrowed with a wrong slice, or single-line data
// allocating; only joined data may own.
#[test]
fn only_multi_line_data_allocates() {
    let mut framer = SseFramer::new();
    let mut kinds = Vec::new();
    framer.push(b"data: a\ndata: b\n\ndata: c\n\n", |f| {
        kinds.push(matches!(f.expect("frame").data, Cow::Owned(_)));
    });
    assert_eq!(kinds, vec![true, false]);
}

// Refutes: a framer whose state survives `finish` (second stream on the same instance would
// start mid-event or stay in discard mode).
#[test]
fn finish_resets_the_framer_for_reuse() {
    let mut framer = SseFramer::with_max_event_bytes(16);
    let mut out = Vec::new();
    collect(
        &mut framer,
        format!("data: {}", "q".repeat(50)).as_bytes(),
        &mut out,
    );
    collect_finish(&mut framer, &mut out);
    collect(&mut framer, b"data: fresh\n\n", &mut out);
    assert_eq!(
        out,
        vec![
            Err(FrameError::EventTooLarge { limit: 16 }),
            frame(None, "fresh")
        ]
    );
}
