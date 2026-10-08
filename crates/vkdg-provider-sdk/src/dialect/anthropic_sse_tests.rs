use super::anthropic_sse::AnthropicSseDecoder;
use super::event_text::{assert_events_at_every_boundary, decode, sse};
use crate::ConversationStreamDecoder;

fn make() -> Box<dyn ConversationStreamDecoder> {
    Box::new(AnthropicSseDecoder::new())
}

const START: &str = r#"{"type":"message_start","message":{"id":"msg_01","type":"message","role":"assistant","model":"claude-sonnet-4-5","content":[],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":25,"output_tokens":1}}}"#;
const STOP: &str = r#"{"type":"message_stop"}"#;

fn text_start(index: u32) -> String {
    format!(
        r#"{{"type":"content_block_start","index":{index},"content_block":{{"type":"text","text":""}}}}"#
    )
}

fn text_delta(index: u32, text: &str) -> String {
    format!(
        r#"{{"type":"content_block_delta","index":{index},"delta":{{"type":"text_delta","text":"{text}"}}}}"#
    )
}

fn block_stop(index: u32) -> String {
    format!(r#"{{"type":"content_block_stop","index":{index}}}"#)
}

fn message_delta(stop_reason: &str, usage: &str) -> String {
    format!(
        r#"{{"type":"message_delta","delta":{{"stop_reason":"{stop_reason}","stop_sequence":null}},"usage":{usage}}}"#
    )
}

// Refutes: merging deltas, dropping the start usage, losing multi-byte/escaped text, or
// completing before the message_delta usage is reported.
#[test]
fn plain_text_stream_maps_one_event_per_upstream_event() {
    let input = sse(&[
        ("message_start", START),
        ("content_block_start", &text_start(0)),
        ("ping", r#"{"type": "ping"}"#),
        ("content_block_delta", &text_delta(0, "Hello")),
        (
            "content_block_delta",
            &text_delta(0, " w\u{f6}rld \\\"quoted\\\" \u{1f600}"),
        ),
        ("content_block_stop", &block_stop(0)),
        (
            "message_delta",
            &message_delta("end_turn", r#"{"output_tokens":15}"#),
        ),
        ("message_stop", STOP),
    ]);
    assert_events_at_every_boundary(
        make,
        input.as_bytes(),
        &[
            "started",
            "usage in=R25 out=R1 cache_read=- cache_create=-",
            r#"text[0] "Hello""#,
            r#"text[0] " wörld \"quoted\" 😀""#,
            "usage in=R25 out=R15 cache_read=- cache_create=-",
            "completed EndTurn",
        ],
    );
}

// Refutes: text that already rides on content_block_start (some proxies) being dropped.
#[test]
fn text_carried_on_block_start_is_emitted() {
    let input = sse(&[
        ("message_start", START),
        (
            "content_block_start",
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":"Hi"}}"#,
        ),
        (
            "message_delta",
            &message_delta("end_turn", r#"{"output_tokens":2}"#),
        ),
        ("message_stop", STOP),
    ]);
    assert_events_at_every_boundary(
        make,
        input.as_bytes(),
        &[
            "started",
            "usage in=R25 out=R1 cache_read=- cache_create=-",
            r#"text[0] "Hi""#,
            "usage in=R25 out=R2 cache_read=- cache_create=-",
            "completed EndTurn",
        ],
    );
}

// Refutes: mixing thinking into text, leaking the signature (or fabricating one), surfacing
// redacted_thinking, and renumbering the source block index.
#[test]
fn thinking_becomes_reasoning_and_signature_and_redacted_blocks_are_dropped() {
    let input = sse(&[
        ("message_start", START),
        (
            "content_block_start",
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}}"#,
        ),
        (
            "content_block_delta",
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"Let me think"}}"#,
        ),
        (
            "content_block_delta",
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":" more"}}"#,
        ),
        (
            "content_block_delta",
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"EqQBCkYIARgCKkDSECRET"}}"#,
        ),
        ("content_block_stop", &block_stop(0)),
        (
            "content_block_start",
            r#"{"type":"content_block_start","index":1,"content_block":{"type":"redacted_thinking","data":"EmwKAhgBSECRET"}}"#,
        ),
        ("content_block_stop", &block_stop(1)),
        ("content_block_start", &text_start(2)),
        ("content_block_delta", &text_delta(2, "Answer")),
        ("content_block_stop", &block_stop(2)),
        (
            "message_delta",
            &message_delta("end_turn", r#"{"output_tokens":30}"#),
        ),
        ("message_stop", STOP),
    ]);
    assert_events_at_every_boundary(
        make,
        input.as_bytes(),
        &[
            "started",
            "usage in=R25 out=R1 cache_read=- cache_create=-",
            r#"reasoning[0] "Let me think""#,
            r#"reasoning[0] " more""#,
            r#"text[2] "Answer""#,
            "usage in=R25 out=R30 cache_read=- cache_create=-",
            "completed EndTurn",
        ],
    );
}

// Refutes: buffering tool JSON until the block closes, repeating id/name on later fragments,
// emitting a fragment for the empty opening partial_json, or ending the call early/never.
#[test]
fn tool_use_streams_fragments_unbuffered_and_ends_at_block_stop() {
    let input = sse(&[
        ("message_start", START),
        ("content_block_start", &text_start(0)),
        ("content_block_delta", &text_delta(0, "I'll check")),
        ("content_block_stop", &block_stop(0)),
        (
            "content_block_start",
            r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_01A","name":"get_weather","input":{}}}"#,
        ),
        (
            "content_block_delta",
            r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":""}}"#,
        ),
        (
            "content_block_delta",
            r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"city\": \"Pa"}}"#,
        ),
        (
            "content_block_delta",
            r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"ris\", \"unit\""}}"#,
        ),
        (
            "content_block_delta",
            r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":": \"c\"}"}}"#,
        ),
        ("content_block_stop", &block_stop(1)),
        (
            "message_delta",
            &message_delta("tool_use", r#"{"output_tokens":60}"#),
        ),
        ("message_stop", STOP),
    ]);
    assert_events_at_every_boundary(
        make,
        input.as_bytes(),
        &[
            "started",
            "usage in=R25 out=R1 cache_read=- cache_create=-",
            r#"text[0] "I'll check""#,
            r#"tool[1] id="toolu_01A" name="get_weather" args="""#,
            r#"tool[1] id="" name="" args="{\"city\": \"Pa""#,
            r#"tool[1] id="" name="" args="ris\", \"unit\"""#,
            r#"tool[1] id="" name="" args=": \"c\"}""#,
            "tool_end[1]",
            "usage in=R25 out=R60 cache_read=- cache_create=-",
            "completed ToolUse",
        ],
    );
}

// Refutes: the message_delta usage (output only) overwriting the start usage with zero/Unknown
// (the input and cache counts would vanish), reading the cache_creation object form as the
// token count, and ignoring cache fields.
#[test]
fn cache_usage_survives_an_output_only_message_delta() {
    let start = r#"{"type":"message_start","message":{"id":"m","usage":{"input_tokens":10,"cache_creation_input_tokens":100,"cache_read_input_tokens":200,"cache_creation":{"ephemeral_5m_input_tokens":60,"ephemeral_1h_input_tokens":40},"output_tokens":1}}}"#;
    let input = sse(&[
        ("message_start", start),
        (
            "message_delta",
            &message_delta("end_turn", r#"{"output_tokens":42}"#),
        ),
        ("message_stop", STOP),
    ]);
    assert_events_at_every_boundary(
        make,
        input.as_bytes(),
        &[
            "started",
            "usage in=R10 out=R1 cache_read=R200 cache_create=R100",
            "usage in=R10 out=R42 cache_read=R200 cache_create=R100",
            "completed EndTurn",
        ],
    );
}

// Refutes: ignoring input/cache fields on message_delta (reported cumulatively since 2025), so
// final input counts stay at the placeholder from message_start.
#[test]
fn message_delta_with_full_usage_replaces_every_reported_field() {
    let start = r#"{"type":"message_start","message":{"id":"m","usage":{"input_tokens":10,"output_tokens":1}}}"#;
    let delta_usage = r#"{"input_tokens":12,"output_tokens":9,"cache_read_input_tokens":3,"cache_creation_input_tokens":4}"#;
    let input = sse(&[
        ("message_start", start),
        ("message_delta", &message_delta("end_turn", delta_usage)),
        ("message_stop", STOP),
    ]);
    assert_events_at_every_boundary(
        make,
        input.as_bytes(),
        &[
            "started",
            "usage in=R10 out=R1 cache_read=- cache_create=-",
            "usage in=R12 out=R9 cache_read=R3 cache_create=R4",
            "completed EndTurn",
        ],
    );
}

// Refutes: a wrong stop-reason table (pause_turn/refusal must not fail or invent a variant;
// stop_sequence must stay distinguishable from end_turn).
#[test]
fn stop_reasons_map_per_contract() {
    for (wire, expected) in [
        ("end_turn", "completed EndTurn"),
        ("max_tokens", "completed MaxTokens"),
        ("stop_sequence", "completed StopSequence"),
        ("tool_use", "completed ToolUse"),
        ("pause_turn", "completed EndTurn"),
        ("refusal", "completed EndTurn"),
        ("something_new", "completed EndTurn"),
    ] {
        let input = sse(&[
            ("message_start", START),
            (
                "message_delta",
                &message_delta(wire, r#"{"output_tokens":2}"#),
            ),
            ("message_stop", STOP),
        ]);
        let events = decode(make().as_mut(), [input.as_bytes()]);
        assert_eq!(events.last().map(String::as_str), Some(expected), "{wire}");
    }
}

// Refutes: swallowing the upstream error, losing its code, continuing after Failed (a late
// message_stop must not turn into Completed), or echoing nothing useful.
#[test]
fn error_event_fails_with_mapped_code_and_ends_the_stream() {
    let input = sse(&[
        ("message_start", START),
        ("content_block_start", &text_start(0)),
        ("content_block_delta", &text_delta(0, "partial")),
        (
            "error",
            r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#,
        ),
        ("content_block_delta", &text_delta(0, "ignored")),
        ("message_stop", STOP),
    ]);
    assert_events_at_every_boundary(
        make,
        input.as_bytes(),
        &[
            "started",
            "usage in=R25 out=R1 cache_read=- cache_create=-",
            r#"text[0] "partial""#,
            r#"failed 529 "Overloaded""#,
        ],
    );
}

// Refutes: a wrong error-type table, and an untruncated provider message (diagnostics are
// capped at 300 characters, counted as characters not bytes).
#[test]
fn upstream_error_types_map_to_codes_and_messages_are_capped() {
    for (kind, code) in [
        ("invalid_request_error", 400),
        ("authentication_error", 401),
        ("billing_error", 402),
        ("permission_error", 403),
        ("not_found_error", 404),
        ("request_too_large", 413),
        ("rate_limit_error", 429),
        ("api_error", 500),
        ("timeout_error", 504),
        ("overloaded_error", 529),
        ("brand_new_error", 502),
    ] {
        let data = format!(r#"{{"type":"error","error":{{"type":"{kind}","message":"nope"}}}}"#);
        let input = sse(&[("error", &data)]);
        let events = decode(make().as_mut(), [input.as_bytes()]);
        assert_eq!(events, vec![format!(r#"failed {code} "nope""#)], "{kind}");
    }
    let long = "\u{e9}".repeat(400);
    let data = format!(r#"{{"type":"error","error":{{"type":"api_error","message":"{long}"}}}}"#);
    let events = decode(make().as_mut(), [sse(&[("error", &data)]).as_bytes()]);
    assert_eq!(
        events,
        vec![format!("failed 500 {:?}", "\u{e9}".repeat(300))]
    );
}

// Refutes: a silent Completed for a stream that never produced anything.
#[test]
fn empty_and_ping_only_streams_fail() {
    let expected = [r#"failed 502 "upstream returned an empty stream""#];
    assert_events_at_every_boundary(make, b"", &expected);
    let pings = sse(&[("ping", r#"{"type":"ping"}"#)]);
    assert_events_at_every_boundary(make, pings.as_bytes(), &expected);
}

// Refutes: turning a truncated stream (no stop reason, no message_stop) into a clean
// Completed, which makes clients run half-written tool JSON; also covers a cut inside the
// last event, which must read as truncation and not as a malformed event.
#[test]
fn truncated_streams_fail_instead_of_completing() {
    let mut input = sse(&[
        ("message_start", START),
        ("content_block_start", &text_start(0)),
        ("content_block_delta", &text_delta(0, "half")),
    ]);
    let expected = [
        "started",
        "usage in=R25 out=R1 cache_read=- cache_create=-",
        r#"text[0] "half""#,
        r#"failed 502 "upstream stream ended before completion""#,
    ];
    assert_events_at_every_boundary(make, input.as_bytes(), &expected);
    input.push_str("event: content_block_delta\ndata: {\"type\":\"content_block_del");
    assert_events_at_every_boundary(make, input.as_bytes(), &expected);
}

// Refutes: requiring message_stop even though a stop_reason was already reported (some
// gateways drop the last frame), and failing a stream that is in fact complete.
#[test]
fn recorded_stop_reason_completes_at_eof_without_message_stop() {
    let input = sse(&[
        ("message_start", START),
        ("content_block_start", &text_start(0)),
        ("content_block_delta", &text_delta(0, "ok")),
        ("content_block_stop", &block_stop(0)),
        (
            "message_delta",
            &message_delta("max_tokens", r#"{"output_tokens":4}"#),
        ),
    ]);
    assert_events_at_every_boundary(
        make,
        input.as_bytes(),
        &[
            "started",
            "usage in=R25 out=R1 cache_read=- cache_create=-",
            r#"text[0] "ok""#,
            "usage in=R25 out=R4 cache_read=- cache_create=-",
            "completed MaxTokens",
        ],
    );
}

// Refutes: defaulting message_stop-without-reason to a fixed reason regardless of tools.
#[test]
fn message_stop_without_stop_reason_depends_on_tool_calls() {
    let tool = sse(&[
        ("message_start", START),
        (
            "content_block_start",
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_9","name":"f","input":{}}}"#,
        ),
        ("content_block_stop", &block_stop(0)),
        ("message_stop", STOP),
    ]);
    assert_events_at_every_boundary(
        make,
        tool.as_bytes(),
        &[
            "started",
            "usage in=R25 out=R1 cache_read=- cache_create=-",
            r#"tool[0] id="toolu_9" name="f" args="""#,
            "tool_end[0]",
            "completed ToolUse",
        ],
    );
    let text = sse(&[("message_start", START), ("message_stop", STOP)]);
    assert_events_at_every_boundary(
        make,
        text.as_bytes(),
        &[
            "started",
            "usage in=R25 out=R1 cache_read=- cache_create=-",
            "completed EndTurn",
        ],
    );
}

// Refutes: echoing the upstream payload in the failure (it may carry user content), panicking
// on bad JSON, or carrying on after a protocol violation.
#[test]
fn malformed_json_fails_with_a_fixed_message_and_stops_decoding() {
    let input = sse(&[
        ("message_start", START),
        (
            "content_block_delta",
            r#"{"type":"content_block_delta","index":0,"delta": SECRET-USER-TEXT"#,
        ),
        ("content_block_delta", &text_delta(0, "late")),
        ("message_stop", STOP),
    ]);
    assert_events_at_every_boundary(
        make,
        input.as_bytes(),
        &[
            "started",
            "usage in=R25 out=R1 cache_read=- cache_create=-",
            r#"failed 502 "upstream sent a malformed anthropic stream event""#,
        ],
    );
}

// Refutes: an event over the framer bound passing through, or being reported with the
// internal framer text; the failure message is fixed and terminal.
#[test]
fn oversized_sse_event_fails_the_stream() {
    let big = text_delta(0, &"x".repeat(200));
    let input = sse(&[
        ("message_start", START),
        ("content_block_delta", &big),
        ("message_stop", STOP),
    ]);
    let make_small = || -> Box<dyn ConversationStreamDecoder> {
        // message_start is 240 bytes on the wire (event line included): the limit must let
        // it through and still stop the 270-byte delta.
        Box::new(AnthropicSseDecoder::with_limits(256, 1 << 20))
    };
    assert_events_at_every_boundary(
        make_small,
        input.as_bytes(),
        &[
            "started",
            "usage in=R25 out=R1 cache_read=- cache_create=-",
            r#"failed 502 "upstream sent an SSE event larger than the gateway limit""#,
        ],
    );
}

// Refutes: an unbounded per-call argument buffer: the call fails at the first byte over the
// limit (and the offending fragment is not forwarded), exactly-at-limit is still allowed.
#[test]
fn tool_arguments_over_the_limit_fail_the_stream() {
    let tool_start = r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_1","name":"f","input":{}}}"#;
    let frag = |s: &str| {
        format!(
            r#"{{"type":"content_block_delta","index":0,"delta":{{"type":"input_json_delta","partial_json":"{s}"}}}}"#
        )
    };
    let make_small = || -> Box<dyn ConversationStreamDecoder> {
        Box::new(AnthropicSseDecoder::with_limits(1 << 20, 10))
    };
    let at_limit = sse(&[
        ("message_start", START),
        ("content_block_start", tool_start),
        ("content_block_delta", &frag("12345")),
        ("content_block_delta", &frag("67890")),
        ("content_block_stop", &block_stop(0)),
        ("message_stop", STOP),
    ]);
    assert_events_at_every_boundary(
        make_small,
        at_limit.as_bytes(),
        &[
            "started",
            "usage in=R25 out=R1 cache_read=- cache_create=-",
            r#"tool[0] id="toolu_1" name="f" args="""#,
            r#"tool[0] id="" name="" args="12345""#,
            r#"tool[0] id="" name="" args="67890""#,
            "tool_end[0]",
            "completed ToolUse",
        ],
    );
    let over = sse(&[
        ("message_start", START),
        ("content_block_start", tool_start),
        ("content_block_delta", &frag("12345")),
        ("content_block_delta", &frag("678901")),
        ("content_block_stop", &block_stop(0)),
        ("message_stop", STOP),
    ]);
    assert_events_at_every_boundary(
        make_small,
        over.as_bytes(),
        &[
            "started",
            "usage in=R25 out=R1 cache_read=- cache_create=-",
            r#"tool[0] id="toolu_1" name="f" args="""#,
            r#"tool[0] id="" name="" args="12345""#,
            r#"failed 502 "upstream tool call arguments exceeded the gateway limit""#,
        ],
    );
}

// Refutes: surfacing server-side tool blocks (web_search) as client tool calls, letting them
// flip the stop reason, or choking on unknown block/event types.
#[test]
fn server_tool_blocks_and_unknown_events_are_dropped() {
    let input = sse(&[
        ("message_start", START),
        (
            "content_block_start",
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"server_tool_use","id":"srvtoolu_1","name":"web_search","input":{}}}"#,
        ),
        (
            "content_block_delta",
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"query\":\"x\"}"}}"#,
        ),
        ("content_block_stop", &block_stop(0)),
        (
            "content_block_start",
            r#"{"type":"content_block_start","index":1,"content_block":{"type":"web_search_tool_result","tool_use_id":"srvtoolu_1","content":[{"type":"web_search_result","url":"https://example.com","title":"t"}]}}"#,
        ),
        ("content_block_stop", &block_stop(1)),
        (
            "future_event",
            r#"{"type":"future_event","payload":{"a":[1,2,3]}}"#,
        ),
        ("content_block_start", &text_start(2)),
        ("content_block_delta", &text_delta(2, "Result")),
        (
            "content_block_delta",
            r#"{"type":"content_block_delta","index":2,"delta":{"type":"citations_delta","citation":{"type":"web_search_result_location"}}}"#,
        ),
        ("content_block_stop", &block_stop(2)),
        ("message_stop", STOP),
    ]);
    assert_events_at_every_boundary(
        make,
        input.as_bytes(),
        &[
            "started",
            "usage in=R25 out=R1 cache_read=- cache_create=-",
            r#"text[2] "Result""#,
            "completed EndTurn",
        ],
    );
}

// Refutes: requiring message_start (Started must still come first, before any content), and
// ignoring the data-less `type` fallback to the SSE event name.
#[test]
fn started_is_emitted_lazily_and_event_name_is_the_fallback_type() {
    let input = "event: content_block_delta\ndata: {\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"x\"}}\n\nevent: message_stop\ndata: {}\n\n";
    assert_events_at_every_boundary(
        make,
        input.as_bytes(),
        &["started", r#"text[0] "x""#, "completed EndTurn"],
    );
}

// Refutes: emitting anything after Completed (a second message_stop, trailing frames, or
// finish()) which would duplicate the terminal event downstream.
#[test]
fn nothing_is_emitted_after_completed() {
    let mut decoder = make();
    let first = sse(&[("message_start", START), ("message_stop", STOP)]);
    let events = decode(decoder.as_mut(), [first.as_bytes()]);
    assert_eq!(events.last().map(String::as_str), Some("completed EndTurn"));
    let more = sse(&[
        ("content_block_delta", &text_delta(0, "x")),
        ("message_stop", STOP),
    ]);
    assert_eq!(
        decode(decoder.as_mut(), [more.as_bytes()]),
        Vec::<String>::new()
    );
}
