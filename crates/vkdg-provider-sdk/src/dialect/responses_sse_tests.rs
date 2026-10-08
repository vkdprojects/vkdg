use bytes::Bytes;

use super::event_text::{assert_events_at_every_boundary, decode, render_all, sse};
use super::ResponsesSseDecoder;
use crate::ConversationStreamDecoder;

fn make() -> Box<dyn ConversationStreamDecoder> {
    Box::new(ResponsesSseDecoder::new())
}

// Refutes raw Responses passthrough, reasoning mixed into text, premature text.done
// completion, double-counted cached tokens, and UTF-8 fragmentation corruption.
#[test]
fn responses_text_reasoning_usage_and_terminal_at_every_byte_boundary() {
    let input = sse(&[
        (
            "response.created",
            r#"{"type":"response.created","response":{"status":"in_progress"}}"#,
        ),
        ("response.in_progress", r#"{"type":"response.in_progress"}"#),
        (
            "response.reasoning_summary_text.delta",
            r#"{"type":"response.reasoning_summary_text.delta","output_index":0,"delta":"考え 😀"}"#,
        ),
        (
            "response.output_text.delta",
            r#"{"type":"response.output_text.delta","output_index":1,"delta":"héllo 😀"}"#,
        ),
        (
            "response.output_text.done",
            r#"{"type":"response.output_text.done","output_index":1,"text":"héllo 😀"}"#,
        ),
        (
            "response.completed",
            r#"{"type":"response.completed","response":{"status":"completed","usage":{"input_tokens":140,"output_tokens":5,"input_tokens_details":{"cached_tokens":40}}}}"#,
        ),
    ]);
    assert_events_at_every_boundary(
        make,
        input.as_bytes(),
        &[
            "started",
            r#"reasoning[0] "考え 😀""#,
            r#"text[1] "héllo 😀""#,
            "usage in=R100 out=R5 cache_read=R40 cache_create=-",
            "completed EndTurn",
        ],
    );
}

// Refutes closing a parallel tool at the next added item, losing call_id/name/index,
// repeating done snapshots, and dropping terminal usage or the tool stop reason.
#[test]
fn responses_parallel_tools_preserve_indices_and_do_not_duplicate_done_arguments() {
    let input = sse(&[
        (
            "",
            r#"{"type":"response.output_item.added","output_index":2,"item":{"type":"function_call","id":"fc_a","call_id":"call_a","name":"weather","arguments":""}}"#,
        ),
        (
            "",
            r#"{"type":"response.output_item.added","output_index":4,"item":{"type":"function_call","id":"fc_b","call_id":"call_b","name":"clock","arguments":""}}"#,
        ),
        (
            "",
            r#"{"type":"response.function_call_arguments.delta","output_index":2,"delta":"{\"city\":"}"#,
        ),
        (
            "",
            r#"{"type":"response.function_call_arguments.delta","output_index":4,"delta":"{}"}"#,
        ),
        (
            "",
            r#"{"type":"response.function_call_arguments.delta","output_index":2,"delta":"\"Paris\"}"}"#,
        ),
        (
            "",
            r#"{"type":"response.function_call_arguments.done","output_index":2,"arguments":"{\"city\":\"Paris\"}"}"#,
        ),
        (
            "",
            r#"{"type":"response.output_item.done","output_index":2,"item":{"type":"function_call","call_id":"call_a","name":"weather","arguments":"{\"city\":\"Paris\"}"}}"#,
        ),
        (
            "",
            r#"{"type":"response.output_item.done","output_index":4,"item":{"type":"function_call","call_id":"call_b","name":"clock","arguments":"{}"}}"#,
        ),
        (
            "",
            r#"{"type":"response.done","response":{"status":"completed","usage":{"input_tokens":3,"output_tokens":7}}}"#,
        ),
    ]);
    assert_events_at_every_boundary(
        make,
        input.as_bytes(),
        &[
            "started",
            r#"tool[2] id="call_a" name="weather" args="""#,
            r#"tool[4] id="call_b" name="clock" args="""#,
            r#"tool[2] id="" name="" args="{\"city\":""#,
            r#"tool[4] id="" name="" args="{}""#,
            r#"tool[2] id="" name="" args="\"Paris\"}""#,
            "tool_end[2]",
            "tool_end[4]",
            "usage in=R3 out=R7 cache_read=- cache_create=-",
            "completed ToolUse",
        ],
    );
}

// Refutes dropping full arguments when an upstream emits them only in done.
#[test]
fn responses_done_only_arguments_are_streamed_once() {
    let input = sse(&[
        (
            "",
            r#"{"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","call_id":"c","name":"f","arguments":""}}"#,
        ),
        (
            "",
            r#"{"type":"response.function_call_arguments.done","output_index":0,"arguments":"{}"}"#,
        ),
        (
            "",
            r#"{"type":"response.completed","response":{"status":"completed"}}"#,
        ),
    ]);
    assert_events_at_every_boundary(
        make,
        input.as_bytes(),
        &[
            "started",
            r#"tool[0] id="c" name="f" args="""#,
            r#"tool[0] id="" name="" args="{}""#,
            "tool_end[0]",
            "completed ToolUse",
        ],
    );
}

// Refutes treating text.done/[DONE]/EOF as completion and hiding late failures.
#[test]
fn responses_text_done_is_nonterminal_and_late_errors_remain_visible() {
    let prefix = sse(&[
        (
            "",
            r#"{"type":"response.output_text.delta","output_index":0,"delta":"x"}"#,
        ),
        (
            "",
            r#"{"type":"response.output_text.done","output_index":0,"text":"x"}"#,
        ),
    ]);
    let mut decoder = ResponsesSseDecoder::new();
    assert_eq!(
        render_all(&decoder.feed(Bytes::from(prefix.clone()))),
        ["started", r#"text[0] "x""#]
    );
    for error in [
        r#"{"type":"response.failed","response":{"error":{"type":"server_error","message":"late failure"}}}"#,
        r#"{"type":"error","error":{"type":"server_error","message":"late failure"}}"#,
        r#"{"type":"response.done","response":{"status":"failed","error":{"type":"server_error","message":"late failure"}}}"#,
    ] {
        let input = prefix.clone() + &sse(&[("", error)]);
        assert_events_at_every_boundary(
            make,
            input.as_bytes(),
            &["started", r#"text[0] "x""#, r#"failed 500 "late failure""#],
        );
    }
    for suffix in ["", "data: [DONE]\n\n", "data: {\"type\":"] {
        let input = prefix.clone() + suffix;
        let events = decode(&mut ResponsesSseDecoder::new(), [input.as_bytes()]);
        assert_eq!(
            events,
            [
                "started",
                r#"text[0] "x""#,
                r#"failed 502 "upstream stream ended before completion""#
            ]
        );
    }
}

// Refutes reporting incomplete max-token output as a successful EndTurn or ignoring usage.
#[test]
fn responses_incomplete_max_tokens_and_unknown_reason_are_distinguished() {
    let input = sse(&[(
        "",
        r#"{"type":"response.incomplete","response":{"status":"incomplete","incomplete_details":{"reason":"max_output_tokens"},"usage":{"output_tokens":9}}}"#,
    )]);
    assert_events_at_every_boundary(
        make,
        input.as_bytes(),
        &[
            "started",
            "usage in=- out=R9 cache_read=- cache_create=-",
            "completed MaxTokens",
        ],
    );
    let input = sse(&[(
        "",
        r#"{"type":"response.incomplete","response":{"incomplete_details":{"reason":"content_filter"}}}"#,
    )]);
    assert!(decode(&mut ResponsesSseDecoder::new(), [input.as_bytes()])
        .last()
        .unwrap()
        .starts_with("failed 502"));
}

// Refutes unbounded event/argument retention, missing shape validation, and accepting
// argument deltas before the identity event needed by client encoders.
#[test]
fn responses_malformed_and_resource_limits_fail_without_completion() {
    for payload in [
        "not json",
        r#"{"type":"response.output_text.delta","delta":"x"}"#,
        r#"{"type":"response.function_call_arguments.delta","output_index":0,"delta":"{}"}"#,
    ] {
        let input = sse(&[("", payload)]);
        let events = decode(&mut ResponsesSseDecoder::new(), [input.as_bytes()]);
        assert!(events.last().unwrap().starts_with("failed 502"));
        assert!(!events.iter().any(|e| e.starts_with("completed")));
    }
    let input = sse(&[(
        "",
        r#"{"type":"response.output_text.delta","output_index":0,"delta":"oversized"}"#,
    )]);
    assert_eq!(
        decode(
            &mut ResponsesSseDecoder::with_limits(20, 8),
            input.as_bytes().chunks(1)
        ),
        [r#"failed 502 "upstream sent an SSE event larger than the gateway limit""#]
    );
    let input = sse(&[
        (
            "",
            r#"{"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","call_id":"c","name":"f","arguments":""}}"#,
        ),
        (
            "",
            r#"{"type":"response.function_call_arguments.delta","output_index":0,"delta":"123"}"#,
        ),
        (
            "",
            r#"{"type":"response.function_call_arguments.delta","output_index":0,"delta":"456"}"#,
        ),
    ]);
    assert_eq!(
        decode(
            &mut ResponsesSseDecoder::with_limits(1024, 5),
            [input.as_bytes()]
        ),
        [
            "started",
            r#"tool[0] id="c" name="f" args="""#,
            r#"tool[0] id="" name="" args="123""#,
            r#"failed 502 "upstream tool call arguments exceeded the gateway limit""#
        ]
    );
}

// Refutes unbounded metadata growth even when every tool is tiny and already ended.
#[test]
fn responses_tool_metadata_is_bounded_across_finished_calls() {
    let mut decoder = ResponsesSseDecoder::new();
    for index in 0..4096 {
        let payload = format!(
            r#"{{"type":"response.output_item.done","output_index":{index},"item":{{"type":"function_call","call_id":"c{index}","name":"f","arguments":""}}}}"#
        );
        let events = render_all(&decoder.feed(Bytes::from(sse(&[("", &payload)]))));
        assert_eq!(events.last().unwrap(), &format!("tool_end[{index}]"));
    }
    let payload = r#"{"type":"response.output_item.added","output_index":4096,"item":{"type":"function_call","call_id":"overflow","name":"f","arguments":""}}"#;
    assert_eq!(
        render_all(&decoder.feed(Bytes::from(sse(&[("", payload)])))),
        [r#"failed 502 "upstream tool call count exceeded the gateway limit""#]
    );
    assert!(decoder.finish().is_empty());
}

// Refutes assuming all errors are nested or only error.type classifies them.
#[test]
fn responses_flat_and_code_errors_keep_status_and_capped_messages() {
    for payload in [
        r#"{"type":"error","code":"rate_limit_exceeded","message":"slow down"}"#,
        r#"{"type":"response.failed","response":{"error":{"code":"rate_limit_exceeded","message":"slow down"}}}"#,
    ] {
        let input = sse(&[("", payload)]);
        assert_events_at_every_boundary(make, input.as_bytes(), &[r#"failed 429 "slow down""#]);
    }
    let payload = format!(r#"{{"type":"error","message":"{}"}}"#, "😀".repeat(301));
    let input = sse(&[("", &payload)]);
    assert_eq!(
        decode(&mut ResponsesSseDecoder::new(), [input.as_bytes()]),
        [format!("failed 502 {:?}", "😀".repeat(300))]
    );
}

// Refutes emitting a full done snapshot twice, discarding a not-yet-streamed suffix,
// or accepting deltas for an already-ended tool.
#[test]
fn responses_done_snapshot_suffix_and_invalid_tool_transitions() {
    let prefix = sse(&[
        (
            "",
            r#"{"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","call_id":"c","name":"f","arguments":""}}"#,
        ),
        (
            "",
            r#"{"type":"response.function_call_arguments.delta","output_index":0,"delta":"{"}"#,
        ),
        (
            "",
            r#"{"type":"response.function_call_arguments.done","output_index":0,"arguments":"{}"}"#,
        ),
    ]);
    let input = prefix.clone()
        + &sse(&[(
            "",
            r#"{"type":"response.completed","response":{"status":"completed"}}"#,
        )]);
    assert_events_at_every_boundary(
        make,
        input.as_bytes(),
        &[
            "started",
            r#"tool[0] id="c" name="f" args="""#,
            r#"tool[0] id="" name="" args="{""#,
            r#"tool[0] id="" name="" args="}""#,
            "tool_end[0]",
            "completed ToolUse",
        ],
    );
    let input = prefix
        + &sse(&[(
            "",
            r#"{"type":"response.function_call_arguments.delta","output_index":0,"delta":"x"}"#,
        )]);
    assert!(decode(&mut ResponsesSseDecoder::new(), [input.as_bytes()])
        .last()
        .unwrap()
        .starts_with("failed 502"));
}

// Refutes duplicate terminals or observing data after a real terminal, and rejecting
// a complete terminal JSON event solely because its trailing blank line was omitted.
#[test]
fn responses_terminal_is_final_and_complete_json_at_eof_is_accepted() {
    let input = b"data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\"}}";
    assert_events_at_every_boundary(make, input, &["started", "completed EndTurn"]);
    let mut decoder = ResponsesSseDecoder::new();
    let input = sse(&[(
        "",
        r#"{"type":"response.completed","response":{"status":"completed"}}"#,
    )]);
    assert_eq!(
        render_all(&decoder.feed(Bytes::from(input))),
        ["started", "completed EndTurn"]
    );
    assert!(decoder
        .feed(Bytes::from_static(b"data: garbage\n\n"))
        .is_empty());
    assert!(decoder.finish().is_empty());
}
