//! Wire-level tests for the client-dialect stream encoders. Every expected value
//! is written by hand from the Anthropic / `OpenAI` streaming documentation.

use serde_json::{json, Value};
use vkdg_core::{RequestId, VkdgError};

use super::{AnthropicStreamEncoder, OpenAiStreamEncoder};
use crate::UsageCount::{Estimated, Reported, Unknown};
use crate::{ConversationEvent as E, StopReason, StreamContext, StreamEncoder, UsageCount};

// ── Fixtures ──────────────────────────────────────────────────────────────────

struct Frame {
    event: Option<String>,
    data: String,
}

impl Frame {
    fn json(&self) -> Value {
        serde_json::from_str(&self.data)
            .unwrap_or_else(|e| panic!("frame data is not JSON ({e}): {}", self.data))
    }
}

/// Splits encoder output into SSE frames; fails when output stops mid-event
/// (the heartbeat wrapper relies on every chunk ending on `\n\n`).
fn frames(bytes: &[u8]) -> Vec<Frame> {
    let text = std::str::from_utf8(bytes).expect("encoder output is UTF-8");
    assert!(
        text.is_empty() || text.ends_with("\n\n"),
        "chunk does not end on an event boundary: {text:?}"
    );
    text.split_terminator("\n\n")
        .map(|raw| {
            let mut event = None;
            let mut data = None;
            for line in raw.lines() {
                if let Some(v) = line.strip_prefix("event: ") {
                    event = Some(v.to_owned());
                } else if let Some(v) = line.strip_prefix("data: ") {
                    assert!(data.is_none(), "two data lines in one frame: {raw:?}");
                    data = Some(v.to_owned());
                } else {
                    panic!("unexpected line {line:?} in frame {raw:?}");
                }
            }
            Frame {
                event,
                data: data.unwrap_or_else(|| panic!("frame without data: {raw:?}")),
            }
        })
        .collect()
}

fn anthropic(rid: &RequestId) -> AnthropicStreamEncoder {
    AnthropicStreamEncoder::new(&StreamContext {
        model: "m",
        request_id: rid,
    })
}

fn openai(rid: &RequestId) -> OpenAiStreamEncoder {
    OpenAiStreamEncoder::new(&StreamContext {
        model: "m",
        request_id: rid,
    })
}

fn started() -> E {
    E::Started {
        request_id: RequestId::new(),
    }
}

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

fn tool_end(index: u32) -> E {
    E::ToolCallEnd { index }
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

fn failed(error: VkdgError) -> E {
    E::Failed { error }
}

fn run(enc: &mut dyn StreamEncoder, events: &[E]) -> Vec<u8> {
    let mut out = Vec::new();
    for event in events {
        out.extend(enc.encode(event));
    }
    out
}

fn find_event(frames: &[Frame], name: &str) -> Value {
    frames
        .iter()
        .find(|f| f.event.as_deref() == Some(name))
        .unwrap_or_else(|| panic!("no {name} event"))
        .json()
}

fn openai_usage_chunk(frames: &[Frame]) -> Value {
    frames
        .iter()
        .filter(|f| f.data != "[DONE]")
        .map(Frame::json)
        .find(|v| v.get("usage").is_some())
        .expect("no usage chunk")
}

fn openai_finish_reasons(frames: &[Frame]) -> Vec<Value> {
    frames
        .iter()
        .filter(|f| f.data != "[DONE]")
        .map(Frame::json)
        .filter_map(|v| v["choices"].get(0).map(|c| c["finish_reason"].clone()))
        .filter(|r| !r.is_null())
        .collect()
}

fn usage_event() -> E {
    usage(Estimated(500), Reported(3), Reported(40), Unknown)
}

// ── Anthropic ─────────────────────────────────────────────────────────────────

// Refutes: a writer that reorders keys, drops the `event:` line, leaves a chunk off the
// `\n\n` boundary, or mis-escapes quotes/newlines in text.
#[test]
fn anthropic_plain_text_turn_is_byte_exact() {
    let rid = RequestId::new();
    let mut enc = anthropic(&rid);
    let mut out = run(
        &mut enc,
        &[
            started(),
            text("say \"hi\"\n é", 0),
            completed(StopReason::EndTurn),
        ],
    );
    out.extend(enc.finish());
    let expected = format!(
        concat!(
            "event: message_start\n",
            "data: {{\"type\":\"message_start\",\"message\":{{\"id\":\"msg_{rid}\",\"type\":\"message\",",
            "\"role\":\"assistant\",\"model\":\"m\",\"content\":[],\"stop_reason\":null,",
            "\"stop_sequence\":null,\"usage\":{{\"input_tokens\":0,\"output_tokens\":0}}}}}}\n\n",
            "event: content_block_start\n",
            "data: {{\"type\":\"content_block_start\",\"index\":0,",
            "\"content_block\":{{\"type\":\"text\",\"text\":\"\"}}}}\n\n",
            "event: content_block_delta\n",
            "data: {{\"type\":\"content_block_delta\",\"index\":0,",
            "\"delta\":{{\"type\":\"text_delta\",\"text\":\"say \\\"hi\\\"\\n é\"}}}}\n\n",
            "event: content_block_stop\n",
            "data: {{\"type\":\"content_block_stop\",\"index\":0}}\n\n",
            "event: message_delta\n",
            "data: {{\"type\":\"message_delta\",\"delta\":{{\"stop_reason\":\"end_turn\",",
            "\"stop_sequence\":null}},\"usage\":{{\"input_tokens\":0,\"output_tokens\":0}}}}\n\n",
            "event: message_stop\n",
            "data: {{\"type\":\"message_stop\"}}\n\n",
        ),
        rid = rid.0
    );
    assert_eq!(String::from_utf8(out).unwrap(), expected);
}

// Refutes: a stream that opens content without message_start when the provider sent no Started.
#[test]
fn anthropic_started_less_stream_still_opens_with_message_start() {
    let rid = RequestId::new();
    let mut enc = anthropic(&rid);
    let out = frames(&run(&mut enc, &[text("x", 0)]));
    let names: Vec<_> = out.iter().map(|f| f.event.as_deref().unwrap()).collect();
    assert_eq!(
        names,
        [
            "message_start",
            "content_block_start",
            "content_block_delta"
        ]
    );
}

// Refutes: inventing a thinking signature, or a thinking block that is not opened as the
// Anthropic spec says (`thinking: ""`, no signature) before its thinking_delta.
#[test]
fn anthropic_thinking_block_has_no_fabricated_signature() {
    let rid = RequestId::new();
    let mut enc = anthropic(&rid);
    let out = frames(&run(
        &mut enc,
        &[started(), reasoning("hmm", 0), text("ok", 1)],
    ));
    assert_eq!(
        out[1].json(),
        json!({"type":"content_block_start","index":0,
               "content_block":{"type":"thinking","thinking":""}})
    );
    assert_eq!(
        out[2].json(),
        json!({"type":"content_block_delta","index":0,
               "delta":{"type":"thinking_delta","thinking":"hmm"}})
    );
    // Switching kind closes the thinking block and renumbers the text block to wire index 1.
    assert_eq!(out[3].event.as_deref(), Some("content_block_stop"));
    assert_eq!(out[4].json()["index"], 1);
}

// Found live: Kiro key usage recorded input 0. message_start goes out before any usage is
// known, and message_delta carried output only, so the decoder's input count never reached
// the client or the meter.
#[test]
fn anthropic_message_delta_carries_the_full_final_usage() {
    let rid = RequestId::new();
    let mut enc = anthropic(&rid);
    let mut out = enc.encode(&usage_event());
    out.extend(enc.finish());
    let delta = find_event(&frames(&out), "message_delta");
    assert_eq!(delta["usage"]["input_tokens"], 500, "{delta}");
    assert_eq!(delta["usage"]["output_tokens"], 3, "{delta}");
    assert_eq!(delta["usage"]["cache_read_input_tokens"], 40, "{delta}");
}

// Refutes: a later Usage event whose fields are Unknown overwriting the known input count
// with 0 (message_start usage followed by a message_delta that only reports output).
#[test]
fn anthropic_unknown_usage_fields_keep_the_previous_value() {
    let rid = RequestId::new();
    let mut enc = anthropic(&rid);
    let out = run(
        &mut enc,
        &[
            started(),
            usage(Reported(120), Reported(1), Reported(30), Reported(9)),
            usage(Unknown, Reported(42), Unknown, Unknown),
            completed(StopReason::EndTurn),
        ],
    );
    let delta = find_event(&frames(&out), "message_delta");
    assert_eq!(
        delta["usage"],
        json!({"input_tokens":120,"output_tokens":42,
               "cache_read_input_tokens":30,"cache_creation_input_tokens":9})
    );
}

// Refutes: a stream that ends after a tool call without Completed reporting `end_turn`, which
// makes agent clients stop instead of running the tool.
#[test]
fn anthropic_finish_after_tool_call_reports_tool_use() {
    let rid = RequestId::new();
    let mut enc = anthropic(&rid);
    let mut out = run(&mut enc, &[started(), tool_start("toolu_1", "ls", "{}", 0)]);
    out.extend(enc.finish());
    let delta = find_event(&frames(&out), "message_delta");
    assert_eq!(delta["delta"]["stop_reason"], "tool_use");
}

// Refutes: finish() reporting tool_use for a plain text turn.
#[test]
fn anthropic_finish_without_tool_call_reports_end_turn() {
    let rid = RequestId::new();
    let mut enc = anthropic(&rid);
    let mut out = run(&mut enc, &[started(), text("hi", 0)]);
    out.extend(enc.finish());
    let delta = find_event(&frames(&out), "message_delta");
    assert_eq!(delta["delta"]["stop_reason"], "end_turn");
}

// Refutes: every Failed becoming `api_error`, losing the 429 that tells the client to back off.
#[test]
fn anthropic_failed_renders_through_the_shared_error_mapper() {
    let cases = [
        (
            VkdgError::UpstreamError {
                code: 429,
                message: "slow down".into(),
                retry_after: None,
            },
            "rate_limit_error",
            "upstream error 429: slow down",
        ),
        (
            VkdgError::UpstreamError {
                code: 502,
                message: "upstream sent a malformed anthropic stream event".into(),
                retry_after: None,
            },
            "api_error",
            "upstream error 502: upstream sent a malformed anthropic stream event",
        ),
        (
            VkdgError::Unauthenticated,
            "authentication_error",
            "unauthenticated",
        ),
    ];
    for (error, kind, message) in cases {
        let rid = RequestId::new();
        let mut enc = anthropic(&rid);
        let out = frames(&run(&mut enc, &[started(), failed(error)]));
        let last = out.last().unwrap();
        assert_eq!(last.event.as_deref(), Some("error"));
        assert_eq!(
            last.json(),
            json!({"type":"error","error":{"type":kind,"message":message}})
        );
        // Failed is terminal.
        assert_eq!(enc.encode(&text("late", 0)), Vec::<u8>::new());
        assert_eq!(enc.finish(), Vec::<u8>::new());
    }
}

// Refutes: forwarding an empty or non-conforming provider tool id, which the Anthropic API
// rejects on the next turn (`^[a-zA-Z0-9_-]+$`), or reusing one id for two calls.
#[test]
fn anthropic_tool_ids_are_sanitized_and_synthesized_when_empty() {
    let rid = RequestId::new();
    let mut enc = anthropic(&rid);
    let out = frames(&run(
        &mut enc,
        &[
            started(),
            tool_start("", "a", "", 0),
            tool_end(0),
            tool_start("", "b", "", 1),
            tool_end(1),
            tool_start("call.1:x y", "c", "", 2),
        ],
    ));
    let ids: Vec<String> = out
        .iter()
        .map(Frame::json)
        .filter(|v| v["type"] == "content_block_start")
        .map(|v| v["content_block"]["id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(ids.len(), 3);
    assert!(
        ids[0].starts_with("toolu_") && ids[1].starts_with("toolu_"),
        "{ids:?}"
    );
    assert_ne!(ids[0], ids[1], "synthesized ids must be unique per call");
    assert!(ids[..2].iter().all(|id| id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')));
    assert_eq!(ids[2], "call_1_x_y");
}

// Refutes: silently dropping or mis-attributing an argument fragment that arrives after its
// block was closed (or for a call never opened) -- the client would run corrupted tool JSON.
#[test]
fn anthropic_out_of_order_tool_fragment_ends_the_stream_with_an_error_event() {
    let shapes: [Vec<E>; 2] = [
        // Fragment for a call whose block already closed.
        vec![
            started(),
            tool_start("toolu_1", "ls", "{\"a\":", 0),
            tool_end(0),
            tool_more("1}", 0),
        ],
        // Fragment for a call that never opened.
        vec![started(), text("hi", 0), tool_more("{}", 3)],
    ];
    for events in shapes {
        let rid = RequestId::new();
        let mut enc = anthropic(&rid);
        let out = frames(&run(&mut enc, &events));
        let last = out.last().unwrap();
        assert_eq!(last.event.as_deref(), Some("error"));
        assert_eq!(
            last.json(),
            json!({"type":"error","error":{"type":"api_error",
                   "message":"tool call fragments arrived out of order"}})
        );
        assert!(
            out.iter().all(|f| !f.data.contains("\"1}\"")),
            "the stray fragment must not reach the client as arguments"
        );
        assert_eq!(
            enc.encode(&completed(StopReason::ToolUse)),
            Vec::<u8>::new()
        );
        assert_eq!(enc.finish(), Vec::<u8>::new());
    }
}

// Refutes: closing the call block on the first fragment so later argument fragments of the
// same call trip the out-of-order error (the normal streaming path must stay normal).
#[test]
fn anthropic_tool_arguments_stream_as_input_json_deltas_in_one_block() {
    let rid = RequestId::new();
    let mut enc = anthropic(&rid);
    let out = frames(&run(
        &mut enc,
        &[
            started(),
            tool_start("toolu_1", "ls", "", 4),
            tool_more("{\"a\":", 4),
            tool_more("1}", 4),
            tool_end(4),
            completed(StopReason::ToolUse),
        ],
    ));
    let json: Vec<Value> = out.iter().map(Frame::json).collect();
    assert_eq!(
        json[1],
        json!({"type":"content_block_start","index":0,
               "content_block":{"type":"tool_use","id":"toolu_1","name":"ls","input":{}}})
    );
    assert_eq!(
        json[2]["delta"],
        json!({"type":"input_json_delta","partial_json":"{\"a\":"})
    );
    assert_eq!(
        json[3]["delta"],
        json!({"type":"input_json_delta","partial_json":"1}"})
    );
    assert_eq!(json[4], json!({"type":"content_block_stop","index":0}));
    assert_eq!(json[5]["delta"]["stop_reason"], "tool_use");
}

// ── OpenAI ────────────────────────────────────────────────────────────────────

// Refutes: a writer that mis-frames chunks, mis-escapes text, or omits the role chunk / [DONE].
#[test]
fn openai_plain_text_turn_is_byte_exact_apart_from_created() {
    let rid = RequestId::new();
    let mut enc = openai(&rid);
    let mut out = run(
        &mut enc,
        &[
            started(),
            text("say \"hi\"\n é", 0),
            completed(StopReason::EndTurn),
        ],
    );
    out.extend(enc.finish());
    let text = String::from_utf8(out).unwrap();
    let created =
        serde_json::from_str::<Value>(text.lines().next().unwrap().strip_prefix("data: ").unwrap())
            .unwrap()["created"]
            .as_u64()
            .unwrap();
    let head = format!(
        "{{\"id\":\"chatcmpl-{}\",\"object\":\"chat.completion.chunk\",\"created\":{created},\
         \"model\":\"m\",\"choices\":[{{\"index\":0,\"delta\":",
        rid.0
    );
    let expected = format!(
        "data: {head}{{\"role\":\"assistant\",\"content\":\"\"}},\"finish_reason\":null}}]}}\n\n\
         data: {head}{{\"content\":\"say \\\"hi\\\"\\n é\"}},\"finish_reason\":null}}]}}\n\n\
         data: {head}{{}},\"finish_reason\":\"stop\"}}]}}\n\n\
         data: [DONE]\n\n"
    );
    assert_eq!(text, expected);
}

// Refutes: prompt_tokens that exclude cache reads (undercounting what OpenAI clients bill),
// or that include cache *creation* (OmniRoute #2215: a 2-token prompt billed as ~2000).
#[test]
fn openai_usage_chunk_follows_openai_prompt_token_semantics() {
    let cases = [
        // (input, output, cache_read, cache_creation) -> (prompt, completion, total, cached, created)
        ((100, 5, 40, 7), (140, 5, 145, Some(40), Some(7))),
        ((2, 9, 0, 2000), (2, 9, 11, None, Some(2000))),
        ((10, 1, 0, 0), (10, 1, 11, None, None)),
    ];
    for ((i, o, r, c), (prompt, completion, total, cached, created)) in cases {
        let rid = RequestId::new();
        let mut enc = openai(&rid);
        let out = frames(&run(
            &mut enc,
            &[
                started(),
                usage(Reported(i), Reported(o), Reported(r), Reported(c)),
                completed(StopReason::EndTurn),
            ],
        ));
        let chunk = openai_usage_chunk(&out);
        let u = &chunk["usage"];
        assert_eq!(u["prompt_tokens"], prompt, "{chunk}");
        assert_eq!(u["completion_tokens"], completion, "{chunk}");
        assert_eq!(u["total_tokens"], total, "{chunk}");
        assert_eq!(
            u.get("prompt_tokens_details"),
            cached.map(|n| json!({"cached_tokens": n})).as_ref(),
            "{chunk}"
        );
        assert_eq!(
            u.get("cache_creation_input_tokens"),
            created.map(Value::from).as_ref(),
            "{chunk}"
        );
        assert_eq!(chunk["choices"], json!([]));
    }
}

// Refutes: Usage::update-style overwrite wiping known counts when a later Usage event
// reports only some fields.
#[test]
fn openai_unknown_usage_fields_keep_the_previous_value() {
    let rid = RequestId::new();
    let mut enc = openai(&rid);
    let out = frames(&run(
        &mut enc,
        &[
            started(),
            usage(Reported(120), Reported(1), Reported(30), Unknown),
            usage(Unknown, Reported(42), Unknown, Unknown),
            completed(StopReason::EndTurn),
        ],
    ));
    let chunk = openai_usage_chunk(&out);
    assert_eq!(chunk["usage"]["prompt_tokens"], 150, "{chunk}");
    assert_eq!(chunk["usage"]["completion_tokens"], 42, "{chunk}");
}

// Refutes: the usage chunk moving before finish_reason or after [DONE]; gateway metering and
// OpenAI clients expect finish_reason chunk, then usage chunk, then [DONE].
#[test]
fn openai_usage_chunk_sits_between_finish_reason_and_done_on_both_termination_paths() {
    for via_completed in [true, false] {
        let rid = RequestId::new();
        let mut enc = openai(&rid);
        let mut events = vec![started(), text("hi", 0), usage_event()];
        if via_completed {
            events.push(completed(StopReason::EndTurn));
        }
        let mut out = run(&mut enc, &events);
        out.extend(enc.finish());
        let out = frames(&out);
        let n = out.len();
        assert_eq!(out[n - 1].data, "[DONE]");
        assert!(out[n - 2].json().get("usage").is_some());
        assert_eq!(out[n - 3].json()["choices"][0]["finish_reason"], "stop");
    }
}

// Refutes: emitting a zero-filled usage chunk when the provider reported no usage at all.
#[test]
fn openai_without_usage_emits_no_usage_chunk() {
    let rid = RequestId::new();
    let mut enc = openai(&rid);
    let out = frames(&run(
        &mut enc,
        &[
            started(),
            text("hi", 0),
            usage(Unknown, Unknown, Unknown, Unknown),
            completed(StopReason::EndTurn),
        ],
    ));
    assert!(out
        .iter()
        .filter(|f| f.data != "[DONE]")
        .all(|f| f.json().get("usage").is_none()));
}

// Refutes: finish() always answering `stop` even after tool calls, so agent clients never
// run the tool.
#[test]
fn openai_finish_after_tool_call_reports_tool_calls() {
    let rid = RequestId::new();
    let mut enc = openai(&rid);
    let mut out = run(&mut enc, &[started(), tool_start("call_1", "ls", "{}", 0)]);
    out.extend(enc.finish());
    assert_eq!(openai_finish_reasons(&frames(&out)), [json!("tool_calls")]);
}

// Refutes: finish() reporting tool_calls for a plain text turn.
#[test]
fn openai_finish_without_tool_call_reports_stop() {
    let rid = RequestId::new();
    let mut enc = openai(&rid);
    let mut out = run(&mut enc, &[started(), text("hi", 0)]);
    out.extend(enc.finish());
    assert_eq!(openai_finish_reasons(&frames(&out)), [json!("stop")]);
}

// Refutes: every Failed rendered as `api_error` without a code, or the stream left without
// [DONE] (OpenAI SDKs wait for it).
#[test]
fn openai_failed_renders_error_object_then_done() {
    let cases = [
        (
            VkdgError::UpstreamError {
                code: 429,
                message: "slow down".into(),
                retry_after: None,
            },
            json!({"message":"upstream error 429: slow down","type":"rate_limit_error",
                   "param":null,"code":"rate_limit_exceeded"}),
        ),
        (
            VkdgError::UpstreamError {
                code: 502,
                message: "upstream returned an empty stream".into(),
                retry_after: None,
            },
            json!({"message":"upstream error 502: upstream returned an empty stream",
                   "type":"server_error","param":null,"code":null}),
        ),
    ];
    for (error, body) in cases {
        let rid = RequestId::new();
        let mut enc = openai(&rid);
        let out = frames(&run(&mut enc, &[started(), failed(error)]));
        let n = out.len();
        assert_eq!(out[n - 1].data, "[DONE]");
        assert_eq!(out[n - 2].json(), json!({ "error": body }));
        assert_eq!(enc.encode(&text("late", 0)), Vec::<u8>::new());
        assert_eq!(enc.finish(), Vec::<u8>::new());
    }
}

// Refutes: forwarding an empty tool id (clients cannot answer it), repeating id/name on
// continuation fragments, or leaking the source block index instead of a 0-based wire index.
#[test]
fn openai_tool_calls_get_wire_indices_unique_ids_and_argument_only_continuations() {
    let rid = RequestId::new();
    let mut enc = openai(&rid);
    let out = frames(&run(
        &mut enc,
        &[
            started(),
            tool_start("", "a", "{\"x\":", 5),
            tool_more("1}", 5),
            tool_end(5),
            tool_start("", "b", "{}", 9),
            tool_end(9),
        ],
    ));
    let calls: Vec<Value> = out
        .iter()
        .map(Frame::json)
        .filter_map(|v| v["choices"][0]["delta"]["tool_calls"].get(0).cloned())
        .collect();
    assert_eq!(calls.len(), 3);
    let id_a = calls[0]["id"].as_str().unwrap();
    let id_b = calls[2]["id"].as_str().unwrap();
    assert!(id_a.starts_with("call_") && id_b.starts_with("call_"));
    assert_ne!(id_a, id_b);
    assert_eq!(
        calls[0],
        json!({"index":0,"id":id_a,"type":"function","function":{"name":"a","arguments":"{\"x\":"}})
    );
    assert_eq!(calls[1], json!({"index":0,"function":{"arguments":"1}"}}));
    assert_eq!(
        calls[2],
        json!({"index":1,"id":id_b,"type":"function","function":{"name":"b","arguments":"{}"}})
    );
}

// Refutes: emitting orphan argument fragments (no id/name ever sent for that call) that an
// OpenAI client would attach to nothing.
#[test]
fn openai_fragment_for_unknown_or_closed_call_ends_the_stream_with_an_error() {
    let shapes: [Vec<E>; 2] = [
        vec![started(), tool_more("{}", 3)],
        vec![
            started(),
            tool_start("c1", "ls", "", 0),
            tool_end(0),
            tool_more("{}", 0),
        ],
    ];
    for events in shapes {
        let rid = RequestId::new();
        let mut enc = openai(&rid);
        let out = frames(&run(&mut enc, &events));
        let n = out.len();
        assert_eq!(out[n - 1].data, "[DONE]");
        assert_eq!(
            out[n - 2].json(),
            json!({"error":{"message":"tool call fragments arrived out of order",
                   "type":"server_error","param":null,"code":null}})
        );
        assert_eq!(
            enc.encode(&completed(StopReason::ToolUse)),
            Vec::<u8>::new()
        );
    }
}

// Refutes: reasoning leaking into `content` (clients would show it as the answer).
#[test]
fn openai_reasoning_goes_to_reasoning_content() {
    let rid = RequestId::new();
    let mut enc = openai(&rid);
    let out = frames(&run(&mut enc, &[started(), reasoning("hmm", 0)]));
    assert_eq!(
        out[1].json()["choices"][0]["delta"],
        json!({"reasoning_content":"hmm"})
    );
}

// ── Migrated: pre-existing usage tests ────────────────────────────────────────

#[test]
fn openai_final_usage_chunk_carries_prompt_tokens() {
    let rid = RequestId::new();
    let mut enc = openai(&rid);
    let mut out = enc.encode(&usage_event());
    out.extend(enc.finish());
    let chunk = openai_usage_chunk(&frames(&out));
    // input 500 (excludes cache) + cache_read 40 = OpenAI prompt_tokens 540.
    assert_eq!(chunk["usage"]["prompt_tokens"], 540, "{chunk}");
    assert_eq!(chunk["usage"]["completion_tokens"], 3, "{chunk}");
}
