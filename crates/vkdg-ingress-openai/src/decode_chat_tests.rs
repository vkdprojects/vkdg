//! Contract tests for `decode_request` (Chat Completions). Expected values are written
//! by hand from the `OpenAI` Chat Completions reference, not derived from the decoder.

use serde_json::{json, Value};
use vkdg_core::{Capability, VkdgError};
use vkdg_operations::{ConversationRequest, MessageContent, Operation, Role, ToolChoice};

use crate::decode::decode_request;

fn conversation(body: &Value) -> ConversationRequest {
    let bytes = serde_json::to_vec(body).expect("test body serialises");
    match decode_request(&bytes) {
        Ok((_, Operation::Conversation(req))) => req,
        other => panic!("expected a decoded conversation, got {other:?}"),
    }
}

/// Decodes `body` and returns the `(field, message)` of the `ConfigInvalid` it must produce.
fn rejection(body: &Value) -> (String, String) {
    let bytes = serde_json::to_vec(body).expect("test body serialises");
    match decode_request(&bytes) {
        Err(VkdgError::ConfigInvalid { field, message }) => (field, message),
        other => panic!("expected ConfigInvalid, got {other:?}"),
    }
}

/// Request with a user turn plus the given extra top-level fields.
fn with_fields(extra: Value) -> Value {
    let mut body = json!({
        "model": "gpt-4o",
        "messages": [{"role": "user", "content": "hi"}],
    });
    if let (Some(map), Value::Object(more)) = (body.as_object_mut(), extra) {
        map.extend(more);
    }
    body
}

fn with_messages(messages: impl Into<Value>) -> Value {
    json!({"model": "gpt-4o", "messages": messages.into()})
}

fn weather_tool() -> Value {
    json!({"type": "function", "function": {
        "name": "get_weather",
        "description": "Weather",
        "parameters": {"type": "object", "properties": {"city": {"type": "string"}}}
    }})
}

fn messages_json(req: &ConversationRequest) -> Value {
    serde_json::to_value(&req.messages).expect("messages serialise")
}

// ── Characterisation of behaviour that must survive ──────────────────────────

// Defeat: mapping role:user content to the wrong type or losing the text.
#[test]
fn plain_user_string_stays_text() {
    let req = conversation(&with_messages(
        json!([{"role": "user", "content": "hello"}]),
    ));
    assert_eq!(req.messages.len(), 1);
    assert_eq!(req.messages[0].role, Role::User);
    assert!(matches!(&req.messages[0].content, MessageContent::Text(t) if t == "hello"));
}

// Defeat: dropping the stream flag so callers always get buffered responses.
#[test]
fn stream_flag_is_kept() {
    let req = conversation(&with_fields(json!({"stream": true})));
    assert!(req.stream);
}

// Defeat: silently accepting n>1 and producing invalid downstream behaviour.
#[test]
fn n_greater_than_one_is_rejected() {
    let (field, _) = rejection(&with_fields(json!({"n": 2})));
    assert_eq!(field, "n");
}

// Defeat: ignoring response_format so json_object requests get no JsonSchema capability.
#[test]
fn json_object_response_format_requires_json_schema() {
    let req = conversation(&with_fields(
        json!({"response_format": {"type": "json_object"}}),
    ));
    assert!(req
        .required_capabilities
        .0
        .contains(&Capability::JsonSchema));
}

// Defeat: reasoning_effort lost or left unnormalised.
#[test]
fn reasoning_effort_is_trimmed_and_lowercased() {
    let req = conversation(&with_fields(json!({"reasoning_effort": " HIGH "})));
    let thinking = req.thinking.expect("reasoning_effort sets thinking");
    assert_eq!(thinking.effort.as_deref(), Some("high"));
    assert_eq!(thinking.budget_tokens, None);
}

// ── tool_choice ───────────────────────────────────────────────────────────────

// Defeat: dropping tool_choice (client forcing a tool gets a free-form answer) or
// mapping `none`/`required`/named to the wrong variant.
#[test]
fn tool_choice_wire_values_map_to_variants() {
    let named = json!({"type": "function", "function": {"name": "get_weather"}});
    let cases: Vec<(Value, Option<ToolChoice>)> = vec![
        (json!("auto"), Some(ToolChoice::Auto)),
        (json!("none"), Some(ToolChoice::Disabled)),
        (json!("required"), Some(ToolChoice::Required)),
        (named, Some(ToolChoice::Named("get_weather".into()))),
        (Value::Null, None),
    ];
    for (wire, expected) in cases {
        let req = conversation(&with_fields(json!({
            "tools": [weather_tool()],
            "tool_choice": wire,
        })));
        assert_eq!(req.tool_choice, expected, "tool_choice {wire}");
    }
}

// Defeat: inventing a tool_choice when the client sent none.
#[test]
fn absent_tool_choice_is_none() {
    let req = conversation(&with_fields(json!({"tools": [weather_tool()]})));
    assert_eq!(req.tool_choice, None);
}

// Defeat: forwarding a no-op `auto`/`none` without tools (providers reject it).
#[test]
fn no_op_tool_choice_without_tools_is_dropped() {
    for wire in ["auto", "none"] {
        let req = conversation(&with_fields(json!({"tool_choice": wire})));
        assert_eq!(req.tool_choice, None, "tool_choice {wire}");
    }
}

// Defeat: silently accepting a tool_choice shape the gateway cannot honour.
#[test]
fn unsupported_or_malformed_tool_choice_is_rejected() {
    let cases = [
        json!("sometimes"),
        json!(""),
        json!({"type": "allowed_tools", "allowed_tools": {"mode": "auto", "tools": []}}),
        json!({"type": "custom", "custom": {"name": "x"}}),
        json!({"type": "function"}),
        json!({"type": "function", "function": {}}),
        json!({"type": "function", "function": {"name": ""}}),
        json!({"type": "function", "function": {"name": 7}}),
        json!({"function": {"name": "get_weather"}}),
        json!(7),
        json!(true),
        json!(["auto"]),
    ];
    for wire in cases {
        let (field, _) = rejection(&with_fields(json!({
            "tools": [weather_tool()],
            "tool_choice": wire,
        })));
        assert_eq!(field, "tool_choice", "tool_choice {wire}");
    }
}

// Defeat: passing Required/Named that no tool can satisfy upstream instead of failing here.
#[test]
fn unsatisfiable_tool_choice_is_rejected() {
    let missing_name = json!({"type": "function", "function": {"name": "nope"}});
    for (tools, wire) in [
        (json!([]), json!("required")),
        (json!([weather_tool()]), missing_name.clone()),
        (json!([]), missing_name),
    ] {
        let (field, _) = rejection(&with_fields(json!({"tools": tools, "tool_choice": wire})));
        assert_eq!(field, "tool_choice", "tool_choice {wire}");
    }
}

// ── parallel_tool_calls ───────────────────────────────────────────────────────

// Defeat: ignoring parallel_tool_calls:false, or inverting the flag.
#[test]
fn parallel_tool_calls_false_disables_parallel_use() {
    let cases = [
        (json!({"parallel_tool_calls": false}), true),
        (json!({"parallel_tool_calls": true}), false),
        (json!({"parallel_tool_calls": null}), false),
        (json!({}), false),
    ];
    for (extra, expected) in cases {
        let req = conversation(&with_fields(extra.clone()));
        assert_eq!(req.disable_parallel_tool_use, expected, "{extra}");
    }
}

// ── max tokens ────────────────────────────────────────────────────────────────

// Defeat: reading only max_tokens (the deprecated field) so max_completion_tokens is lost,
// or letting the deprecated field win when both are present.
#[test]
fn max_completion_tokens_wins_over_max_tokens() {
    let cases = [
        (
            json!({"max_completion_tokens": 100, "max_tokens": 50}),
            Some(100),
        ),
        (json!({"max_completion_tokens": 100}), Some(100)),
        (json!({"max_tokens": 50}), Some(50)),
        (
            json!({"max_completion_tokens": null, "max_tokens": 50}),
            Some(50),
        ),
        (json!({}), None),
    ];
    for (extra, expected) in cases {
        let req = conversation(&with_fields(extra.clone()));
        assert_eq!(req.max_tokens, expected, "{extra}");
    }
}

// ── stop ──────────────────────────────────────────────────────────────────────

// Defeat: dropping `stop`, or handling only the array form.
#[test]
fn stop_accepts_string_or_array() {
    let cases = [
        (json!({"stop": "END"}), vec!["END"]),
        (json!({"stop": ["a", "b"]}), vec!["a", "b"]),
        (json!({"stop": ["", "a"]}), vec!["a"]),
        (json!({"stop": []}), vec![]),
        (json!({"stop": null}), vec![]),
        (json!({}), vec![]),
    ];
    for (extra, expected) in cases {
        let req = conversation(&with_fields(extra.clone()));
        assert_eq!(req.stop_sequences, expected, "{extra}");
    }
}

// Defeat: coercing or skipping non-string stop entries, and copying hostile input upstream.
#[test]
fn malformed_or_oversized_stop_is_rejected() {
    let many: Vec<String> = (0..1000).map(|i| format!("s{i}")).collect();
    let cases = [
        json!(5),
        json!(true),
        json!({"a": 1}),
        json!([1]),
        json!(["a", null]),
        json!([["nested"]]),
        json!(many),
        json!("x".repeat(10 * 1024)),
        json!(["ok", "x".repeat(257)]),
    ];
    for wire in cases {
        let (field, _) = rejection(&with_fields(json!({"stop": wire})));
        assert_eq!(field, "stop");
    }
}

// ── top_p ─────────────────────────────────────────────────────────────────────

// Defeat: dropping top_p, or letting an out-of-range value reach the provider.
#[test]
fn top_p_is_carried_and_range_checked() {
    assert_eq!(
        conversation(&with_fields(json!({"top_p": 0.5}))).top_p,
        Some(0.5)
    );
    assert_eq!(conversation(&with_fields(json!({}))).top_p, None);
    for bad in [json!(1.5), json!(-0.1)] {
        let (field, _) = rejection(&with_fields(json!({"top_p": bad})));
        assert_eq!(field, "top_p", "top_p {bad}");
    }
}

// ── roles: system / developer ─────────────────────────────────────────────────

// Defeat: keeping only the first system message, or ignoring developer messages.
#[test]
fn system_and_developer_messages_are_joined_in_order() {
    let req = conversation(&with_messages(json!([
        {"role": "system", "content": "a"},
        {"role": "developer", "content": "b"},
        {"role": "user", "content": "hi"},
        {"role": "system", "content": [{"type": "text", "text": "c1"}, {"type": "text", "text": "c2"}]},
        {"role": "developer", "content": ""},
        {"role": "system", "content": null},
    ])));
    assert_eq!(req.system.as_deref(), Some("a\n\nb\n\nc1\nc2"));
    assert_eq!(req.messages.len(), 1);
    assert_eq!(req.messages[0].role, Role::User);
}

// Defeat: emitting Some("") (an empty system prompt) when every system message is blank.
#[test]
fn blank_system_messages_leave_system_unset() {
    let req = conversation(&with_messages(json!([
        {"role": "system", "content": ""},
        {"role": "user", "content": "hi"},
    ])));
    assert_eq!(req.system, None);
}

// Defeat: treating an unknown role as `user` (legacy `function` results would become user text).
#[test]
fn unknown_role_is_rejected_with_message_index() {
    for role in ["function", "bogus"] {
        let (field, message) = rejection(&with_messages(json!([
            {"role": "user", "content": "hi"},
            {"role": role, "name": "f", "content": "x"},
        ])));
        assert_eq!(field, "messages[1].role");
        assert!(message.contains(role), "message names the role: {message}");
    }
}

// ── assistant ─────────────────────────────────────────────────────────────────

// Defeat: dropping the assistant's text when tool_calls are present.
#[test]
fn assistant_text_precedes_tool_calls() {
    let req = conversation(&with_messages(json!([
        {"role": "assistant", "content": "Let me check.", "tool_calls": [
            {"id": "call_1", "type": "function", "function": {"name": "get_weather", "arguments": "{\"city\":\"Paris\"}"}},
            {"id": "call_2", "type": "function", "function": {"name": "get_time", "arguments": "{}"}},
        ]},
    ])));
    assert_eq!(
        messages_json(&req),
        json!([{"role": "Assistant", "content": [
            {"type": "text", "text": "Let me check."},
            {"type": "tool_use", "id": "call_1", "name": "get_weather", "input": {"city": "Paris"}},
            {"type": "tool_use", "id": "call_2", "name": "get_time", "input": {}},
        ]}])
    );
}

// Defeat: emitting an empty Text block before tool calls (upstreams reject empty text).
#[test]
fn assistant_empty_or_null_content_adds_no_text_block() {
    for content in [json!(null), json!(""), json!([])] {
        let req = conversation(&with_messages(json!([
            {"role": "assistant", "content": content, "tool_calls": [
                {"id": "call_1", "type": "function", "function": {"name": "f", "arguments": "{}"}},
            ]},
        ])));
        assert_eq!(
            messages_json(&req),
            json!([{"role": "Assistant", "content": [
                {"type": "tool_use", "id": "call_1", "name": "f", "input": {}},
            ]}]),
            "content {content}"
        );
    }
}

// Defeat: producing Value::Null (or failing) for blank/invalid/non-object arguments, which
// wedges every later turn when the client replays model output verbatim.
#[test]
fn tool_call_arguments_always_decode_to_an_object() {
    let cases = [
        ("", json!({})),
        (" \n\t ", json!({})),
        ("{}", json!({})),
        (
            r#"{"city":"Paris","days":3}"#,
            json!({"city": "Paris", "days": 3}),
        ),
        (r#"{"a":"#, json!({})),
        ("not json", json!({})),
        ("[1,2]", json!({})),
        (r#""text""#, json!({})),
        ("42", json!({})),
        ("null", json!({})),
    ];
    for (arguments, expected) in cases {
        let req = conversation(&with_messages(json!([
            {"role": "assistant", "content": null, "tool_calls": [
                {"id": "call_1", "type": "function", "function": {"name": "f", "arguments": arguments}},
            ]},
        ])));
        assert_eq!(
            messages_json(&req),
            json!([{"role": "Assistant", "content": [
                {"type": "tool_use", "id": "call_1", "name": "f", "input": expected},
            ]}]),
            "arguments {arguments:?}"
        );
    }
}

// Defeat: dropping refusal parts or failing on assistant content arrays.
#[test]
fn assistant_content_parts_keep_text_and_refusal() {
    let req = conversation(&with_messages(json!([
        {"role": "assistant", "content": [
            {"type": "text", "text": "Sorry."},
            {"type": "refusal", "refusal": "I can't help with that."},
        ]},
    ])));
    assert!(matches!(
        &req.messages[0].content,
        MessageContent::Text(t) if t == "Sorry.\nI can't help with that."
    ));
}

// Defeat: silently dropping unknown assistant parts (e.g. audio) instead of naming them.
#[test]
fn unknown_assistant_part_is_rejected() {
    let (field, message) = rejection(&with_messages(json!([
        {"role": "user", "content": "hi"},
        {"role": "assistant", "content": [{"type": "audio", "audio": {"id": "a1"}}]},
    ])));
    assert_eq!(field, "messages[1].content");
    assert!(message.contains("audio"), "{message}");
}

// Defeat: a tool_calls list on a user message being decoded as the assistant's calls.
#[test]
fn tool_calls_on_non_assistant_message_are_rejected() {
    let (field, _) = rejection(&with_messages(json!([
        {"role": "user", "content": "hi", "tool_calls": [
            {"id": "call_1", "type": "function", "function": {"name": "f", "arguments": "{}"}},
        ]},
    ])));
    assert_eq!(field, "messages[0].tool_calls");
}

// ── tool messages ─────────────────────────────────────────────────────────────

// Defeat: losing tool_call_id, or merging consecutive tool messages (adapters group them).
#[test]
fn each_tool_message_stays_its_own_tool_result() {
    let req = conversation(&with_messages(json!([
        {"role": "tool", "tool_call_id": "call_1", "content": "42"},
        {"role": "tool", "tool_call_id": "call_2", "content": [
            {"type": "text", "text": "line one"},
            {"type": "text", "text": "line two"},
        ]},
    ])));
    assert_eq!(
        messages_json(&req),
        json!([
            {"role": "Tool", "content": [
                {"type": "tool_result", "tool_use_id": "call_1", "content": "42", "is_error": false},
            ]},
            {"role": "Tool", "content": [
                {"type": "tool_result", "tool_use_id": "call_2", "content": "line one\nline two", "is_error": false},
            ]},
        ])
    );
}

// Defeat: silently dropping a part type a tool message cannot carry (audio), which would
// hand the model a result that is not what the tool returned.
#[test]
fn tool_content_rejects_parts_that_are_neither_text_nor_image() {
    let (field, message) = rejection(&with_messages(json!([
        {"role": "tool", "tool_call_id": "call_1", "content": [
            {"type": "input_audio", "input_audio": {"data": "AAAA", "format": "wav"}},
        ]},
    ])));
    assert_eq!(field, "messages[0].content");
    assert!(message.contains("input_audio"), "{message}");
}

// ── user content arrays ───────────────────────────────────────────────────────

// Defeat: flattening images into prompt text (corrupting the prompt), losing the media type,
// or misclassifying data URLs vs remote URLs.
#[test]
fn user_images_decode_to_image_blocks() {
    let req = conversation(&with_messages(json!([
        {"role": "user", "content": [
            {"type": "text", "text": "What is in these?"},
            {"type": "image_url", "image_url": {"url": "data:image/png;base64,iVBORw0KGgo=", "detail": "high"}},
            {"type": "image_url", "image_url": {"url": "https://example.com/cat.jpg"}},
            {"type": "text", "text": "Answer briefly."},
        ]},
    ])));
    assert_eq!(
        messages_json(&req),
        json!([{"role": "User", "content": [
            {"type": "text", "text": "What is in these?"},
            {"type": "image", "media_type": "image/png", "data": {"Base64": {"data": "iVBORw0KGgo="}}},
            {"type": "image", "media_type": "", "data": {"Url": {"url": "https://example.com/cat.jpg"}}},
            {"type": "text", "text": "Answer briefly."},
        ]}])
    );
}

// Defeat: a data-URL media type parameter (`;charset=`) leaking into the media type.
#[test]
fn data_url_media_type_ignores_parameters() {
    let req = conversation(&with_messages(json!([
        {"role": "user", "content": [
            {"type": "image_url", "image_url": {"url": "data:image/webp;foo=bar;base64,QUJD"}},
        ]},
    ])));
    assert_eq!(
        messages_json(&req),
        json!([{"role": "User", "content": [
            {"type": "image", "media_type": "image/webp", "data": {"Base64": {"data": "QUJD"}}},
        ]}])
    );
}

// Defeat: turning an all-text user array into Blocks (changes behaviour for text-only clients)
// or dropping a part.
#[test]
fn all_text_user_array_is_joined_with_newline() {
    let req = conversation(&with_messages(json!([
        {"role": "user", "content": [
            {"type": "text", "text": "one"},
            {"type": "text", "text": "two"},
        ]},
    ])));
    assert!(matches!(
        &req.messages[0].content,
        MessageContent::Text(t) if t == "one\ntwo"
    ));
}

// Defeat: ignoring unsupported user parts (audio, files) so the model answers without them.
#[test]
fn unknown_user_part_is_rejected_naming_the_type() {
    for kind in ["input_audio", "file"] {
        let (field, message) = rejection(&with_messages(json!([
            {"role": "user", "content": "first"},
            {"role": "assistant", "content": "ok"},
            {"role": "user", "content": [
                {"type": "text", "text": "listen"},
                {"type": kind},
            ]},
        ])));
        assert_eq!(field, "messages[2].content");
        assert!(message.contains(kind), "{message}");
    }
}

// ── images inside tool results ────────────────────────────────────────────────

// Defeat: rejecting or flattening an `image_url` part of a `tool` message (clients that
// return screenshots send them this way), or leaving the data URL as an opaque string.
#[test]
fn tool_message_image_parts_become_result_images() {
    let req = conversation(&with_messages(json!([
        {"role": "user", "content": "look"},
        {"role": "assistant", "content": null, "tool_calls": [
            {"id": "call_1", "type": "function", "function": {"name": "screenshot", "arguments": "{}"}}
        ]},
        {"role": "tool", "tool_call_id": "call_1", "content": [
            {"type": "text", "text": "page loaded"},
            {"type": "image_url", "image_url": {"url": "data:image/png;base64,AAAA"}},
            {"type": "image_url", "image_url": {"url": "https://x.test/a.png"}}
        ]}
    ])));
    let result = &messages_json(&req)[2]["content"][0];
    assert_eq!(result["type"], "tool_result");
    assert_eq!(result["content"], "page loaded");
    assert_eq!(
        result["images"],
        json!([
            {"media_type": "image/png", "data": {"Base64": {"data": "AAAA"}}},
            {"media_type": "", "data": {"Url": {"url": "https://x.test/a.png"}}}
        ])
    );
}
