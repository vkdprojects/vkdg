//! The Kiro request body: conversation state, tools, and tool round trips.
//!
//! Wire shape from AWS's Smithy model (`amzn-codewhisperer-streaming-client`):
//! tools live in `userInputMessage.userInputMessageContext.tools` as
//! `{ toolSpecification: { name, description, inputSchema: { json } } }`, results in
//! `userInputMessageContext.toolResults`, and an assistant turn's calls in
//! `assistantResponseMessage.toolUses` with an object `input`. `OmniRoute`, Kiro-Go
//! and jwadow/kiro-gateway agree.
//!
//! The rejection rules below each correspond to an upstream 400 those gateways
//! recorded in production: description length, name length, JSON-Schema keywords
//! Kiro does not understand, and results that answer no call in the transcript.

use std::collections::HashSet;
use std::fmt::Write as _;

use serde::Serialize;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use vkdg_operations::{ContentBlock, ConversationRequest, MessageContent, Role, Tool};

/// Kiro rejects a `toolSpecification.description` longer than this.
const TOOL_DESCRIPTION_MAX: usize = 10_000;

/// Kiro rejects tool names longer than this.
const TOOL_NAME_MAX: usize = 64;

/// JSON-Schema keywords Kiro answers with 400 "Improperly formed request",
/// wherever they appear in a tool schema.
const STRIPPED_SCHEMA_KEYS: &[&str] = &[
    "additionalProperties",
    "anyOf",
    "oneOf",
    "allOf",
    "not",
    "$schema",
    "$id",
    "$ref",
    "$defs",
    "definitions",
    "if",
    "then",
    "else",
    "unevaluatedProperties",
    "unevaluatedItems",
    "contentEncoding",
    "contentMediaType",
];

// ── Wire types ────────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct ConversationState {
    #[serde(rename = "chatTriggerType")]
    chat_trigger_type: &'static str,
    #[serde(rename = "agentTaskType")]
    agent_task_type: &'static str,
    #[serde(rename = "conversationId")]
    conversation_id: String,
    #[serde(rename = "currentMessage")]
    current_message: CurrentMessage,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    history: Vec<HistoryItem>,
}

#[derive(Serialize)]
struct CurrentMessage {
    #[serde(rename = "userInputMessage")]
    user_input_message: UserInput,
}

#[derive(Serialize)]
struct UserInput {
    content: String,
    #[serde(rename = "modelId", skip_serializing_if = "Option::is_none")]
    model_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    origin: Option<String>,
    /// Cache the prompt prefix through this message. Only `default` exists.
    #[serde(rename = "cachePoint", skip_serializing_if = "Option::is_none")]
    cache_point: Option<CachePoint>,
    #[serde(
        rename = "userInputMessageContext",
        skip_serializing_if = "MessageContext::is_empty"
    )]
    context: MessageContext,
}

#[derive(Serialize)]
struct CachePoint {
    #[serde(rename = "type")]
    kind: &'static str,
}

#[derive(Serialize, Default)]
struct MessageContext {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<Value>,
    #[serde(rename = "toolResults", skip_serializing_if = "Vec::is_empty")]
    tool_results: Vec<Value>,
}

impl MessageContext {
    fn is_empty(&self) -> bool {
        self.tools.is_empty() && self.tool_results.is_empty()
    }
}

#[derive(Serialize)]
#[serde(untagged)]
enum HistoryItem {
    User {
        #[serde(rename = "userInputMessage")]
        user_input_message: UserInput,
    },
    Assistant {
        #[serde(rename = "assistantResponseMessage")]
        assistant_response_message: AssistantMessage,
    },
}

#[derive(Serialize)]
struct AssistantMessage {
    content: String,
    #[serde(rename = "toolUses", skip_serializing_if = "Vec::is_empty")]
    tool_uses: Vec<Value>,
}

// ── Building ──────────────────────────────────────────────────────────────────

/// One side of the conversation after consecutive same-side messages merge.
///
/// Kiro alternates user and assistant turns and has no `system` or `tool` role, so
/// tool results and system messages ride on the user side.
struct Turn {
    assistant: bool,
    text: Vec<String>,
    tool_uses: Vec<Value>,
    /// `(tool_use_id, content)` pairs awaiting validation against earlier calls.
    tool_results: Vec<(String, String)>,
}

impl Turn {
    fn new(assistant: bool) -> Self {
        Self {
            assistant,
            text: Vec::new(),
            tool_uses: Vec::new(),
            tool_results: Vec::new(),
        }
    }

    fn joined_text(&self) -> String {
        self.text.join("\n")
    }
}

pub fn build_conversation_state(
    conv: &ConversationRequest,
    model_id: &str,
    origin: &str,
) -> ConversationState {
    // The <thinking_mode> directive must come before the user's content.
    let thinking_prefix = conv
        .thinking
        .as_ref()
        .and_then(|t| crate::thinking::directive(model_id, t));
    let (tools, relocated_docs) = tool_specs(&conv.tools);
    let mut turns = collect_turns(conv);

    if let Some(system) = conv.system.as_deref().filter(|s| !s.is_empty()) {
        if let Some(first_user) = turns.iter_mut().find(|t| !t.assistant) {
            first_user.text.insert(0, system.to_owned());
        } else {
            let mut t = Turn::new(false);
            t.text.push(system.to_owned());
            turns.insert(0, t);
        }
    }

    demote_orphan_results(&mut turns);

    // Kiro's current message is always a user turn. A transcript ending on the
    // assistant keeps that turn in history and asks for a continuation.
    let current_turn = match turns.last() {
        Some(t) if !t.assistant => turns.pop(),
        _ => None,
    };

    let history = turns.into_iter().map(history_item).collect();

    let mut current_text = String::new();
    if !relocated_docs.is_empty() {
        current_text.push_str(&relocated_docs);
    }
    let (turn_text, results) = match &current_turn {
        Some(t) => (t.joined_text(), t.tool_results.clone()),
        None => ("Continue.".to_owned(), Vec::new()),
    };
    if !turn_text.is_empty() {
        if !current_text.is_empty() {
            current_text.push_str("\n\n");
        }
        current_text.push_str(&turn_text);
    }

    ConversationState {
        chat_trigger_type: "MANUAL",
        agent_task_type: "vibe",
        conversation_id: Uuid::new_v4().to_string(),
        current_message: CurrentMessage {
            user_input_message: UserInput {
                content: if let Some(prefix) = &thinking_prefix {
                    format!("{prefix}\n\n{current_text}")
                } else {
                    current_text
                },
                model_id: Some(model_id.to_owned()),
                origin: Some(origin.to_owned()),
                cache_point: Some(CachePoint { kind: "default" }),
                // Tools always ride on the current message, even when only the
                // history uses them: Kiro needs the schemas to validate earlier
                // `toolUses`, and rejects `toolResults` without `tools`.
                context: MessageContext {
                    tools,
                    tool_results: results.iter().map(tool_result_value).collect(),
                },
            },
        },
        history,
    }
}

/// Group messages into alternating turns, keeping text, calls and results apart.
fn collect_turns(conv: &ConversationRequest) -> Vec<Turn> {
    let mut turns: Vec<Turn> = Vec::new();
    for msg in &conv.messages {
        let assistant = msg.role == Role::Assistant;
        if turns.last().map_or(true, |t| t.assistant != assistant) {
            turns.push(Turn::new(assistant));
        }
        let turn = turns.last_mut().expect("pushed above");
        match &msg.content {
            MessageContent::Text(s) => {
                if !s.is_empty() {
                    turn.text.push(s.clone());
                }
            }
            MessageContent::Blocks(blocks) => {
                for block in blocks {
                    match block {
                        ContentBlock::Text { text } if !text.is_empty() => {
                            turn.text.push(text.clone());
                        }
                        ContentBlock::ToolUse { id, name, input } => {
                            turn.tool_uses.push(json!({
                                "toolUseId": id,
                                "name": wire_tool_name(name),
                                "input": tool_input_object(input),
                            }));
                        }
                        ContentBlock::ToolResult {
                            tool_use_id,
                            content,
                            is_error: _,
                        } => turn
                            .tool_results
                            .push((tool_use_id.clone(), content.clone())),
                        _ => {}
                    }
                }
            }
        }
    }
    turns
}

/// Turn results that answer no earlier call into plain text.
///
/// Kiro rejects `toolResults` without a preceding matching `toolUses`. Clients such
/// as Cline, Roo and Cursor send exactly that after trimming history, so the output
/// is kept as text rather than dropped or sent to be rejected.
fn demote_orphan_results(turns: &mut [Turn]) {
    let mut called: HashSet<String> = HashSet::new();
    for turn in turns.iter_mut() {
        if turn.assistant {
            for u in &turn.tool_uses {
                if let Some(id) = u["toolUseId"].as_str() {
                    called.insert(id.to_owned());
                }
            }
            continue;
        }
        let (known, orphans): (Vec<_>, Vec<_>) = std::mem::take(&mut turn.tool_results)
            .into_iter()
            .partition(|(id, _)| called.contains(id));
        turn.tool_results = known;
        for (id, content) in orphans {
            turn.text.push(format!("Tool result ({id}):\n{content}"));
        }
    }
}

fn history_item(turn: Turn) -> HistoryItem {
    let content = turn.joined_text();
    if turn.assistant {
        HistoryItem::Assistant {
            assistant_response_message: AssistantMessage {
                content,
                tool_uses: turn.tool_uses,
            },
        }
    } else {
        HistoryItem::User {
            user_input_message: UserInput {
                content,
                model_id: None,
                origin: None,
                cache_point: None,
                context: MessageContext {
                    tools: Vec::new(),
                    tool_results: turn.tool_results.iter().map(tool_result_value).collect(),
                },
            },
        }
    }
}

fn tool_result_value((id, content): &(String, String)) -> Value {
    json!({
        "toolUseId": id,
        "content": [{ "text": content }],
        "status": "success",
    })
}

/// Kiro requires an object `input`. Clients may send the streamed JSON string.
fn tool_input_object(input: &Value) -> Value {
    match input {
        Value::Object(_) => input.clone(),
        Value::String(s) => match serde_json::from_str::<Value>(s) {
            Ok(v @ Value::Object(_)) => v,
            _ => json!({}),
        },
        _ => json!({}),
    }
}

// ── Tools ─────────────────────────────────────────────────────────────────────

/// Tool specifications, plus documentation relocated out of oversized
/// descriptions so the model still sees it.
fn tool_specs(tools: &[Tool]) -> (Vec<Value>, String) {
    let mut docs: Vec<String> = Vec::new();
    let specs = tools
        .iter()
        .map(|t| {
            let name = wire_tool_name(&t.name);
            let mut description = t
                .description
                .clone()
                .filter(|d| !d.trim().is_empty())
                .unwrap_or_else(|| format!("Tool: {}", t.name));
            if description.len() > TOOL_DESCRIPTION_MAX {
                docs.push(format!("## Tool: {name}\n\n{description}"));
                description =
                    format!("[Full documentation in the message under '## Tool: {name}']");
            }
            json!({
                "toolSpecification": {
                    "name": name,
                    "description": description,
                    "inputSchema": { "json": sanitize_schema(&t.input_schema) },
                }
            })
        })
        .collect();
    (specs, docs.join("\n\n---\n\n"))
}

/// The name sent to Kiro for a tool.
///
/// Names over the limit are cut and suffixed with a hash of the full name, so two
/// long names sharing a prefix stay distinct and the same tool always maps to the
/// same wire name across turns.
pub fn wire_tool_name(name: &str) -> String {
    if name.len() <= TOOL_NAME_MAX {
        return name.to_owned();
    }
    let digest = Sha256::digest(name.as_bytes());
    let hash: String = digest
        .iter()
        .take(4)
        .fold(String::with_capacity(8), |mut s, b| {
            write!(s, "{b:02x}").expect("infallible");
            s
        });
    let hash = &hash[..7];
    let keep = TOOL_NAME_MAX - hash.len() - 1;
    let mut cut = keep;
    while !name.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}_{hash}", &name[..cut])
}

/// A tool schema Kiro will accept.
///
/// Keys are emitted in sorted order so the serialized schema is byte-stable across
/// requests; a reordered schema would defeat the upstream prompt cache.
fn sanitize_schema(schema: &Value) -> Value {
    let mut out = if let Value::Object(m) = sanitize_node(schema) {
        m
    } else {
        let mut m = Map::new();
        m.insert("type".into(), json!("object"));
        m.insert("properties".into(), json!({}));
        m
    };
    // Kiro expects the key at the top level, even when nothing is required.
    out.entry("required").or_insert_with(|| json!([]));
    Value::Object(sorted(out))
}

fn sanitize_node(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut out = Map::new();
            for (key, val) in map {
                if STRIPPED_SCHEMA_KEYS.contains(&key.as_str()) {
                    continue;
                }
                if key == "required" && val.as_array().is_some_and(Vec::is_empty) {
                    continue;
                }
                let cleaned = if key == "properties" {
                    // Property names are user data, not keywords: a property called
                    // `if` or `not` must survive, so only its schema is cleaned.
                    match val {
                        Value::Object(props) => Value::Object(sorted(
                            props
                                .iter()
                                .map(|(k, v)| (k.clone(), sanitize_node(v)))
                                .collect(),
                        )),
                        other => other.clone(),
                    }
                } else {
                    sanitize_node(val)
                };
                out.insert(key.clone(), cleaned);
            }
            Value::Object(sorted(out))
        }
        Value::Array(items) => Value::Array(items.iter().map(sanitize_node).collect()),
        other => other.clone(),
    }
}

fn sorted(map: Map<String, Value>) -> Map<String, Value> {
    let mut entries: Vec<(String, Value)> = map.into_iter().collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    entries.into_iter().collect()
}
