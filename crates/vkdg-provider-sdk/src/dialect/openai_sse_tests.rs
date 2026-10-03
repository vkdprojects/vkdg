// Expected events are written by hand from the OpenAI Chat Completions streaming
// reference and from the quirks of compatible providers (usage chunk after
// finish_reason, missing [DONE], in-band errors).

use super::event_text::{assert_events_at_every_boundary, decode, sse};
use super::openai_sse::OpenAiSseDecoder;
use crate::ConversationStreamDecoder;

fn make() -> Box<dyn ConversationStreamDecoder> {
    Box::new(OpenAiSseDecoder::new())
}

fn chunk(delta: &str, finish: &str) -> String {
    format!(
        r#"{{"id":"c1","object":"chat.completion.chunk","created":1,"model":"gpt-4o","choices":[{{"index":0,"delta":{delta},"finish_reason":{finish}}}]}}"#
    )
}

fn usage_chunk(usage: &str) -> String {
    format!(
        r#"{{"id":"c1","object":"chat.completion.chunk","created":1,"model":"gpt-4o","choices":[],"usage":{usage}}}"#
    )
}

const DONE: &str = "[DONE]";

// Refutes: completing at finish_reason (the usage chunk that follows would then be lost
// behind a terminal event), dropping the role chunk's Started, merging deltas, or
// mis-decoding multi-byte and escaped text.
#[test]
fn plain_text_stream_defers_completion_until_the_usage_chunk_and_done() {
    let input = sse(&[
        ("", &chunk(r#"{"role":"assistant","content":""}"#, "null")),
        ("", &chunk(r#"{"content":"Hello"}"#, "null")),
        (
            "",
            &chunk(r#"{"content":" w\u00f6rld \"q\" \ud83d\ude00"}"#, "null"),
        ),
        ("", &chunk("{}", r#""stop""#)),
        (
            "",
            &usage_chunk(r#"{"prompt_tokens":25,"completion_tokens":15,"total_tokens":40}"#),
        ),
        ("", DONE),
    ]);
    assert_events_at_every_boundary(
        make,
        input.as_bytes(),
        &[
            "started",
            r#"text[0] "Hello""#,
            r#"text[0] " wörld \"q\" 😀""#,
            "usage in=R25 out=R15 cache_read=- cache_create=-",
            "completed EndTurn",
        ],
    );
}

// Refutes: counting cached tokens twice (prompt_tokens already includes them) or ignoring
// them; the event model's input excludes the cache.
#[test]
fn prompt_tokens_include_cached_tokens_so_input_excludes_them() {
    let input = sse(&[
        ("", &chunk(r#"{"content":"x"}"#, r#""stop""#)),
        (
            "",
            &usage_chunk(
                r#"{"prompt_tokens":140,"completion_tokens":5,"total_tokens":145,"prompt_tokens_details":{"cached_tokens":40}}"#,
            ),
        ),
        ("", DONE),
    ]);
    assert_events_at_every_boundary(
        make,
        input.as_bytes(),
        &[
            "started",
            r#"text[0] "x""#,
            "usage in=R100 out=R5 cache_read=R40 cache_create=-",
            "completed EndTurn",
        ],
    );
}

// Refutes: reasoning leaking into the answer, or only one of the two spellings working.
#[test]
fn reasoning_content_and_reasoning_both_become_reasoning() {
    let input = sse(&[
        ("", &chunk(r#"{"reasoning_content":"think "}"#, "null")),
        ("", &chunk(r#"{"reasoning":"more"}"#, "null")),
        ("", &chunk(r#"{"content":"ok"}"#, "null")),
        ("", &chunk("{}", r#""stop""#)),
        ("", DONE),
    ]);
    assert_events_at_every_boundary(
        make,
        input.as_bytes(),
        &[
            "started",
            r#"reasoning[0] "think ""#,
            r#"reasoning[0] "more""#,
            r#"text[0] "ok""#,
            "completed EndTurn",
        ],
    );
}

// Refutes: repeating id/name on continuation fragments, buffering arguments until the end,
// not ending each call, merging parallel calls, or completing as EndTurn after tool calls.
#[test]
fn tool_calls_stream_fragments_and_end_when_the_next_call_or_the_finish_arrives() {
    let input = sse(&[
        (
            "",
            &chunk(
                r#"{"role":"assistant","content":null,"tool_calls":[{"index":0,"id":"call_a","type":"function","function":{"name":"get_weather","arguments":""}}]}"#,
                "null",
            ),
        ),
        (
            "",
            &chunk(
                r#"{"tool_calls":[{"index":0,"function":{"arguments":"{\"city\":"}}]}"#,
                "null",
            ),
        ),
        (
            "",
            &chunk(
                r#"{"tool_calls":[{"index":0,"function":{"arguments":"\"Paris\"}"}}]}"#,
                "null",
            ),
        ),
        (
            "",
            &chunk(
                r#"{"tool_calls":[{"index":1,"id":"call_b","type":"function","function":{"name":"now","arguments":"{}"}}]}"#,
                "null",
            ),
        ),
        ("", &chunk("{}", r#""tool_calls""#)),
        ("", DONE),
    ]);
    assert_events_at_every_boundary(
        make,
        input.as_bytes(),
        &[
            "started",
            r#"tool[0] id="call_a" name="get_weather" args="""#,
            r#"tool[0] id="" name="" args="{\"city\":""#,
            r#"tool[0] id="" name="" args="\"Paris\"}""#,
            "tool_end[0]",
            r#"tool[1] id="call_b" name="now" args="{}""#,
            "tool_end[1]",
            "completed ToolUse",
        ],
    );
}

// Refutes: a wrong finish_reason table (the legacy function_call spelling, content_filter).
#[test]
fn finish_reasons_map_per_contract() {
    for (wire, expected) in [
        ("stop", "completed EndTurn"),
        ("length", "completed MaxTokens"),
        ("tool_calls", "completed ToolUse"),
        ("function_call", "completed ToolUse"),
        ("content_filter", "completed EndTurn"),
        ("something_new", "completed EndTurn"),
    ] {
        let input = sse(&[
            ("", &chunk(r#"{"content":"x"}"#, &format!("\"{wire}\""))),
            ("", DONE),
        ]);
        let events = decode(make().as_mut(), [input.as_bytes()]);
        assert_eq!(events.last().map(String::as_str), Some(expected), "{wire}");
    }
}

// Refutes: requiring [DONE] (several compatible providers omit it) or failing a complete stream.
#[test]
fn a_recorded_finish_reason_completes_at_eof_without_done() {
    let input = sse(&[
        ("", &chunk(r#"{"content":"ok"}"#, "null")),
        ("", &chunk("{}", r#""length""#)),
    ]);
    assert_events_at_every_boundary(
        make,
        input.as_bytes(),
        &["started", r#"text[0] "ok""#, "completed MaxTokens"],
    );
}

// Refutes: surfacing choices other than the first as interleaved text.
#[test]
fn only_the_first_choice_is_decoded() {
    let second =
        r#"{"id":"c1","choices":[{"index":1,"delta":{"content":"other"},"finish_reason":null}]}"#;
    let input = sse(&[
        ("", &chunk(r#"{"content":"a"}"#, "null")),
        ("", second),
        ("", &chunk("{}", r#""stop""#)),
        ("", DONE),
    ]);
    assert_events_at_every_boundary(
        make,
        input.as_bytes(),
        &["started", r#"text[0] "a""#, "completed EndTurn"],
    );
}

// Refutes: swallowing an in-band error, mapping every one to 502, echoing nothing useful,
// or continuing after Failed.
#[test]
fn in_band_error_object_fails_with_a_mapped_status_and_ends_the_stream() {
    for (kind, status) in [
        ("invalid_request_error", 400),
        ("authentication_error", 401),
        ("permission_error", 403),
        ("rate_limit_error", 429),
        ("server_error", 500),
        ("something_new", 502),
    ] {
        let error = format!(r#"{{"error":{{"message":"nope","type":"{kind}","code":null}}}}"#);
        let input = sse(&[
            ("", &chunk(r#"{"content":"partial"}"#, "null")),
            ("", &error),
            ("", &chunk(r#"{"content":"late"}"#, "null")),
            ("", DONE),
        ]);
        let events = decode(make().as_mut(), [input.as_bytes()]);
        assert_eq!(
            events,
            vec![
                "started".to_owned(),
                r#"text[0] "partial""#.to_owned(),
                format!(r#"failed {status} "nope""#),
            ],
            "{kind}"
        );
    }
}

// Refutes: a silent Completed for a stream that never produced anything.
#[test]
fn empty_and_done_only_streams_fail() {
    let expected = [r#"failed 502 "upstream returned an empty stream""#];
    assert_events_at_every_boundary(make, b"", &expected);
    let done_only = sse(&[("", DONE)]);
    assert_events_at_every_boundary(make, done_only.as_bytes(), &expected);
    let comments = ": keep-alive\n\n: keep-alive\n\n";
    assert_events_at_every_boundary(make, comments.as_bytes(), &expected);
}

// Refutes: turning a truncated stream (no finish_reason, no [DONE]) into a clean Completed,
// which makes clients run half-written tool JSON; a cut inside the last event is truncation,
// not a malformed event.
#[test]
fn truncated_streams_fail_instead_of_completing() {
    let mut input = sse(&[("", &chunk(r#"{"content":"half"}"#, "null"))]);
    let expected = [
        "started",
        r#"text[0] "half""#,
        r#"failed 502 "upstream stream ended before completion""#,
    ];
    assert_events_at_every_boundary(make, input.as_bytes(), &expected);
    input.push_str("data: {\"choices\":[{\"index\":0,\"del");
    assert_events_at_every_boundary(make, input.as_bytes(), &expected);
}

// Refutes: echoing the upstream payload in the failure (it may carry user content),
// panicking on bad JSON, or carrying on after a protocol violation.
#[test]
fn malformed_json_fails_with_a_fixed_message_and_stops_decoding() {
    let input = sse(&[
        ("", &chunk(r#"{"content":"a"}"#, "null")),
        ("", r#"{"choices":[{"delta": SECRET-USER-TEXT"#),
        ("", &chunk(r#"{"content":"late"}"#, "null")),
        ("", DONE),
    ]);
    assert_events_at_every_boundary(
        make,
        input.as_bytes(),
        &[
            "started",
            r#"text[0] "a""#,
            r#"failed 502 "upstream sent a malformed openai stream event""#,
        ],
    );
}

// Refutes: an event over the framer bound passing through, or a fixed message that leaks the
// framer's internal text.
#[test]
fn oversized_sse_event_fails_the_stream() {
    let big = chunk(&format!(r#"{{"content":"{}"}}"#, "x".repeat(300)), "null");
    let input = sse(&[
        ("", &chunk(r#"{"content":"a"}"#, "null")),
        ("", &big),
        ("", DONE),
    ]);
    let make_small = || -> Box<dyn ConversationStreamDecoder> {
        Box::new(OpenAiSseDecoder::with_limits(256, 1 << 20))
    };
    assert_events_at_every_boundary(
        make_small,
        input.as_bytes(),
        &[
            "started",
            r#"text[0] "a""#,
            r#"failed 502 "upstream sent an SSE event larger than the gateway limit""#,
        ],
    );
}

// Refutes: an unbounded per-call argument buffer: the call fails at the first byte over the
// limit and the offending fragment is not forwarded; exactly at the limit is allowed.
#[test]
fn tool_arguments_over_the_limit_fail_the_stream() {
    let frag = |args: &str| {
        chunk(
            &format!(r#"{{"tool_calls":[{{"index":0,"function":{{"arguments":"{args}"}}}}]}}"#),
            "null",
        )
    };
    let start = chunk(
        r#"{"tool_calls":[{"index":0,"id":"c1","type":"function","function":{"name":"f","arguments":""}}]}"#,
        "null",
    );
    let make_small = || -> Box<dyn ConversationStreamDecoder> {
        Box::new(OpenAiSseDecoder::with_limits(1 << 20, 10))
    };
    let at_limit = sse(&[
        ("", &start),
        ("", &frag("12345")),
        ("", &frag("67890")),
        ("", &chunk("{}", r#""tool_calls""#)),
        ("", DONE),
    ]);
    assert_events_at_every_boundary(
        make_small,
        at_limit.as_bytes(),
        &[
            "started",
            r#"tool[0] id="c1" name="f" args="""#,
            r#"tool[0] id="" name="" args="12345""#,
            r#"tool[0] id="" name="" args="67890""#,
            "tool_end[0]",
            "completed ToolUse",
        ],
    );
    let over = sse(&[
        ("", &start),
        ("", &frag("12345")),
        ("", &frag("678901")),
        ("", DONE),
    ]);
    assert_events_at_every_boundary(
        make_small,
        over.as_bytes(),
        &[
            "started",
            r#"tool[0] id="c1" name="f" args="""#,
            r#"tool[0] id="" name="" args="12345""#,
            r#"failed 502 "upstream tool call arguments exceeded the gateway limit""#,
        ],
    );
}

// Refutes: emitting anything after Completed (a trailing usage chunk, a second [DONE], or
// finish()) which would duplicate the terminal event downstream.
#[test]
fn nothing_is_emitted_after_completed() {
    let mut decoder = make();
    let first = sse(&[("", &chunk(r#"{"content":"a"}"#, r#""stop""#)), ("", DONE)]);
    let events = decode(decoder.as_mut(), [first.as_bytes()]);
    assert_eq!(events.last().map(String::as_str), Some("completed EndTurn"));
    let more = sse(&[
        ("", &chunk(r#"{"content":"x"}"#, "null")),
        (
            "",
            &usage_chunk(r#"{"prompt_tokens":1,"completion_tokens":1}"#),
        ),
        ("", DONE),
    ]);
    assert_eq!(
        decode(decoder.as_mut(), [more.as_bytes()]),
        Vec::<String>::new()
    );
}
