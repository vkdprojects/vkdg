//! `decode_request` fidelity: request fields the client sent must reach the
//! `ConversationRequest` as the Anthropic Messages docs define them, or be
//! rejected with the wire field named.

use serde_json::{json, Value};
use vkdg_core::VkdgError;
use vkdg_operations::{ContentBlock, ConversationRequest, MessageContent, Operation, ToolChoice};

use crate::decode_request;

fn tools() -> Value {
    json!([
        {"name": "get_weather", "description": "Weather", "input_schema": {"type": "object"}},
        {"name": "get_time", "input_schema": {"type": "object"}}
    ])
}

/// A minimal valid request with `extra` object members merged in.
fn body_with(extra: Value) -> Vec<u8> {
    let mut body = json!({
        "model": "claude-sonnet-4-5",
        "max_tokens": 64,
        "messages": [{"role": "user", "content": "hi"}]
    });
    let (Some(target), Value::Object(extra)) = (body.as_object_mut(), extra) else {
        panic!("test bodies are objects")
    };
    target.extend(extra);
    body.to_string().into_bytes()
}

fn conversation(body: &[u8]) -> Result<ConversationRequest, VkdgError> {
    match decode_request(body)?.1 {
        Operation::Conversation(req) => Ok(req),
        other => panic!("expected conversation, got {other:?}"),
    }
}

fn rejected_field(body: &[u8]) -> String {
    match conversation(body) {
        Err(VkdgError::ConfigInvalid { field, .. }) => field,
        other => panic!("expected ConfigInvalid, got {other:?}"),
    }
}

/// The single block of the last message, which every history test builds.
fn only_block(req: &ConversationRequest) -> &ContentBlock {
    let Some(last) = req.messages.last() else {
        panic!("no messages")
    };
    let MessageContent::Blocks(blocks) = &last.content else {
        panic!("expected blocks, got {:?}", last.content)
    };
    assert_eq!(blocks.len(), 1, "{blocks:?}");
    &blocks[0]
}

fn tool_result_text(content: &Value) -> String {
    let body = body_with(json!({"messages": [
        {"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": "toolu_01", "content": content}
        ]}
    ]}));
    let req = conversation(&body).expect("tool_result must decode");
    match only_block(&req) {
        ContentBlock::ToolResult { content, .. } => content.clone(),
        other => panic!("expected ToolResult, got {other:?}"),
    }
}

// ── tool_choice ───────────────────────────────────────────────────────────────

// Defeat: ignoring `tool_choice` (the model answers in prose when the client
// forced a tool), mapping `any` to Auto, or leaking `disable_parallel_tool_use`
// from a `none` choice where Anthropic defines it as meaningless.
#[test]
fn tool_choice_maps_each_documented_type() {
    let cases = [
        (json!({"type": "auto"}), Some(ToolChoice::Auto), false),
        (json!({"type": "any"}), Some(ToolChoice::Required), false),
        (json!({"type": "none"}), Some(ToolChoice::Disabled), false),
        (
            json!({"type": "tool", "name": "get_weather"}),
            Some(ToolChoice::Named("get_weather".into())),
            false,
        ),
        (
            json!({"type": "auto", "disable_parallel_tool_use": true}),
            Some(ToolChoice::Auto),
            true,
        ),
        (
            json!({"type": "any", "disable_parallel_tool_use": true}),
            Some(ToolChoice::Required),
            true,
        ),
        (
            json!({"type": "tool", "name": "get_time", "disable_parallel_tool_use": true}),
            Some(ToolChoice::Named("get_time".into())),
            true,
        ),
        (
            json!({"type": "auto", "disable_parallel_tool_use": false}),
            Some(ToolChoice::Auto),
            false,
        ),
        (
            json!({"type": "none", "disable_parallel_tool_use": true}),
            Some(ToolChoice::Disabled),
            false,
        ),
    ];
    for (wire, want_choice, want_disable) in cases {
        let body = body_with(json!({"tools": tools(), "tool_choice": wire}));
        let req = conversation(&body).unwrap_or_else(|e| panic!("{wire}: {e:?}"));
        assert_eq!(req.tool_choice, want_choice, "{wire}");
        assert_eq!(req.disable_parallel_tool_use, want_disable, "{wire}");
    }
}

// Defeat: inventing a default (`auto`) for a request that said nothing, which
// would override a provider's own default or add a forbidden field.
#[test]
fn absent_or_null_tool_choice_is_none() {
    for extra in [
        json!({"tools": tools()}),
        json!({"tools": tools(), "tool_choice": null}),
    ] {
        let req = conversation(&body_with(extra)).expect("must decode");
        assert_eq!(req.tool_choice, None);
        assert!(!req.disable_parallel_tool_use);
    }
}

// Defeat: a lenient decoder that defaults unknown or malformed choices to
// `auto`, silently running a request the client constrained, or a panic on a
// non-object value.
#[test]
fn malformed_tool_choice_is_rejected_naming_the_field() {
    let bad = [
        json!("auto"),
        json!(["auto"]),
        json!(7),
        json!({}),
        json!({"type": 5}),
        json!({"type": "bogus"}),
        json!({"type": "tool"}),
        json!({"type": "tool", "name": ""}),
        json!({"type": "tool", "name": 5}),
        json!({"type": "auto", "disable_parallel_tool_use": "yes"}),
        json!({"type": "any", "disable_parallel_tool_use": 1}),
        json!({"type": "none", "disable_parallel_tool_use": "yes"}),
    ];
    for wire in bad {
        let body = body_with(json!({"tools": tools(), "tool_choice": wire}));
        assert_eq!(rejected_field(&body), "tool_choice", "{wire}");
    }
}

// Defeat: skipping `ToolChoice::settle`, which forwards `tool_choice` without
// tools (both providers answer 400), or accepts a forced/named tool that cannot
// be called.
#[test]
fn tool_choice_is_settled_against_the_tools_sent() {
    for wire in [json!({"type": "auto"}), json!({"type": "none"})] {
        let req = conversation(&body_with(json!({"tool_choice": wire}))).expect("must decode");
        assert_eq!(req.tool_choice, None, "{wire}");
    }
    // A parallel-call restriction with no tools has nothing to restrict.
    let req = conversation(&body_with(json!({
        "tool_choice": {"type": "auto", "disable_parallel_tool_use": true}
    })))
    .expect("must decode");
    assert_eq!(req.tool_choice, None);
    assert!(!req.disable_parallel_tool_use);
    let rejected = [
        body_with(json!({"tool_choice": {"type": "any"}})),
        body_with(json!({"tool_choice": {"type": "tool", "name": "get_weather"}})),
        body_with(json!({
            "tools": tools(),
            "tool_choice": {"type": "tool", "name": "not_a_tool"}
        })),
    ];
    for body in rejected {
        assert_eq!(rejected_field(&body), "tool_choice");
    }
}

// ── stop_sequences / top_p ────────────────────────────────────────────────────

// Defeat: dropping `stop_sequences` (the model runs past the client's
// delimiter), reordering them, or forwarding an empty string Anthropic rejects.
#[test]
fn stop_sequences_are_carried_in_order_without_empty_strings() {
    let cases = [
        (json!(["END", "\n\nHuman:"]), vec!["END", "\n\nHuman:"]),
        (json!(["a", "", "b"]), vec!["a", "b"]),
        (json!([]), vec![]),
        (json!(null), vec![]),
    ];
    for (wire, want) in cases {
        let req = conversation(&body_with(json!({"stop_sequences": wire}))).expect("decode");
        assert_eq!(req.stop_sequences, want, "{wire}");
    }
    let absent = conversation(&body_with(json!({})))
        .expect("decode")
        .stop_sequences;
    assert_eq!(absent, Vec::<String>::new());
}

// Defeat: copying an unbounded hostile array into every upstream attempt, or
// reporting the failure against a field the client never sent.
#[test]
fn hostile_stop_sequences_are_rejected_naming_the_field() {
    let thousand: Vec<String> = (0..1000).map(|i| format!("stop-{i}")).collect();
    let long = "x".repeat(257);
    for wire in [json!(thousand), json!([long])] {
        let body = body_with(json!({"stop_sequences": wire}));
        assert_eq!(rejected_field(&body), "stop_sequences");
    }
}

// Defeat: dropping `top_p`, or passing an out-of-range value upstream where it
// becomes an opaque provider 400.
#[test]
fn top_p_is_carried_or_rejected_naming_the_field() {
    for (wire, want) in [(0.9, 0.9_f32), (0.0, 0.0), (1.0, 1.0)] {
        let req = conversation(&body_with(json!({"top_p": wire}))).expect("decode");
        assert_eq!(req.top_p, Some(want), "{wire}");
    }
    assert_eq!(
        conversation(&body_with(json!({"top_p": null})))
            .expect("decode")
            .top_p,
        None
    );
    assert_eq!(
        conversation(&body_with(json!({}))).expect("decode").top_p,
        None
    );
    for wire in [1.5, -0.1] {
        assert_eq!(rejected_field(&body_with(json!({"top_p": wire}))), "top_p");
    }
}

// ── tool_result content ───────────────────────────────────────────────────────

// Defeat: JSON-serializing the sub-block array into the string, so the model
// receives `[{"type":"text","text":"..."}]` (escaped JSON) as the tool output;
// or joining without a separator; or fabricating text for an image.
#[test]
fn tool_result_sub_blocks_flatten_to_their_text() {
    let png = json!({"type": "image", "source": {
        "type": "base64", "media_type": "image/png", "data": "iVBORw0KGgo="}});
    let cases = [
        (json!("15 degrees"), "15 degrees"),
        (
            json!([{"type": "text", "text": "15 degrees"}]),
            "15 degrees",
        ),
        (
            json!([{"type": "text", "text": "line one"}, {"type": "text", "text": "line \"two\""}]),
            "line one\nline \"two\"",
        ),
        (
            json!([{"type": "text", "text": "before"}, png.clone(), {"type": "text", "text": "after"}]),
            "before\nafter",
        ),
        (json!([png]), ""),
        (json!([]), ""),
        (
            json!([{"type": "document", "source": {"type": "text", "data": "x"}},
                   {"type": "text", "text": "kept"}]),
            "kept",
        ),
    ];
    for (wire, want) in cases {
        assert_eq!(tool_result_text(&wire), want, "{wire}");
    }
}

// Defeat: a missing `content` (allowed by the docs for an empty result) failing
// the request or yielding the JSON text `null`.
#[test]
fn tool_result_without_content_is_empty() {
    let body = body_with(json!({"messages": [
        {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "toolu_01"}]}
    ]}));
    let req = conversation(&body).expect("decode");
    let ContentBlock::ToolResult {
        content,
        tool_use_id,
        is_error,
    } = only_block(&req)
    else {
        panic!("expected ToolResult")
    };
    assert_eq!(content, "");
    assert_eq!(tool_use_id, "toolu_01");
    assert!(!is_error);
}

// ── tool_use input ────────────────────────────────────────────────────────────

// Defeat: decoding a missing `input` as JSON null, which providers reject as a
// tool-call argument object; or rewriting inputs that were sent.
#[test]
fn tool_use_input_defaults_to_an_empty_object_and_keeps_sent_values() {
    let cases = [
        (
            json!({"type": "tool_use", "id": "toolu_01", "name": "get_time"}),
            json!({}),
        ),
        (
            json!({"type": "tool_use", "id": "toolu_01", "name": "get_time", "input": null}),
            json!({}),
        ),
        (
            json!({"type": "tool_use", "id": "toolu_01", "name": "get_time", "input": {}}),
            json!({}),
        ),
        (
            json!({"type": "tool_use", "id": "toolu_01", "name": "get_weather",
                   "input": {"city": "Paris", "opts": {"units": ["c"], "n": 2}}}),
            json!({"city": "Paris", "opts": {"units": ["c"], "n": 2}}),
        ),
    ];
    for (wire, want) in cases {
        let body = body_with(json!({"messages": [
            {"role": "user", "content": "hi"},
            {"role": "assistant", "content": [wire]}
        ]}));
        let req = conversation(&body).expect("decode");
        let ContentBlock::ToolUse { id, name, input } = only_block(&req) else {
            panic!("expected ToolUse")
        };
        assert_eq!(id, "toolu_01");
        assert!(name.starts_with("get_"));
        assert_eq!(input, &want, "{wire}");
    }
}
