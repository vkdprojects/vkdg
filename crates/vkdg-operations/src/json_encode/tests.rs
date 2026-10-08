//! Non-streaming bodies, with every expected value written by hand from the Anthropic
//! Messages / `OpenAI` Chat Completions response documentation.

use serde_json::{json, Value};
use vkdg_core::{ApiType, RequestId, VkdgError};

use super::{json_encoder_for, MAX_JSON_RESPONSE_BYTES};
use crate::UsageCount::{Reported, Unknown};
use crate::{ConversationEvent as E, StopReason, StreamContext, UsageCount};

// ── Fixtures ──────────────────────────────────────────────────────────────────

fn text(s: &str, index: u32) -> E {
    E::OutputDelta {
        delta: s.to_owned(),
        index,
    }
}

fn reasoning(s: &str, index: u32) -> E {
    E::ReasoningDelta {
        delta: s.to_owned(),
        index,
    }
}

fn tool_start(id: &str, name: &str, args: &str, index: u32) -> E {
    E::ToolCallDelta {
        tool_use_id: id.to_owned(),
        name: name.to_owned(),
        input_delta: args.to_owned(),
        index,
    }
}

fn tool_more(args: &str, index: u32) -> E {
    tool_start("", "", args, index)
}

fn usage(input: UsageCount, output: UsageCount, read: UsageCount, creation: UsageCount) -> E {
    E::Usage {
        input_tokens: input,
        output_tokens: output,
        cache_read_tokens: read,
        cache_creation_tokens: creation,
    }
}

fn completed(reason: StopReason) -> E {
    E::Completed {
        stop_reason: reason,
    }
}

fn encode(api: &ApiType, rid: &RequestId, events: &[E]) -> Result<Vec<u8>, VkdgError> {
    let mut enc = json_encoder_for(
        api,
        &StreamContext {
            model: "m",
            request_id: rid,
        },
    );
    for event in events {
        enc.push(event);
    }
    enc.finish()
}

fn body(api: &ApiType, rid: &RequestId, events: &[E]) -> Value {
    let bytes = encode(api, rid, events).expect("encoder returned an error");
    serde_json::from_slice(&bytes).expect("body is JSON")
}

const ANTHROPIC: ApiType = ApiType::AnthropicMessages;
const OPENAI: ApiType = ApiType::OpenAiChatCompletions;

fn assert_upstream_502(err: &VkdgError, message: &str) {
    match err {
        VkdgError::UpstreamError {
            code: 502,
            message: m,
            ..
        } => assert_eq!(m, message),
        other => panic!("expected UpstreamError 502, got {other:?}"),
    }
}

// ── Anthropic ─────────────────────────────────────────────────────────────────

// Refutes: blocks emitted grouped by kind instead of event order, deltas of one block split
// into several, a missing thinking signature field, or tool input left as a string.
#[test]
fn anthropic_body_has_blocks_in_event_order_with_coalesced_deltas() {
    let rid = RequestId::new();
    let got = body(
        &ANTHROPIC,
        &rid,
        &[
            E::Started {
                request_id: RequestId::new(),
            },
            reasoning("a", 0),
            reasoning("b", 0),
            text("Hel", 1),
            text("lo", 1),
            tool_start("toolu_1", "ls", "{\"p\":", 2),
            tool_more("\"/\"}", 2),
            E::ToolCallEnd { index: 2 },
            usage(Reported(10), Reported(5), Reported(3), Unknown),
            completed(StopReason::ToolUse),
        ],
    );
    assert_eq!(
        got,
        json!({
            "id": format!("msg_{}", rid.0),
            "type": "message",
            "role": "assistant",
            "model": "m",
            "content": [
                {"type":"thinking","thinking":"ab","signature":""},
                {"type":"text","text":"Hello"},
                {"type":"tool_use","id":"toolu_1","name":"ls","input":{"p":"/"}},
            ],
            "stop_reason": "tool_use",
            "stop_sequence": null,
            "usage": {"input_tokens":10,"output_tokens":5,"cache_read_input_tokens":3},
        })
    );
}

// Refutes: merging non-adjacent blocks (text, reasoning, text) or different source indices.
#[test]
fn anthropic_only_adjacent_deltas_of_the_same_block_coalesce() {
    let rid = RequestId::new();
    let got = body(
        &ANTHROPIC,
        &rid,
        &[
            text("a", 0),
            reasoning("r", 0),
            text("b", 0),
            text("c", 1),
            completed(StopReason::EndTurn),
        ],
    );
    assert_eq!(
        got["content"],
        json!([
            {"type":"text","text":"a"},
            {"type":"thinking","thinking":"r","signature":""},
            {"type":"text","text":"b"},
            {"type":"text","text":"c"},
        ])
    );
}

// Refutes: empty tool arguments rendered as "" (invalid for Anthropic's object `input`).
#[test]
fn anthropic_empty_tool_arguments_become_an_empty_object() {
    let rid = RequestId::new();
    let got = body(
        &ANTHROPIC,
        &rid,
        &[
            tool_start("toolu_1", "now", "", 0),
            completed(StopReason::ToolUse),
        ],
    );
    assert_eq!(got["content"][0]["input"], json!({}));
}

// Refutes: returning half-written tool JSON as a success, or echoing the arguments
// (user payload) into the error.
#[test]
fn anthropic_unparseable_tool_arguments_fail_without_echoing_them() {
    let rid = RequestId::new();
    let err = encode(
        &ANTHROPIC,
        &rid,
        &[
            tool_start("toolu_1", "pay", "{\"card\":\"4111-SECRET", 0),
            completed(StopReason::ToolUse),
        ],
    )
    .unwrap_err();
    assert_upstream_502(
        &err,
        "upstream returned tool call arguments that are not valid JSON",
    );
    assert!(!err.to_string().contains("SECRET"));
}

// Refutes: accepting a valid-JSON non-object as Anthropic `input`.
#[test]
fn anthropic_non_object_tool_arguments_are_rejected() {
    let rid = RequestId::new();
    let err = encode(
        &ANTHROPIC,
        &rid,
        &[
            tool_start("toolu_1", "x", "[1]", 0),
            completed(StopReason::ToolUse),
        ],
    )
    .unwrap_err();
    assert_upstream_502(
        &err,
        "upstream returned tool call arguments that are not valid JSON",
    );
}

// Refutes: forwarding a provider id the Anthropic API would reject next turn.
#[test]
fn anthropic_tool_ids_are_sanitized_or_synthesized() {
    let rid = RequestId::new();
    let got = body(
        &ANTHROPIC,
        &rid,
        &[
            tool_start("call.1:x y", "a", "{}", 0),
            tool_start("", "b", "{}", 1),
            tool_start("", "c", "{}", 2),
            completed(StopReason::ToolUse),
        ],
    );
    let ids: Vec<&str> = (0..3)
        .map(|i| got["content"][i]["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids[0], "call_1_x_y");
    assert!(
        ids[1].starts_with("toolu_") && ids[2].starts_with("toolu_"),
        "{ids:?}"
    );
    assert_ne!(ids[1], ids[2]);
    assert!(ids[1..].iter().all(|id| id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')));
}

// Refutes: usage fields absent for a provider that reported none (Anthropic requires `usage`),
// or cache fields printed as 0.
#[test]
fn anthropic_usage_defaults_to_zero_and_omits_empty_cache_fields() {
    let rid = RequestId::new();
    let got = body(
        &ANTHROPIC,
        &rid,
        &[text("x", 0), completed(StopReason::EndTurn)],
    );
    assert_eq!(got["usage"], json!({"input_tokens":0,"output_tokens":0}));
}

// Refutes: wrong stop_reason strings.
#[test]
fn anthropic_stop_reasons() {
    let cases = [
        (StopReason::EndTurn, "end_turn"),
        (StopReason::MaxTokens, "max_tokens"),
        (StopReason::ToolUse, "tool_use"),
        (StopReason::StopSequence, "stop_sequence"),
        (StopReason::Cancelled, "end_turn"),
    ];
    for (reason, wire) in cases {
        let rid = RequestId::new();
        let got = body(&ANTHROPIC, &rid, &[text("x", 0), completed(reason)]);
        assert_eq!(got["stop_reason"], wire);
    }
}

// ── OpenAI ────────────────────────────────────────────────────────────────────

// Refutes: wrong envelope fields, reasoning in `content`, `content: ""` next to tool calls,
// empty arguments left empty, or cache creation inflating prompt_tokens.
#[test]
fn openai_body_with_reasoning_tool_calls_and_usage() {
    let rid = RequestId::new();
    let mut got = body(
        &OPENAI,
        &rid,
        &[
            E::Started {
                request_id: RequestId::new(),
            },
            reasoning("think", 0),
            tool_start("call_1", "ls", "{\"p\":", 0),
            tool_more("1}", 0),
            E::ToolCallEnd { index: 0 },
            tool_start("", "now", "", 1),
            E::ToolCallEnd { index: 1 },
            usage(Reported(2), Reported(9), Reported(40), Reported(2000)),
            completed(StopReason::ToolUse),
        ],
    );
    let created = got["created"].as_u64().expect("created is unix seconds");
    assert!(created > 1_700_000_000, "created {created}");
    got.as_object_mut().unwrap().remove("created");
    let synthesized = got["choices"][0]["message"]["tool_calls"][1]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(synthesized.starts_with("call_"), "{synthesized}");
    assert_eq!(
        got,
        json!({
            "id": format!("chatcmpl-{}", rid.0),
            "object": "chat.completion",
            "model": "m",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": null,
                    "reasoning_content": "think",
                    "tool_calls": [
                        {"id":"call_1","type":"function",
                         "function":{"name":"ls","arguments":"{\"p\":1}"}},
                        {"id":synthesized,"type":"function",
                         "function":{"name":"now","arguments":"{}"}},
                    ],
                },
                "finish_reason": "tool_calls",
            }],
            "usage": {
                "prompt_tokens": 42,
                "completion_tokens": 9,
                "total_tokens": 51,
                "prompt_tokens_details": {"cached_tokens": 40},
                "cache_creation_input_tokens": 2000,
            },
        })
    );
}

// Refutes: `content: null` or a stray reasoning/tool_calls key on a plain text answer.
#[test]
fn openai_plain_text_body_has_only_content() {
    let rid = RequestId::new();
    let got = body(
        &OPENAI,
        &rid,
        &[
            text("Hel", 0),
            text("lo", 0),
            usage(Reported(7), Reported(2), Unknown, Unknown),
            completed(StopReason::EndTurn),
        ],
    );
    assert_eq!(
        got["choices"][0],
        json!({"index":0,"message":{"role":"assistant","content":"Hello"},"finish_reason":"stop"})
    );
    assert_eq!(
        got["usage"],
        json!({"prompt_tokens":7,"completion_tokens":2,"total_tokens":9})
    );
}

// Refutes: inventing a zero usage object when the provider reported none.
#[test]
fn openai_without_reported_usage_omits_the_usage_object() {
    let rid = RequestId::new();
    let got = body(
        &OPENAI,
        &rid,
        &[text("x", 0), completed(StopReason::EndTurn)],
    );
    assert!(got.get("usage").is_none(), "{got}");
}

// Refutes: wrong finish_reason strings.
#[test]
fn openai_finish_reasons() {
    let cases = [
        (StopReason::EndTurn, "stop"),
        (StopReason::MaxTokens, "length"),
        (StopReason::ToolUse, "tool_calls"),
        (StopReason::StopSequence, "stop"),
        (StopReason::Cancelled, "stop"),
    ];
    for (reason, wire) in cases {
        let rid = RequestId::new();
        let got = body(&OPENAI, &rid, &[text("x", 0), completed(reason)]);
        assert_eq!(got["choices"][0]["finish_reason"], wire);
    }
}

// ── Both dialects: termination and limits ─────────────────────────────────────

// Refutes: a Failed event swallowed into a 200 body, or its error flattened to a generic one.
#[test]
fn failed_event_makes_finish_return_that_error() {
    for api in [ANTHROPIC, OPENAI] {
        let rid = RequestId::new();
        let err = encode(
            &api,
            &rid,
            &[
                text("partial", 0),
                E::Failed {
                    error: VkdgError::UpstreamError {
                        code: 429,
                        message: "slow".into(),
                        retry_after: Some(7),
                    },
                },
                completed(StopReason::EndTurn),
            ],
        )
        .unwrap_err();
        match err {
            VkdgError::UpstreamError {
                code: 429,
                retry_after: Some(7),
                ..
            } => {}
            other => panic!("wrong error {other:?}"),
        }
    }
}

// Refutes: a truncated stream (no Completed) after a tool call becoming `stop`, or an
// empty/never-started stream becoming a successful empty answer.
#[test]
fn missing_completed_infers_the_reason_only_when_something_arrived() {
    for (api, tool_reason, plain_reason) in [
        (ANTHROPIC, "tool_use", "end_turn"),
        (OPENAI, "tool_calls", "stop"),
    ] {
        let key = |v: &Value| {
            if v.get("stop_reason").is_some() {
                v["stop_reason"].clone()
            } else {
                v["choices"][0]["finish_reason"].clone()
            }
        };
        let rid = RequestId::new();
        let with_tool = body(&api, &rid, &[tool_start("c1", "ls", "{}", 0)]);
        assert_eq!(key(&with_tool), tool_reason);
        let with_text = body(&api, &rid, &[text("x", 0)]);
        assert_eq!(key(&with_text), plain_reason);
        let with_usage = body(
            &api,
            &rid,
            &[usage(Reported(1), Reported(1), Unknown, Unknown)],
        );
        assert_eq!(key(&with_usage), plain_reason);

        assert!(encode(&api, &rid, &[]).is_err());
        assert!(encode(
            &api,
            &rid,
            &[E::Started {
                request_id: RequestId::new()
            }]
        )
        .is_err());
    }
}

// Refutes: tool argument fragments for an unknown call being attached to the wrong call or
// dropped while the body still claims success.
#[test]
fn argument_fragment_for_unknown_call_is_an_error() {
    for api in [ANTHROPIC, OPENAI] {
        let rid = RequestId::new();
        let err = encode(
            &api,
            &rid,
            &[
                tool_start("c1", "ls", "{}", 0),
                tool_more("{}", 5),
                completed(StopReason::ToolUse),
            ],
        )
        .unwrap_err();
        assert_upstream_502(&err, "upstream sent tool call fragments out of order");
    }
}

// Refutes: unbounded accumulation (memory exhaustion by a hostile or runaway upstream), and an
// off-by-one at the limit.
#[test]
fn accumulated_content_is_bounded() {
    for api in [ANTHROPIC, OPENAI] {
        let rid = RequestId::new();
        let at_limit = "a".repeat(MAX_JSON_RESPONSE_BYTES);
        assert!(encode(
            &api,
            &rid,
            &[text(&at_limit, 0), completed(StopReason::EndTurn)]
        )
        .is_ok());

        let over = [
            text(&at_limit, 0),
            reasoning("x", 0),
            completed(StopReason::EndTurn),
        ];
        let err = encode(&api, &rid, &over).unwrap_err();
        assert_upstream_502(
            &err,
            "upstream response exceeds the non-streaming size limit",
        );

        // Tool arguments count toward the same budget.
        let args = [
            tool_start("c1", "ls", &at_limit, 0),
            tool_more("x", 0),
            completed(StopReason::ToolUse),
        ];
        let err = encode(&api, &rid, &args).unwrap_err();
        assert_upstream_502(
            &err,
            "upstream response exceeds the non-streaming size limit",
        );
    }
}
