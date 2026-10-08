//! `decode_request` fidelity: request fields the client sent must reach the
//! `ConversationRequest` as the Anthropic Messages docs define them, or be
//! rejected with the wire field named.

use serde_json::{json, Value};
use vkdg_core::VkdgError;
use vkdg_operations::{
    CacheControl, ContentBlock, ConversationRequest, MessageContent, Operation, SystemBlock,
    ToolChoice,
};

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
        ..
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
        let ContentBlock::ToolUse {
            id, name, input, ..
        } = only_block(&req)
        else {
            panic!("expected ToolUse")
        };
        assert_eq!(id, "toolu_01");
        assert!(name.starts_with("get_"));
        assert_eq!(input, &want, "{wire}");
    }
}

// ── images inside tool results ────────────────────────────────────────────────

// Defeat: keeping only the text of an array result, which loses the screenshot a tool
// returned (`computer`-style tools answer with images). Images keep their order and form.
#[test]
fn tool_result_array_keeps_its_images() {
    let body = body_with(json!({"messages": [
        {"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": "toolu_01", "content": [
                {"type": "text", "text": "page loaded"},
                {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "AAAA"}},
                {"type": "image", "source": {"type": "url", "url": "https://x.test/a.png"}},
                {"type": "text", "text": "done"}
            ]}
        ]}
    ]}));
    let req = conversation(&body).expect("decode");
    let ContentBlock::ToolResult {
        content, images, ..
    } = only_block(&req)
    else {
        panic!("expected ToolResult")
    };
    assert_eq!(content, "page loaded\ndone");
    assert_eq!(
        images,
        &vec![
            vkdg_operations::ToolResultImage {
                media_type: "image/png".into(),
                data: vkdg_operations::ImageData::Base64 {
                    data: "AAAA".into()
                },
            },
            vkdg_operations::ToolResultImage {
                media_type: "image/jpeg".into(),
                data: vkdg_operations::ImageData::Url {
                    url: "https://x.test/a.png".into()
                },
            },
        ]
    );
}

// ── server tools ──────────────────────────────────────────────────────────────

// Defeat: answering 400 "missing field `input_schema`" for a server tool (what the real
// API accepted as `web_search_20250305`), or turning it into a custom tool.
#[test]
fn server_tool_declarations_are_kept_verbatim_beside_custom_tools() {
    let search = json!({"type": "web_search_20250305", "name": "web_search", "max_uses": 2});
    let mut all = tools();
    all.as_array_mut().unwrap().push(search.clone());
    let req = conversation(&body_with(json!({
        "tools": all,
        "tool_choice": {"type": "tool", "name": "web_search"}
    })))
    .expect("decode");
    assert_eq!(req.tools.len(), 2);
    assert_eq!(req.server_tools.len(), 1);
    assert_eq!(req.server_tools[0].name, "web_search");
    assert_eq!(req.server_tools[0].declaration, search);
    assert_eq!(
        req.tool_choice,
        Some(ToolChoice::Named("web_search".into()))
    );
}

// Defeat: accepting a tool entry that is neither a custom tool nor a typed server tool.
#[test]
fn a_tool_without_schema_or_type_is_rejected_naming_its_index() {
    let field = rejected_field(&body_with(json!({"tools": [{"name": "x"}]})));
    assert_eq!(field, "tools[0]");
}

// ── cache_control ─────────────────────────────────────────────────────────────

fn last_blocks(req: &ConversationRequest) -> &[ContentBlock] {
    let Some(last) = req.messages.last() else {
        panic!("no messages")
    };
    let MessageContent::Blocks(blocks) = &last.content else {
        panic!("expected blocks, got {:?}", last.content)
    };
    blocks
}

#[allow(clippy::unnecessary_wraps)] // fills the `Option<CacheControl>` field of expected blocks
fn marker(ttl: Option<&str>) -> Option<CacheControl> {
    Some(CacheControl {
        ttl: ttl.map(str::to_owned),
    })
}

// Defeat: dropping the client's breakpoints at the door, so the upstream never
// caches what the client asked it to, or losing the 1h `ttl` on the way.
#[test]
fn cache_control_on_every_block_kind_is_carried_with_its_ttl() {
    let body = body_with(json!({"messages": [{"role": "user", "content": [
        {"type": "text", "text": "a", "cache_control": {"type": "ephemeral"}},
        {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "AAAA"},
         "cache_control": {"type": "ephemeral", "ttl": "1h"}},
        {"type": "tool_use", "id": "toolu_1", "name": "t", "input": {},
         "cache_control": {"type": "ephemeral", "ttl": "5m"}},
        {"type": "tool_result", "tool_use_id": "toolu_1", "content": "r",
         "cache_control": {"type": "ephemeral"}},
        {"type": "text", "text": "unmarked"}
    ]}]}));
    let req = conversation(&body).unwrap();
    let blocks = last_blocks(&req);
    let found: Vec<Option<CacheControl>> = blocks
        .iter()
        .map(|b| match b {
            ContentBlock::Text { cache_control, .. }
            | ContentBlock::Image { cache_control, .. }
            | ContentBlock::ToolUse { cache_control, .. }
            | ContentBlock::ToolResult { cache_control, .. } => cache_control.clone(),
            other => panic!("unexpected block {other:?}"),
        })
        .collect();
    assert_eq!(
        found,
        vec![
            marker(None),
            marker(Some("1h")),
            marker(Some("5m")),
            marker(None),
            None
        ]
    );
}

// Defeat: marking a custom tool's definition is lost, so the tools prefix is
// never cached.
#[test]
fn cache_control_on_a_tool_definition_is_carried() {
    let body = body_with(json!({"tools": [
        {"name": "a", "input_schema": {"type": "object"}},
        {"name": "b", "input_schema": {"type": "object"},
         "cache_control": {"type": "ephemeral", "ttl": "1h"}}
    ]}));
    let req = conversation(&body).unwrap();
    assert_eq!(req.tools[0].cache_control, None);
    assert_eq!(req.tools[1].cache_control, marker(Some("1h")));
}

// Defeat: flattening system blocks to a string and losing which block is marked.
#[test]
fn marked_system_blocks_are_kept_next_to_the_joined_text() {
    let body = body_with(json!({"system": [
        {"type": "text", "text": "first"},
        {"type": "text", "text": "second", "cache_control": {"type": "ephemeral", "ttl": "1h"}}
    ]}));
    let req = conversation(&body).unwrap();
    assert_eq!(req.system.as_deref(), Some("first\n\nsecond"));
    assert_eq!(
        req.system_blocks,
        vec![
            SystemBlock {
                text: "first".into(),
                cache_control: None
            },
            SystemBlock {
                text: "second".into(),
                cache_control: marker(Some("1h"))
            },
        ]
    );
    assert!(req.system_for_wire().is_some());
}

// Defeat: keeping structure nobody asked for, which would change every
// non-cache consumer of system blocks.
#[test]
fn unmarked_or_plain_string_system_keeps_no_blocks() {
    let plain = conversation(&body_with(json!({"system": "be brief"}))).unwrap();
    assert_eq!(plain.system.as_deref(), Some("be brief"));
    assert_eq!(plain.system_blocks, []);

    let unmarked = conversation(&body_with(json!({"system": [
        {"type": "text", "text": "a"}, {"type": "text", "text": "b"}
    ]})))
    .unwrap();
    assert_eq!(unmarked.system.as_deref(), Some("a\n\nb"));
    assert_eq!(unmarked.system_blocks, []);
}

// Defeat: treating any `cache_control` object as a breakpoint although only
// `ephemeral` is defined.
#[test]
fn a_marker_of_an_unknown_type_is_not_a_breakpoint() {
    let body = body_with(json!({"messages": [{"role": "user", "content": [
        {"type": "text", "text": "a", "cache_control": {"type": "persistent"}}
    ]}]}));
    let req = conversation(&body).unwrap();
    match only_block(&req) {
        ContentBlock::Text { cache_control, .. } => assert_eq!(*cache_control, None),
        other => panic!("expected Text, got {other:?}"),
    }
}
