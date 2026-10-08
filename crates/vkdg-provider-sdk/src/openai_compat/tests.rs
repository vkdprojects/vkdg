//! Expected wire JSON below is written by hand from the `OpenAI` Chat Completions
//! API reference (messages, `tool_calls`, tool role, `tool_choice`, `stop`, `top_p`,
//! `parallel_tool_calls`, `max_completion_tokens`, `stream_options`, `image_url`).

use serde_json::{json, Value};
use vkdg_operations::{
    ContentBlock, ConversationRequest, ImageData, Message, MessageContent, Role, Tool, ToolChoice,
};

use super::{chat_completions_body, chat_completions_body_with, TokenLimitField};

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

fn request(messages: Vec<Message>) -> ConversationRequest {
    ConversationRequest {
        model: "gpt-4o".into(),
        messages,
        ..Default::default()
    }
}

fn with_tools(mut req: ConversationRequest) -> ConversationRequest {
    req.tools = ["get_weather", "get_time"]
        .iter()
        .map(|n| Tool {
            name: (*n).into(),
            description: Some(format!("{n} tool")),
            input_schema: json!({"type": "object", "properties": {}}),
            cache_control: None,
        })
        .collect();
    req
}

fn wire(req: &ConversationRequest) -> Value {
    serde_json::from_slice(&chat_completions_body(req, "gpt-4o")).unwrap()
}

// ── tool_choice, parallel_tool_calls ──────────────────────────────────────────

// Defeat: dropping tool_choice (a forced tool is ignored), or sending the
// Anthropic spellings (`any`, `{type:tool}`), which OpenAI rejects.
#[test]
fn tool_choice_uses_the_openai_spellings() {
    let cases = [
        (ToolChoice::Auto, json!("auto")),
        (ToolChoice::Required, json!("required")),
        (ToolChoice::Disabled, json!("none")),
        (
            ToolChoice::Named("get_weather".into()),
            json!({"type": "function", "function": {"name": "get_weather"}}),
        ),
    ];
    for (choice, expected) in cases {
        let mut req = with_tools(request(vec![text(Role::User, "hi")]));
        req.tool_choice = Some(choice.clone());
        assert_eq!(wire(&req)["tool_choice"], expected, "{choice:?}");
    }
}

// Defeat: sending tool_choice / parallel_tool_calls without tools (OpenAI: 400),
// or never sending `parallel_tool_calls: false` for a client that forbade
// parallel calls.
#[test]
fn tool_policy_fields_need_tools_and_parallel_is_only_ever_disabled() {
    let mut req = request(vec![text(Role::User, "hi")]);
    req.tool_choice = Some(ToolChoice::Auto);
    req.disable_parallel_tool_use = true;
    let v = wire(&req);
    assert!(v.get("tool_choice").is_none() && v.get("parallel_tool_calls").is_none());

    let mut req = with_tools(request(vec![text(Role::User, "hi")]));
    let v = wire(&req);
    assert!(v.get("tool_choice").is_none() && v.get("parallel_tool_calls").is_none());
    req.disable_parallel_tool_use = true;
    assert_eq!(wire(&req)["parallel_tool_calls"], json!(false));
}

// ── stop, top_p, temperature, token limit, stream ─────────────────────────────

// Defeat: dropping stop/top_p, or widening f32 to 0.8999999761581421.
#[test]
fn stop_top_p_and_temperature_reach_the_wire_as_the_client_wrote_them() {
    let mut req = request(vec![text(Role::User, "hi")]);
    let v = wire(&req);
    assert!(v.get("stop").is_none() && v.get("top_p").is_none() && v.get("temperature").is_none());
    req.stop_sequences = vec!["END".into(), "\n\n".into()];
    req.top_p = Some(0.9);
    req.temperature = Some(0.7);
    let v = wire(&req);
    assert_eq!(v["stop"], json!(["END", "\n\n"]));
    assert_eq!(v["top_p"], json!(0.9));
    assert_eq!(v["temperature"], json!(0.7));
}

// Defeat: sending both limit fields (OpenAI 400s), or the deprecated field to a
// reasoning model that only takes max_completion_tokens; compatible servers
// still expect max_tokens.
#[test]
fn exactly_one_token_limit_field_is_sent() {
    let mut req = request(vec![text(Role::User, "hi")]);
    req.max_tokens = Some(256);
    let compat: Value = serde_json::from_slice(&chat_completions_body(&req, "m")).unwrap();
    assert_eq!(compat["max_tokens"], 256);
    assert!(compat.get("max_completion_tokens").is_none());
    let first_party: Value = serde_json::from_slice(&chat_completions_body_with(
        &req,
        "m",
        TokenLimitField::MaxCompletionTokens,
    ))
    .unwrap();
    assert_eq!(first_party["max_completion_tokens"], 256);
    assert!(first_party.get("max_tokens").is_none());
    req.max_tokens = None;
    let v: Value = serde_json::from_slice(&chat_completions_body_with(
        &req,
        "m",
        TokenLimitField::MaxCompletionTokens,
    ))
    .unwrap();
    assert!(v.get("max_tokens").is_none() && v.get("max_completion_tokens").is_none());
}

// Defeat: losing usage on streams (billing/metering read the final chunk).
#[test]
fn streaming_asks_for_usage() {
    let mut req = request(vec![text(Role::User, "hi")]);
    req.stream = true;
    let v = wire(&req);
    assert_eq!(v["stream"], true);
    assert_eq!(v["stream_options"], json!({"include_usage": true}));
    req.stream = false;
    assert!(wire(&req).get("stream_options").is_none());
}

// ── tool calls and results ────────────────────────────────────────────────────

// Defeat: dropping the assistant's text next to its tool calls, or sending
// `content: ""` instead of null when there is none.
#[test]
fn assistant_text_rides_with_tool_calls() {
    let req = request(vec![
        text(Role::User, "go"),
        blocks(
            Role::Assistant,
            vec![
                ContentBlock::Text {
                    text: "Checking.".into(),
                    cache_control: None,
                },
                call("call_a", "get_weather", json!({"city": "Paris"})),
            ],
        ),
        blocks(Role::Tool, vec![result("call_a", "sunny")]),
        blocks(
            Role::Assistant,
            vec![call("call_b", "get_time", Value::Null)],
        ),
        blocks(Role::Tool, vec![result("call_b", "noon")]),
    ]);
    let v = wire(&req);
    assert_eq!(
        v["messages"][1],
        json!({"role": "assistant", "content": "Checking.", "tool_calls": [
            {"id": "call_a", "type": "function",
             "function": {"name": "get_weather", "arguments": "{\"city\":\"Paris\"}"}}
        ]})
    );
    assert_eq!(
        v["messages"][3],
        json!({"role": "assistant", "content": null, "tool_calls": [
            {"id": "call_b", "type": "function",
             "function": {"name": "get_time", "arguments": "{}"}}
        ]})
    );
}

// Defeat: the Anthropic client's tool results (tool_result blocks inside a USER
// message) being flattened away, so the model never sees any tool output and
// loops; or one result of several surviving. Also text after the results must
// follow them as its own user message, and results must follow their call.
#[test]
fn anthropic_style_tool_results_become_tool_messages_in_call_order() {
    let req = request(vec![
        text(Role::User, "go"),
        blocks(
            Role::Assistant,
            vec![
                call("toolu_a", "get_weather", json!({})),
                call("toolu_b", "get_time", json!({})),
            ],
        ),
        blocks(
            Role::User,
            vec![
                result("toolu_b", "noon"),
                result("toolu_a", "sunny"),
                ContentBlock::Text {
                    text: "now summarize".into(),
                    cache_control: None,
                },
            ],
        ),
    ]);
    let v = wire(&req);
    assert_eq!(
        v["messages"].as_array().unwrap()[2..],
        [
            json!({"role": "tool", "tool_call_id": "toolu_a", "content": "sunny"}),
            json!({"role": "tool", "tool_call_id": "toolu_b", "content": "noon"}),
            json!({"role": "user", "content": "now summarize"}),
        ]
    );
}

// Defeat: only the first ToolResult of a message being sent (the old
// extract_tool_result), losing the rest of a parallel call's output.
#[test]
fn several_results_in_one_message_each_get_a_tool_message() {
    let req = request(vec![
        text(Role::User, "go"),
        blocks(
            Role::Assistant,
            vec![
                call("c1", "get_weather", json!({})),
                call("c2", "get_time", json!({})),
            ],
        ),
        blocks(Role::Tool, vec![result("c1", "one"), result("c2", "two")]),
    ]);
    let v = wire(&req);
    assert_eq!(
        v["messages"].as_array().unwrap()[2..],
        [
            json!({"role": "tool", "tool_call_id": "c1", "content": "one"}),
            json!({"role": "tool", "tool_call_id": "c2", "content": "two"}),
        ]
    );
}

// Defeat: a `tool` message with no preceding tool_calls (OpenAI 400 "messages
// with role 'tool' must be a response to a preceding message with 'tool_calls'")
// and a tool_call with no tool message (OpenAI 400); content must not vanish.
#[test]
fn orphan_results_and_unanswered_calls_are_repaired() {
    let req = request(vec![
        text(Role::User, "go"),
        blocks(Role::Assistant, vec![call("c1", "get_weather", json!({}))]),
        blocks(Role::Tool, vec![result("ghost", "42")]),
    ]);
    let v = wire(&req);
    let messages = v["messages"].as_array().unwrap();
    assert_eq!(
        messages[2],
        json!({"role": "tool", "tool_call_id": "c1", "content": "[No response received]"})
    );
    assert_eq!(messages[3]["role"], "user");
    let rendered = messages[3]["content"].to_string();
    assert!(
        rendered.contains("ghost") && rendered.contains("42"),
        "{rendered}"
    );
    assert!(
        messages.iter().filter(|m| m["role"] == "tool").count() == 1,
        "{messages:?}"
    );
}

// ── content ───────────────────────────────────────────────────────────────────

// Defeat: dropping images (the old text-only filter) or sending the Anthropic
// block shape; OpenAI wants `image_url` with a data URL for inline bytes.
#[test]
fn images_become_image_url_parts() {
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
        wire(&req)["messages"][0],
        json!({"role": "user", "content": [
            {"type": "text", "text": "what is this?"},
            {"type": "image_url", "image_url": {"url": "data:image/png;base64,iVBORw0KGgo="}},
            {"type": "image_url", "image_url": {"url": "https://example.com/cat.jpg"}},
        ]})
    );
}

// Defeat: system text split over several system messages (some servers reject a
// system turn after the first message) or the system prompt lost.
#[test]
fn all_system_text_is_one_leading_system_message() {
    let mut req = request(vec![
        text(Role::User, "hi"),
        text(Role::System, "late rule"),
        text(Role::Assistant, "hello"),
    ]);
    req.system = Some("base".into());
    let v = wire(&req);
    assert_eq!(
        v["messages"],
        json!([
            {"role": "system", "content": "base\n\nlate rule"},
            {"role": "user", "content": "hi"},
            {"role": "assistant", "content": "hello"},
        ])
    );
}

// Defeat: replaying thinking as text/content to a provider that has no such
// field, or emitting an empty assistant message for a thinking-only turn.
#[test]
fn thinking_blocks_are_not_sent() {
    let req = request(vec![
        text(Role::User, "hi"),
        blocks(
            Role::Assistant,
            vec![
                ContentBlock::Thinking {
                    thinking: "secret".into(),
                    signature: Some("sig".into()),
                },
                ContentBlock::Text {
                    text: "yo".into(),
                    cache_control: None,
                },
            ],
        ),
        blocks(
            Role::Assistant,
            vec![ContentBlock::RedactedThinking { data: "x".into() }],
        ),
    ]);
    let v = wire(&req);
    assert_eq!(
        v["messages"],
        json!([
            {"role": "user", "content": "hi"},
            {"role": "assistant", "content": "yo"},
        ])
    );
}

// ── images inside tool results ────────────────────────────────────────────────

// Defeat: dropping the image (the model never sees the screenshot), or putting an
// `image_url` into the `tool` message, which Chat Completions rejects (tool content is
// text only). The image rides in the user message right behind the tool messages.
#[test]
fn tool_result_images_follow_the_tool_message_as_a_user_message() {
    let req = request(vec![
        text(Role::User, "look"),
        blocks(
            Role::Assistant,
            vec![call("call_1", "screenshot", json!({}))],
        ),
        blocks(
            Role::User,
            vec![ContentBlock::ToolResult {
                tool_use_id: "call_1".into(),
                content: "page loaded".into(),
                images: vec![vkdg_operations::ToolResultImage {
                    media_type: "image/png".into(),
                    data: ImageData::Base64 {
                        data: "AAAA".into(),
                    },
                }],
                is_error: false,
                cache_control: None,
            }],
        ),
    ]);
    let messages = wire(&req)["messages"].clone();
    assert_eq!(
        messages[2],
        json!({"role": "tool", "tool_call_id": "call_1", "content": "page loaded"})
    );
    assert_eq!(
        messages[3],
        json!({"role": "user", "content": [
            {"type": "text", "text": "[image returned by tool call call_1]"},
            {"type": "image_url", "image_url": {"url": "data:image/png;base64,AAAA"}},
        ]})
    );
    assert_eq!(messages.as_array().unwrap().len(), 4);
}

// Defeat: an empty tool message (some servers reject empty content) when the result
// was only an image.
#[test]
fn image_only_tool_result_gets_a_pointer_text_not_an_empty_message() {
    let req = request(vec![
        text(Role::User, "look"),
        blocks(
            Role::Assistant,
            vec![call("call_1", "screenshot", json!({}))],
        ),
        blocks(
            Role::User,
            vec![ContentBlock::ToolResult {
                tool_use_id: "call_1".into(),
                content: String::new(),
                images: vec![vkdg_operations::ToolResultImage {
                    media_type: "image/png".into(),
                    data: ImageData::Url {
                        url: "https://x.test/a.png".into(),
                    },
                }],
                is_error: false,
                cache_control: None,
            }],
        ),
    ]);
    let messages = wire(&req)["messages"].clone();
    assert_eq!(
        messages[2]["content"],
        "[the tool returned an image, shown in the next message]"
    );
    assert_eq!(
        messages[3]["content"][1],
        json!({"type": "image_url", "image_url": {"url": "https://x.test/a.png"}})
    );
}

// Defeat: sending an Anthropic server tool declaration to an OpenAI-compatible server,
// which answers 400 on a tool without a `function`. Only Anthropic upstreams get it.
#[test]
fn anthropic_server_tools_are_not_sent_to_openai_upstreams() {
    let mut req = request(vec![text(Role::User, "search")]);
    req.server_tools = vec![vkdg_operations::ServerTool {
        name: "web_search".into(),
        declaration: json!({"type": "web_search_20250305", "name": "web_search"}),
    }];
    req.tool_choice = Some(ToolChoice::Required);
    let body = wire(&req);
    assert!(body.get("tools").is_none(), "{body}");
    assert!(body.get("tool_choice").is_none(), "{body}");
}

// Defeat: leaking Anthropic's prompt-cache breakpoints into an OpenAI-format
// body, which OpenAI rejects as an unknown field on strict-schema providers.
#[test]
fn anthropic_cache_markers_never_reach_an_openai_format_body() {
    use vkdg_operations::{CacheControl, SystemBlock};
    let marker = Some(CacheControl {
        ttl: Some("1h".into()),
    });
    let mut req = with_tools(request(vec![blocks(
        Role::User,
        vec![ContentBlock::Text {
            text: "hi".into(),
            cache_control: marker.clone(),
        }],
    )]));
    req.tools[1].cache_control = marker.clone();
    req.system = Some("rules".into());
    req.system_blocks = vec![SystemBlock {
        text: "rules".into(),
        cache_control: marker,
    }];
    let body = serde_json::to_string(&wire(&req)).unwrap();
    assert!(!body.contains("cache_control"), "{body}");
    assert!(body.contains("rules") && body.contains("hi"), "{body}");
}
