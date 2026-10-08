//! Expected wire JSON below is written by hand from the Anthropic Messages API
//! reference (messages, tool use, `tool_choice`, extended thinking, images).

use serde_json::{json, Value};
use vkdg_operations::{
    CacheControl, ContentBlock, ConversationRequest, ImageData, Message, MessageContent, Role,
    SystemBlock, ThinkingRequest, Tool, ToolChoice,
};

use super::{body_map, messages_body, DEFAULT_MAX_TOKENS};

fn text(role: Role, t: &str) -> Message {
    Message {
        role,
        content: MessageContent::Text(t.into()),
    }
}

fn blocks(role: Role, b: Vec<ContentBlock>) -> Message {
    Message {
        role,
        content: MessageContent::Blocks(b),
    }
}

fn call(id: &str, name: &str, input: Value) -> ContentBlock {
    ContentBlock::ToolUse {
        id: id.into(),
        name: name.into(),
        input,
        cache_control: None,
    }
}

fn result(id: &str, content: &str) -> ContentBlock {
    ContentBlock::ToolResult {
        tool_use_id: id.into(),
        content: content.into(),
        images: vec![],
        is_error: false,
        cache_control: None,
    }
}

fn tool(name: &str) -> Tool {
    Tool {
        name: name.into(),
        description: Some(format!("{name} tool")),
        input_schema: json!({"type": "object", "properties": {}}),
        cache_control: None,
    }
}

fn request(messages: Vec<Message>) -> ConversationRequest {
    ConversationRequest {
        model: "claude-sonnet-4-5".into(),
        messages,
        max_tokens: Some(1000),
        ..Default::default()
    }
}

fn with_tools(mut req: ConversationRequest) -> ConversationRequest {
    req.tools = vec![tool("get_weather"), tool("get_time")];
    req
}

/// The body before breakpoints are settled, so the shape tests see exactly what
/// the request says; the breakpoint rules have their own tests.
fn wire(req: &ConversationRequest) -> Value {
    Value::Object(body_map(req, "claude-sonnet-4-5", None))
}

fn thinking(budget: Option<u32>) -> ThinkingRequest {
    ThinkingRequest {
        budget_tokens: budget,
        effort: None,
    }
}

// ── tool_choice ───────────────────────────────────────────────────────────────

// Defeat: dropping tool_choice (the model ignores a forced tool), or sending the
// OpenAI spellings (`required`, `{type:function}`), which Anthropic rejects.
#[test]
fn tool_choice_uses_the_anthropic_spellings() {
    let cases = [
        (ToolChoice::Auto, json!({"type": "auto"})),
        (ToolChoice::Required, json!({"type": "any"})),
        (ToolChoice::Disabled, json!({"type": "none"})),
        (
            ToolChoice::Named("get_weather".into()),
            json!({"type": "tool", "name": "get_weather"}),
        ),
    ];
    for (choice, expected) in cases {
        let mut req = with_tools(request(vec![text(Role::User, "hi")]));
        req.tool_choice = Some(choice.clone());
        assert_eq!(wire(&req)["tool_choice"], expected, "{choice:?}");
    }
}

// Defeat: losing the client's one-call-per-turn limit, or attaching it to
// `none`, whose schema has no such field.
#[test]
fn disable_parallel_tool_use_rides_inside_tool_choice() {
    let cases = [
        (
            None,
            json!({"type": "auto", "disable_parallel_tool_use": true}),
        ),
        (
            Some(ToolChoice::Auto),
            json!({"type": "auto", "disable_parallel_tool_use": true}),
        ),
        (
            Some(ToolChoice::Required),
            json!({"type": "any", "disable_parallel_tool_use": true}),
        ),
        (
            Some(ToolChoice::Named("get_time".into())),
            json!({"type": "tool", "name": "get_time", "disable_parallel_tool_use": true}),
        ),
        (Some(ToolChoice::Disabled), json!({"type": "none"})),
    ];
    for (choice, expected) in cases {
        let mut req = with_tools(request(vec![text(Role::User, "hi")]));
        req.tool_choice = choice.clone();
        req.disable_parallel_tool_use = true;
        assert_eq!(wire(&req)["tool_choice"], expected, "{choice:?}");
    }
}

// Defeat: sending tool_choice with no tools (Anthropic: 400), or inventing a
// tool_choice when the client expressed no preference.
#[test]
fn no_tool_choice_without_tools_or_without_a_preference() {
    let mut req = request(vec![text(Role::User, "hi")]);
    req.tool_choice = Some(ToolChoice::Auto);
    assert!(wire(&req).get("tool_choice").is_none());
    let req = with_tools(request(vec![text(Role::User, "hi")]));
    assert!(wire(&req).get("tool_choice").is_none());
}

// ── stop_sequences, top_p, max_tokens ─────────────────────────────────────────

// Defeat: omitting stop_sequences so the model runs past the client's stop.
#[test]
fn stop_sequences_reach_the_wire_only_when_set() {
    let mut req = request(vec![text(Role::User, "hi")]);
    assert!(wire(&req).get("stop_sequences").is_none());
    req.stop_sequences = vec!["\n\nHuman:".into(), "END".into()];
    assert_eq!(wire(&req)["stop_sequences"], json!(["\n\nHuman:", "END"]));
}

// Defeat: Claude 4.5+ answers 400 "temperature and top_p cannot both be
// specified" to the pair OpenAI clients commonly send; temperature wins.
#[test]
fn top_p_is_sent_alone_and_yields_to_temperature() {
    let mut req = request(vec![text(Role::User, "hi")]);
    req.top_p = Some(0.9);
    let v = wire(&req);
    assert_eq!(v["top_p"], json!(0.9));
    assert!(v.get("temperature").is_none());
    req.temperature = Some(0.7);
    let v = wire(&req);
    assert_eq!(v["temperature"], json!(0.7));
    assert!(v.get("top_p").is_none());
}

// Defeat: omitting max_tokens (Anthropic requires it: 400) or overriding the
// client's own value with the default.
#[test]
fn max_tokens_defaults_only_when_the_client_sent_none() {
    let mut req = request(vec![text(Role::User, "hi")]);
    req.max_tokens = None;
    assert_eq!(wire(&req)["max_tokens"], json!(DEFAULT_MAX_TOKENS));
    assert_eq!(DEFAULT_MAX_TOKENS, 8192);
    req.max_tokens = Some(77);
    assert_eq!(wire(&req)["max_tokens"], json!(77));
}

// ── extended thinking constraints ─────────────────────────────────────────────

// Defeat: sending temperature 0.7 or top_p 0.9 with thinking on (Anthropic: 400),
// or dropping a value the API allows (temperature 1, top_p >= 0.95).
// 0.95 is f32 0.95; widened to f64 naively it is 0.949999988... < 0.95.
#[test]
fn thinking_keeps_only_the_sampling_values_the_api_allows() {
    let cases = [
        (Some(0.7_f32), None, None, None),
        (Some(1.0), None, Some(json!(1.0)), None),
        (None, Some(0.9), None, None),
        (None, Some(0.95), None, Some(json!(0.95))),
        (None, Some(1.0), None, Some(json!(1.0))),
    ];
    for (temperature, top_p, want_t, want_p) in cases {
        let mut req = request(vec![text(Role::User, "hi")]);
        req.thinking = Some(thinking(Some(2000)));
        req.max_tokens = Some(4000);
        req.temperature = temperature;
        req.top_p = top_p;
        let v = wire(&req);
        assert_eq!(
            v.get("temperature").cloned(),
            want_t,
            "{temperature:?}/{top_p:?}"
        );
        assert_eq!(v.get("top_p").cloned(), want_p, "{temperature:?}/{top_p:?}");
        assert_eq!(
            v["thinking"],
            json!({"type": "enabled", "budget_tokens": 2000})
        );
    }
}

// Defeat: budget_tokens >= max_tokens (Anthropic: 400), or shrinking the
// client's output allowance to fit the budget instead of adding the budget.
#[test]
fn thinking_budget_is_added_on_top_of_a_max_tokens_that_would_not_fit_it() {
    let cases = [
        // (client max_tokens, budget, expected max_tokens, expected budget)
        (Some(4000), Some(2000), 4000, 2000),
        (Some(2000), Some(2000), 4000, 2000),
        (Some(1000), Some(2000), 3000, 2000),
        (None, Some(10_000), DEFAULT_MAX_TOKENS + 10_000, 10_000),
        (Some(500), Some(100), 500 + 1024, 1024),
    ];
    for (max, budget, want_max, want_budget) in cases {
        let mut req = request(vec![text(Role::User, "hi")]);
        req.max_tokens = max;
        req.thinking = Some(thinking(budget));
        let v = wire(&req);
        assert_eq!(
            v["max_tokens"],
            json!(want_max),
            "max {max:?} budget {budget:?}"
        );
        assert_eq!(v["thinking"]["budget_tokens"], json!(want_budget));
    }
}

// Defeat: sending thinking together with a forced tool (Anthropic: 400 "thinking
// may not be enabled when tool_choice forces tool use"); or dropping thinking
// for the choices that allow it.
#[test]
fn forced_tool_choice_drops_thinking_and_keeps_sampling() {
    for (choice, keeps_thinking) in [
        (ToolChoice::Required, false),
        (ToolChoice::Named("get_time".into()), false),
        (ToolChoice::Auto, true),
        (ToolChoice::Disabled, true),
    ] {
        let mut req = with_tools(request(vec![text(Role::User, "hi")]));
        req.tool_choice = Some(choice.clone());
        req.thinking = Some(thinking(Some(2000)));
        req.temperature = Some(0.7);
        let v = wire(&req);
        assert_eq!(v.get("thinking").is_some(), keeps_thinking, "{choice:?}");
        // With thinking gone the client's temperature is valid and must survive.
        assert_eq!(
            v.get("temperature").is_some(),
            !keeps_thinking,
            "{choice:?}"
        );
        if !keeps_thinking {
            assert_eq!(v["max_tokens"], json!(1000), "{choice:?}");
        }
    }
}

fn weather_loop(first_assistant_block: Option<ContentBlock>) -> Vec<Message> {
    let mut assistant: Vec<ContentBlock> = first_assistant_block.into_iter().collect();
    assistant.push(call("toolu_1", "get_weather", json!({"city": "Paris"})));
    vec![
        text(Role::User, "weather?"),
        blocks(Role::Assistant, assistant),
        blocks(Role::Tool, vec![result("toolu_1", "sunny")]),
    ]
}

// Defeat: continuing a tool turn with thinking on when the assistant turn has no
// signed thinking block (Anthropic: 400 "Expected thinking or redacted_thinking,
// but found tool_use"); every OpenAI-origin history looks like this. A signed
// block, or a request that is not mid tool turn, keeps thinking.
#[test]
fn thinking_is_dropped_only_for_a_tool_turn_continuation_without_a_signed_block() {
    let signed = ContentBlock::Thinking {
        thinking: "hmm".into(),
        signature: Some("sig".into()),
    };
    let unsigned = ContentBlock::Thinking {
        thinking: "hmm".into(),
        signature: None,
    };
    let redacted = ContentBlock::RedactedThinking { data: "abc".into() };
    for (first, expect_thinking) in [
        (None, false),
        (Some(unsigned), false),
        (Some(signed), true),
        (Some(redacted), true),
    ] {
        let mut req = with_tools(request(weather_loop(first.clone())));
        req.thinking = Some(thinking(Some(2000)));
        assert_eq!(
            wire(&req).get("thinking").is_some(),
            expect_thinking,
            "{first:?}"
        );
    }
    // Not a continuation: the last turn is a fresh user question.
    let mut msgs = weather_loop(None);
    msgs.push(text(Role::Assistant, "It is sunny."));
    msgs.push(text(Role::User, "and tomorrow?"));
    let mut req = with_tools(request(msgs));
    req.thinking = Some(thinking(Some(2000)));
    assert!(wire(&req).get("thinking").is_some());
}

// ── tool use / tool result adjacency ──────────────────────────────────────────

// Defeat: one user turn per `role:tool` message (consecutive user turns, with
// results split across turns), or results in arrival order instead of the order
// of the assistant's tool_use blocks.
#[test]
fn parallel_tool_results_become_one_user_turn_in_tool_use_order() {
    let req = request(vec![
        text(Role::User, "weather and time?"),
        blocks(
            Role::Assistant,
            vec![
                ContentBlock::Text {
                    text: "Checking.".into(),
                    cache_control: None,
                },
                call("toolu_a", "get_weather", json!({"city": "Paris"})),
                call("toolu_b", "get_time", json!({})),
            ],
        ),
        blocks(Role::Tool, vec![result("toolu_b", "noon")]),
        blocks(Role::Tool, vec![result("toolu_a", "sunny")]),
    ]);
    assert_eq!(
        wire(&req)["messages"],
        json!([
            {"role": "user", "content": "weather and time?"},
            {"role": "assistant", "content": [
                {"type": "text", "text": "Checking."},
                {"type": "tool_use", "id": "toolu_a", "name": "get_weather", "input": {"city": "Paris"}},
                {"type": "tool_use", "id": "toolu_b", "name": "get_time", "input": {}},
            ]},
            {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "toolu_a", "content": "sunny"},
                {"type": "tool_result", "tool_use_id": "toolu_b", "content": "noon"},
            ]},
        ])
    );
}

// Defeat: text before tool_result in the same user turn (Anthropic: tool_result
// blocks must come first), or a separate user turn for the follow-up text.
#[test]
fn user_text_after_tool_results_joins_the_same_turn_after_them() {
    let req = request(vec![
        text(Role::User, "go"),
        blocks(
            Role::Assistant,
            vec![call("toolu_1", "get_time", json!({}))],
        ),
        text(Role::User, "thanks, also be brief"),
        blocks(Role::Tool, vec![result("toolu_1", "noon")]),
    ]);
    assert_eq!(
        wire(&req)["messages"][2],
        json!({"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": "toolu_1", "content": "noon"},
            {"type": "text", "text": "thanks, also be brief"},
        ]})
    );
}

// Defeat: forwarding a tool_result whose id no preceding tool_use carries
// (Anthropic: 400 "unexpected tool_use_id"), or silently dropping its content.
#[test]
fn an_orphan_tool_result_survives_as_text_not_as_a_tool_result() {
    let req = request(vec![
        text(Role::User, "hi"),
        text(Role::Assistant, "hello"),
        blocks(Role::Tool, vec![result("toolu_gone", "42")]),
    ]);
    let v = wire(&req);
    let last = &v["messages"][2];
    assert_eq!(last["role"], "user");
    let rendered = last["content"].to_string();
    assert!(!rendered.contains("tool_result"), "{rendered}");
    assert!(
        rendered.contains("toolu_gone") && rendered.contains("42"),
        "{rendered}"
    );
}

// Defeat: a tool_use with no tool_result in the next user turn (Anthropic: 400
// "tool_use ids were found without tool_result blocks immediately after"), e.g.
// the client interrupted the call; the real result must still win when present.
#[test]
fn an_unanswered_tool_use_gets_an_error_result_and_answered_ones_keep_theirs() {
    let req = request(vec![
        text(Role::User, "go"),
        blocks(
            Role::Assistant,
            vec![
                call("toolu_a", "get_weather", json!({})),
                call("toolu_b", "get_time", json!({})),
            ],
        ),
        blocks(Role::Tool, vec![result("toolu_b", "noon")]),
    ]);
    assert_eq!(
        wire(&req)["messages"][2]["content"],
        json!([
            {"type": "tool_result", "tool_use_id": "toolu_a",
             "content": "[No response received]", "is_error": true},
            {"type": "tool_result", "tool_use_id": "toolu_b", "content": "noon"},
        ])
    );
}

// Defeat: the tool_result of an interrupted exchange being sent twice for one
// id (Anthropic rejects duplicate results), or `is_error` being lost.
#[test]
fn duplicate_results_are_not_repeated_and_is_error_is_kept() {
    let req = request(vec![
        text(Role::User, "go"),
        blocks(
            Role::Assistant,
            vec![call("toolu_1", "get_time", json!({}))],
        ),
        blocks(
            Role::Tool,
            vec![
                ContentBlock::ToolResult {
                    tool_use_id: "toolu_1".into(),
                    content: "boom".into(),
                    images: vec![],
                    is_error: true,
                    cache_control: None,
                },
                result("toolu_1", "late duplicate"),
            ],
        ),
    ]);
    let content = &wire(&req)["messages"][2]["content"];
    assert_eq!(content[0]["content"], "boom");
    assert_eq!(content[0]["is_error"], true);
    assert_eq!(
        content
            .as_array()
            .unwrap()
            .iter()
            .filter(|b| b["type"] == "tool_result")
            .count(),
        1
    );
}

// ── content blocks ────────────────────────────────────────────────────────────

// Defeat: serializing ContentBlock with serde's internal shape
// (`data: {"Base64": {...}}`, no `source`), which Anthropic rejects.
#[test]
fn images_use_the_anthropic_source_shape() {
    let req = request(vec![blocks(
        Role::User,
        vec![
            ContentBlock::Text {
                text: "what is this?".into(),
                cache_control: None,
            },
            ContentBlock::Image {
                media_type: "image/png".into(),
                data: ImageData::Base64 {
                    data: "iVBORw0KGgo=".into(),
                },
                cache_control: None,
            },
            ContentBlock::Image {
                media_type: String::new(),
                data: ImageData::Url {
                    url: "https://example.com/cat.jpg".into(),
                },
                cache_control: None,
            },
        ],
    )]);
    assert_eq!(
        wire(&req)["messages"][0]["content"],
        json!([
            {"type": "text", "text": "what is this?"},
            {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "iVBORw0KGgo="}},
            {"type": "image", "source": {"type": "url", "url": "https://example.com/cat.jpg"}},
        ])
    );
}

// Defeat: replaying a thinking block without a signature (from another
// provider; Anthropic: 400 invalid signature), or mangling a signed one.
#[test]
fn only_signed_thinking_is_replayed() {
    let req = request(vec![
        text(Role::User, "hi"),
        blocks(
            Role::Assistant,
            vec![
                ContentBlock::Thinking {
                    thinking: "from elsewhere".into(),
                    signature: None,
                },
                ContentBlock::Thinking {
                    thinking: "ponder".into(),
                    signature: Some("EqQBCg".into()),
                },
                ContentBlock::RedactedThinking {
                    data: String::new(),
                },
                ContentBlock::RedactedThinking {
                    data: "opaque".into(),
                },
                ContentBlock::Text {
                    text: "done".into(),
                    cache_control: None,
                },
            ],
        ),
    ]);
    assert_eq!(
        wire(&req)["messages"][1]["content"],
        json!([
            {"type": "thinking", "thinking": "ponder", "signature": "EqQBCg"},
            {"type": "redacted_thinking", "data": "opaque"},
            {"type": "text", "text": "done"},
        ])
    );
}

// Defeat: `role: "system"` inside messages (Anthropic: 400) or losing that text.
// Empty text blocks are rejected by Anthropic too.
#[test]
fn system_messages_fold_into_system_and_empty_text_is_dropped() {
    let mut req = request(vec![
        text(Role::System, "mid-conversation rule"),
        text(Role::User, "hi"),
        blocks(
            Role::Assistant,
            vec![
                ContentBlock::Text {
                    text: String::new(),
                    cache_control: None,
                },
                ContentBlock::Text {
                    text: "yo".into(),
                    cache_control: None,
                },
            ],
        ),
    ]);
    req.system = Some("base".into());
    let v = wire(&req);
    assert_eq!(v["system"], json!("base\n\nmid-conversation rule"));
    assert_eq!(
        v["messages"],
        json!([
            {"role": "user", "content": "hi"},
            {"role": "assistant", "content": "yo"},
        ])
    );
}

// Defeat: the preamble replacing or following the client's system prompt (the
// subscription gate needs it first, the client's text untouched after it).
#[test]
fn a_system_preamble_comes_first_as_its_own_block() {
    let mut req = request(vec![text(Role::User, "hi")]);
    req.system = Some("Be brief.".into());
    let v = Value::Object(body_map(&req, "claude-sonnet-4-5", Some("IDENTITY")));
    assert_eq!(
        v["system"],
        json!([
            {"type": "text", "text": "IDENTITY"},
            {"type": "text", "text": "Be brief."},
        ])
    );
    req.system = None;
    let v = Value::Object(body_map(&req, "claude-sonnet-4-5", Some("IDENTITY")));
    assert_eq!(v["system"], json!([{"type": "text", "text": "IDENTITY"}]));
}

// ── prompt-cache breakpoints ─────────────────────────────────────────────────

#[allow(clippy::unnecessary_wraps)] // fills the `Option<CacheControl>` field of every block literal
fn marker(ttl: Option<&str>) -> Option<CacheControl> {
    Some(CacheControl {
        ttl: ttl.map(str::to_owned),
    })
}

fn sent(req: &ConversationRequest, preamble: Option<&str>) -> Value {
    serde_json::from_slice(&messages_body(req, "claude-sonnet-4-5", preamble)).unwrap()
}

// Defeat: markers dropped on the way upstream, `ttl` rewritten, or a marker
// moved off its block.
#[test]
fn client_markers_reach_the_body_on_system_messages_and_tools_with_ttl_verbatim() {
    let mut req = request(vec![
        blocks(
            Role::User,
            vec![
                ContentBlock::Text {
                    text: "context".into(),
                    cache_control: marker(Some("1h")),
                },
                ContentBlock::Text {
                    text: "question".into(),
                    cache_control: None,
                },
            ],
        ),
        blocks(
            Role::Assistant,
            vec![ContentBlock::ToolUse {
                id: "c1".into(),
                name: "get_weather".into(),
                input: json!({}),
                cache_control: marker(None),
            }],
        ),
        blocks(
            Role::User,
            vec![ContentBlock::ToolResult {
                tool_use_id: "c1".into(),
                content: "sunny".into(),
                images: vec![],
                is_error: false,
                cache_control: marker(Some("5m")),
            }],
        ),
    ]);
    req.tools = vec![
        tool("get_weather"),
        Tool {
            cache_control: marker(Some("1h")),
            ..tool("get_time")
        },
    ];
    req.system = Some("rules\n\nproject".into());
    req.system_blocks = vec![
        SystemBlock {
            text: "rules".into(),
            cache_control: None,
        },
        SystemBlock {
            text: "project".into(),
            cache_control: marker(Some("1h")),
        },
    ];
    // Before the 4-breakpoint cap, which has its own tests.
    let v = wire(&req);
    let eph = json!({"type": "ephemeral"});
    let eph_1h = json!({"type": "ephemeral", "ttl": "1h"});
    assert_eq!(v["tools"][0].get("cache_control"), None);
    assert_eq!(v["tools"][1]["cache_control"], eph_1h);
    assert_eq!(v["system"][0].get("cache_control"), None);
    assert_eq!(v["system"][1]["cache_control"], eph_1h);
    assert_eq!(v["messages"][0]["content"][0]["cache_control"], eph_1h);
    assert_eq!(v["messages"][0]["content"][1].get("cache_control"), None);
    assert_eq!(v["messages"][1]["content"][0]["cache_control"], eph);
    assert_eq!(
        v["messages"][2]["content"][0]["cache_control"],
        json!({"type": "ephemeral", "ttl": "5m"})
    );
    assert_eq!(v.to_string().matches("cache_control").count(), 5);
}

// Defeat: a lone marked text collapsing to a plain string and losing its marker.
#[test]
fn a_marked_lone_text_block_keeps_the_block_form() {
    let req = request(vec![blocks(
        Role::User,
        vec![ContentBlock::Text {
            text: "hi".into(),
            cache_control: marker(Some("1h")),
        }],
    )]);
    assert_eq!(
        wire(&req)["messages"][0]["content"],
        json!([{"type": "text", "text": "hi", "cache_control": {"type": "ephemeral", "ttl": "1h"}}])
    );
}

// Defeat: a marker on a text block lost when `arrange` merges same-role turns.
#[test]
fn markers_survive_merging_of_consecutive_user_messages() {
    let req = request(vec![
        blocks(
            Role::User,
            vec![ContentBlock::Text {
                text: "a".into(),
                cache_control: marker(None),
            }],
        ),
        text(Role::User, "b"),
    ]);
    let v = wire(&req);
    assert_eq!(v["messages"].as_array().unwrap().len(), 1);
    assert_eq!(
        v["messages"][0]["content"][0]["cache_control"],
        json!({"type": "ephemeral"})
    );
}

// Defeat: an edited system prompt shipped as the client's old blocks.
#[test]
fn stale_system_blocks_are_not_sent_after_the_system_text_was_edited() {
    let mut req = request(vec![text(Role::User, "hi")]);
    req.system = Some("old".into());
    req.system_blocks = vec![SystemBlock {
        text: "old".into(),
        cache_control: marker(Some("1h")),
    }];
    req.system = Some("injected\n\nold".into());
    let v = wire(&req);
    assert_eq!(v["system"], json!("injected\n\nold"));
}

// Defeat: the identity block marked, moved, or the client's marker lost behind it.
#[test]
fn with_a_preamble_the_identity_block_stays_first_and_unmarked() {
    let mut req = request(vec![text(Role::User, "hi")]);
    req.system = Some("Be brief.".into());
    req.system_blocks = vec![SystemBlock {
        text: "Be brief.".into(),
        cache_control: marker(Some("1h")),
    }];
    let v = sent(&req, Some("IDENTITY"));
    assert_eq!(v["system"][0], json!({"type": "text", "text": "IDENTITY"}));
    assert_eq!(
        v["system"][1],
        json!({"type": "text", "text": "Be brief.", "cache_control": {"type": "ephemeral", "ttl": "1h"}})
    );
}

// Defeat: auto breakpoints added over the client's own placement.
#[test]
fn a_request_without_markers_gets_default_breakpoints_and_one_with_markers_gets_none_extra() {
    let mut req = with_tools(request(vec![text(Role::User, "hi")]));
    req.system = Some("sys".into());
    let v = sent(&req, None);
    assert_eq!(v.to_string().matches("cache_control").count(), 3);
    assert!(v["tools"][1]["cache_control"].is_object());
    assert!(v["tools"][0].get("cache_control").is_none());

    req.tools[0].cache_control = marker(None);
    let v = sent(&req, None);
    assert_eq!(v.to_string().matches("cache_control").count(), 1);
}

// ── images inside tool results ────────────────────────────────────────────────

fn result_with_image(id: &str, content: &str, media_type: &str, data: ImageData) -> ContentBlock {
    ContentBlock::ToolResult {
        tool_use_id: id.into(),
        content: content.into(),
        images: vec![vkdg_operations::ToolResultImage {
            media_type: media_type.into(),
            data,
        }],
        is_error: false,
        cache_control: None,
    }
}

// Defeat: writing the result as a bare string, which silently drops the screenshot a
// tool returned. Anthropic takes an array of text and image blocks as the content.
#[test]
fn tool_result_images_stay_inside_the_result_in_anthropic_shape() {
    let req = request(vec![
        text(Role::User, "look"),
        blocks(
            Role::Assistant,
            vec![call("toolu_1", "screenshot", json!({}))],
        ),
        blocks(
            Role::User,
            vec![ContentBlock::ToolResult {
                tool_use_id: "toolu_1".into(),
                content: "page loaded".into(),
                images: vec![
                    vkdg_operations::ToolResultImage {
                        media_type: "image/png".into(),
                        data: ImageData::Base64 {
                            data: "AAAA".into(),
                        },
                    },
                    vkdg_operations::ToolResultImage {
                        media_type: "image/png".into(),
                        data: ImageData::Url {
                            url: "https://x.test/a.png".into(),
                        },
                    },
                ],
                is_error: false,
                cache_control: None,
            }],
        ),
    ]);
    let body = wire(&req);
    let content = &body["messages"][2]["content"][0];
    assert_eq!(content["type"], "tool_result");
    assert_eq!(content["tool_use_id"], "toolu_1");
    assert_eq!(
        content["content"],
        json!([
            {"type": "text", "text": "page loaded"},
            {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "AAAA"}},
            {"type": "image", "source": {"type": "url", "url": "https://x.test/a.png"}},
        ])
    );
}

// Defeat: an empty text block in front of an image-only result, which Anthropic rejects
// ("text content blocks must be non-empty").
#[test]
fn image_only_tool_result_has_no_empty_text_block() {
    let req = request(vec![
        text(Role::User, "look"),
        blocks(
            Role::Assistant,
            vec![call("toolu_1", "screenshot", json!({}))],
        ),
        blocks(
            Role::User,
            vec![result_with_image(
                "toolu_1",
                "",
                "image/jpeg",
                ImageData::Base64 {
                    data: "BBBB".into(),
                },
            )],
        ),
    ]);
    let body = wire(&req);
    assert_eq!(
        body["messages"][2]["content"][0]["content"],
        json!([{"type": "image", "source": {"type": "base64", "media_type": "image/jpeg", "data": "BBBB"}}])
    );
}

// Defeat: turning every result into an array; plain text results keep the string form
// Anthropic's own SDKs write.
#[test]
fn text_only_tool_result_stays_a_string() {
    let req = request(vec![
        text(Role::User, "hi"),
        blocks(
            Role::Assistant,
            vec![call("toolu_1", "get_time", json!({}))],
        ),
        blocks(Role::User, vec![result("toolu_1", "noon")]),
    ]);
    assert_eq!(wire(&req)["messages"][2]["content"][0]["content"], "noon");
}

// ── server tools ──────────────────────────────────────────────────────────────

fn web_search() -> vkdg_operations::ServerTool {
    vkdg_operations::ServerTool {
        name: "web_search".into(),
        declaration: json!({"type": "web_search_20250305", "name": "web_search", "max_uses": 3,
                            "allowed_domains": ["rust-lang.org"]}),
    }
}

// Defeat: dropping the declaration (the model never searches), or rewriting it into a
// custom tool with an `input_schema`, which Anthropic rejects for a server tool. It is
// forwarded exactly as the client wrote it, after the custom tools.
#[test]
fn server_tools_are_forwarded_verbatim_after_custom_tools() {
    let mut req = with_tools(request(vec![text(Role::User, "search")]));
    req.server_tools = vec![web_search()];
    let body = wire(&req);
    let tools = body["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 3);
    assert_eq!(tools[2], web_search().declaration);
}

// Defeat: gating `tools` on the custom list only, so a request that declares just a
// server tool is sent without it; and a forced server tool losing its tool_choice.
#[test]
fn a_request_with_only_a_server_tool_still_sends_it_and_its_tool_choice() {
    let mut req = request(vec![text(Role::User, "search")]);
    req.server_tools = vec![web_search()];
    req.tool_choice = Some(ToolChoice::Named("web_search".into()));
    let body = wire(&req);
    assert_eq!(body["tools"], json!([web_search().declaration]));
    assert_eq!(
        body["tool_choice"],
        json!({"type": "tool", "name": "web_search"})
    );
}
