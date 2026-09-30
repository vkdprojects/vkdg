//! Tool translation for Kiro: what the model can call, and what it already called.
//!
//! Found against a live account: a request with tools got a plain-text "I don't
//! have access to a..." answer and `stop_reason: end_turn`, because `prepare()`
//! never sent the tools at all.
//!
//! The wire shape is taken from AWS's Smithy model
//! (`amzn-codewhisperer-streaming-client`: `UserInputMessageContext.tools`,
//! `ToolSpecification`, `ToolResult`, `AssistantResponseMessage.toolUses`) and
//! agrees with `OmniRoute`, Kiro-Go and jwadow/kiro-gateway. The rejection rules
//! (description length, name length, schema keywords, orphan results) come from
//! those gateways' production fixes, each of which records the upstream 400.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{json, Value};
use vkdg_connections::{AuthKind, ConnectionConfig, Credential, ProviderKind};
use vkdg_core::ConnectionId;
use vkdg_operations::{
    CapabilitySet, ContentBlock, ConversationRequest, Message, MessageContent, Operation, Role,
    Tool,
};
use vkdg_provider_kiro::KiroAdapter;
use vkdg_provider_sdk::ProviderAdapter;

fn connection() -> ConnectionConfig {
    ConnectionConfig {
        id: ConnectionId("kiro-1".into()),
        provider: ProviderKind::Plugin { id: "kiro".into() },
        auth: AuthKind::ApiKey {
            env_var: "UNUSED".into(),
        },
        models: vec!["claude-*".into()],
        max_concurrent: 4,
        weight: 1,
        tags: vec![],
        endpoint: None,
        capabilities: CapabilitySet::default(),
    }
}

fn credential() -> Credential {
    let extra: HashMap<String, String> =
        HashMap::from([("auth_method".to_owned(), "social".to_owned())]);
    Credential {
        token: "tok".into(),
        extra: Arc::new(extra),
    }
}

fn weather_tool() -> Tool {
    Tool {
        name: "get_weather".into(),
        description: Some("Get the current weather for a city".into()),
        input_schema: json!({
            "type": "object",
            "properties": { "city": { "type": "string" } },
            "required": ["city"]
        }),
    }
}

fn text(role: Role, s: &str) -> Message {
    Message {
        role,
        content: MessageContent::Text(s.into()),
    }
}

fn request(messages: Vec<Message>, tools: Vec<Tool>) -> Operation {
    Operation::Conversation(ConversationRequest {
        model: "claude-sonnet-4.5".into(),
        messages,
        tools,
        max_tokens: Some(256),
        temperature: None,
        stream: true,
        system: None,
        required_capabilities: CapabilitySet::default(),
        thinking: None,
        ..Default::default()
    })
}

fn body(op: &Operation) -> Value {
    let prepared = KiroAdapter
        .prepare(op, &connection(), &credential())
        .unwrap_or_else(|e| panic!("prepare must succeed: {e}"));
    serde_json::from_slice(&prepared.body).expect("body is JSON")
}

fn current(b: &Value) -> &Value {
    &b["conversationState"]["currentMessage"]["userInputMessage"]
}

fn current_tools(b: &Value) -> &Vec<Value> {
    current(b)["userInputMessageContext"]["tools"]
        .as_array()
        .unwrap_or_else(|| panic!("current message must carry tools: {b}"))
}

/// Refutes: dropping `ConversationRequest.tools`, the live defect. Without them the
/// model cannot call anything and answers in prose.
#[test]
fn tools_reach_the_current_message_as_tool_specifications() {
    let b = body(&request(
        vec![text(Role::User, "Weather in Paris?")],
        vec![weather_tool()],
    ));
    let tools = current_tools(&b);
    assert_eq!(tools.len(), 1);
    let spec = &tools[0]["toolSpecification"];
    assert_eq!(spec["name"], "get_weather");
    assert_eq!(spec["description"], "Get the current weather for a city");
    assert_eq!(
        spec["inputSchema"]["json"]["properties"]["city"]["type"],
        "string"
    );
    assert_eq!(spec["inputSchema"]["json"]["required"], json!(["city"]));
}

/// Refutes: forwarding JSON-Schema keywords Kiro rejects with 400 "Improperly
/// formed request", including ones nested below the top level.
#[test]
fn schema_keywords_kiro_rejects_are_stripped_at_every_depth() {
    let tool = Tool {
        name: "search".into(),
        description: Some("Search".into()),
        input_schema: json!({
            "$schema": "http://json-schema.org/draft-07/schema#",
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "q": { "type": "string" },
                "filter": {
                    "type": "object",
                    "additionalProperties": false,
                    "anyOf": [{ "required": ["a"] }, { "required": ["b"] }],
                    "properties": { "a": { "$ref": "#/$defs/x" } },
                    "required": []
                }
            },
            "$defs": { "x": { "type": "string" } }
        }),
    };
    let b = body(&request(vec![text(Role::User, "go")], vec![tool]));
    let schema = current_tools(&b)[0]["toolSpecification"]["inputSchema"]["json"].clone();
    let rendered = schema.to_string();
    for banned in ["additionalProperties", "anyOf", "$ref", "$defs", "$schema"] {
        assert!(
            !rendered.contains(banned),
            "`{banned}` must not reach Kiro: {rendered}"
        );
    }
    // Nested empty `required` is removed; the top level keeps a `required` key,
    // which Kiro expects.
    assert!(schema["properties"]["filter"].get("required").is_none());
    assert!(schema.get("required").is_some(), "top level keeps required");
    // Legitimate structure survives.
    assert_eq!(schema["properties"]["q"]["type"], "string");
}

/// Refutes: sending an empty description (rejected) or truncating a long one
/// (the model then loses the documentation it needs).
#[test]
fn descriptions_are_never_empty_and_long_ones_are_relocated_not_cut() {
    let long_doc = "x".repeat(10_500);
    let tools = vec![
        Tool {
            name: "bare".into(),
            description: None,
            input_schema: json!({ "type": "object", "properties": {} }),
        },
        Tool {
            name: "verbose".into(),
            description: Some(long_doc.clone()),
            input_schema: json!({ "type": "object", "properties": {} }),
        },
    ];
    let b = body(&request(vec![text(Role::User, "hi")], tools));
    let specs = current_tools(&b);
    assert_eq!(specs[0]["toolSpecification"]["description"], "Tool: bare");

    let short = specs[1]["toolSpecification"]["description"]
        .as_str()
        .expect("description string");
    assert!(
        short.len() <= 10_000,
        "Kiro rejects descriptions over 10000 chars; sent {}",
        short.len()
    );
    // The full text is not lost: it travels in the user content instead.
    let content = current(&b)["content"].as_str().expect("content string");
    assert!(
        content.contains(&long_doc),
        "the full documentation must still reach the model"
    );
}

/// Refutes: forwarding a tool name over 64 characters, which Kiro rejects, or
/// shortening it in a way that collides with another tool.
#[test]
fn over_long_tool_names_are_shortened_uniquely() {
    let long_a = format!("mcp__server__{}", "a".repeat(80));
    let long_b = format!("mcp__server__{}", "a".repeat(81));
    let tools = [long_a, long_b]
        .into_iter()
        .map(|name| Tool {
            name,
            description: Some("d".into()),
            input_schema: json!({ "type": "object", "properties": {} }),
        })
        .collect();
    let b = body(&request(vec![text(Role::User, "hi")], tools));
    let names: Vec<&str> = current_tools(&b)
        .iter()
        .map(|t| t["toolSpecification"]["name"].as_str().expect("name"))
        .collect();
    for n in &names {
        assert!(n.len() <= 64, "name {n:?} exceeds Kiro's 64-char limit");
    }
    assert_ne!(names[0], names[1], "shortened names must stay distinct");
}

/// Refutes: flattening a tool round trip into prose. The assistant's call must
/// travel as `toolUses` with an object `input`, and the result as `toolResults`
/// on the following user turn.
#[test]
fn a_tool_round_trip_uses_tool_uses_and_tool_results() {
    let messages = vec![
        text(Role::User, "Weather in Paris?"),
        Message {
            role: Role::Assistant,
            content: MessageContent::Blocks(vec![ContentBlock::ToolUse {
                id: "tu_1".into(),
                name: "get_weather".into(),
                input: json!({ "city": "Paris" }),
            }]),
        },
        Message {
            role: Role::User,
            content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                tool_use_id: "tu_1".into(),
                content: "18C and sunny".into(),
                is_error: false,
            }]),
        },
    ];
    let b = body(&request(messages, vec![weather_tool()]));

    let history = b["conversationState"]["history"]
        .as_array()
        .expect("history array");
    let assistant = history
        .iter()
        .find_map(|h| h.get("assistantResponseMessage"))
        .expect("an assistant turn in history");
    let uses = assistant["toolUses"].as_array().expect("toolUses array");
    assert_eq!(uses[0]["toolUseId"], "tu_1");
    assert_eq!(uses[0]["name"], "get_weather");
    assert_eq!(
        uses[0]["input"],
        json!({ "city": "Paris" }),
        "input must be an object, not a JSON string"
    );

    let results = current(&b)["userInputMessageContext"]["toolResults"]
        .as_array()
        .expect("toolResults on the current turn");
    assert_eq!(results[0]["toolUseId"], "tu_1");
    assert_eq!(results[0]["status"], "success");
    assert_eq!(results[0]["content"][0]["text"], "18C and sunny");
}

/// Refutes: omitting tools from a turn that only carries results. Kiro needs the
/// schemas to validate earlier `toolUses`, and rejects `toolResults` without
/// `tools`.
#[test]
fn a_result_only_turn_still_carries_the_tools() {
    let messages = vec![
        text(Role::User, "Weather in Paris?"),
        Message {
            role: Role::Assistant,
            content: MessageContent::Blocks(vec![ContentBlock::ToolUse {
                id: "tu_1".into(),
                name: "get_weather".into(),
                input: json!({ "city": "Paris" }),
            }]),
        },
        Message {
            role: Role::Tool,
            content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                tool_use_id: "tu_1".into(),
                content: "18C".into(),
                is_error: false,
            }]),
        },
    ];
    let b = body(&request(messages, vec![weather_tool()]));
    assert_eq!(current_tools(&b).len(), 1);
}

/// Refutes: sending a result whose call is not in the transcript. Kiro rejects
/// `toolResults` without a preceding matching `toolUses`; clients such as Cline
/// and Cursor do send these after trimming history.
#[test]
fn orphan_tool_results_become_text_instead_of_a_rejected_request() {
    let messages = vec![Message {
        role: Role::User,
        content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
            tool_use_id: "tu_gone".into(),
            content: "stale output".into(),
            is_error: false,
        }]),
    }];
    let b = body(&request(messages, vec![weather_tool()]));
    let ctx = &current(&b)["userInputMessageContext"];
    assert!(
        ctx.get("toolResults")
            .and_then(Value::as_array)
            .map_or(true, Vec::is_empty),
        "an orphan result must not be sent as toolResults: {b}"
    );
    let content = current(&b)["content"].as_str().expect("content string");
    assert!(
        content.contains("stale output"),
        "the orphan's output must not be lost"
    );
}

/// Refutes: a malformed request when the client sent no tools at all — the
/// context must not appear with an empty `tools` array.
#[test]
fn no_tools_means_no_tool_context() {
    let b = body(&request(vec![text(Role::User, "hi")], vec![]));
    let tools = current(&b)
        .get("userInputMessageContext")
        .and_then(|c| c.get("tools"));
    assert!(
        tools.is_none(),
        "no tools key must be sent when the client declared none: {b}"
    );
}
