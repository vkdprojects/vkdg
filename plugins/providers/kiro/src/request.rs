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

/// Tool result content in history turns is truncated to this length to prevent
/// 50 k+ char file contents from being resent on every turn of the conversation.
const TOOL_RESULT_HISTORY_MAX: usize = 2_000;

/// Max chars for text / tool-result content in mid-range history (distance 4–7 from end).
const AGING_MID_TEXT_MAX: usize = 500;
const AGING_MID_TOOL_MAX: usize = 500;

/// Max chars for text / tool-result content in distant history (distance 8+ from end).
const AGING_FAR_TEXT_MAX: usize = 120;
const AGING_FAR_TOOL_MAX: usize = 50;
/// Hard cap on history turns sent to Kiro. The progressive aging pipeline
/// compresses content but does not bound turn count; beyond ~100 turns the
/// context window fills and Kiro returns empty responses, causing the omp
/// agent to loop without producing output.
const MAX_HISTORY_TURNS: usize = 100;

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

/// Builds the Kiro `conversationState` for `conv`.
///
/// Request fields with no Kiro counterpart are not forwarded, on purpose:
/// `tool_choice`, `stop_sequences`, `top_p`, `temperature`, `max_tokens` and
/// `disable_parallel_tool_use`. Kiro's wire format has no place for them.
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
        // Filter out internal client headers injected by coding agents (omp, Claude Code).
        // The billing header (x-anthropic-billing-header: cc_version=...) arrives as a text
        // block in the system array. Kiro has no separate system field so it gets prepended
        // into the first user turn — if not removed, the model sees it as user content and
        // incorrectly treats it as a pasted system prompt injection.
        let clean: String = system
            .lines()
            .filter(|l| !l.trim_start().starts_with("x-anthropic-billing-header:"))
            .collect::<Vec<_>>()
            .join("\n");
        let clean = clean.trim();
        if !clean.is_empty() {
            if let Some(first_user) = turns.iter_mut().find(|t| !t.assistant) {
                first_user.text.insert(0, clean.to_owned());
            } else {
                let mut t = Turn::new(false);
                t.text.push(clean.to_owned());
                turns.insert(0, t);
            }
        }
    }

    demote_orphan_results(&mut turns);
    // Cap history depth before aging. The progressive aging pipeline compresses
    // content but does not bound turn count; at 2000+ turns the context fills
    // and Kiro returns out=0 responses, causing the omp agent to loop silently.
    // Keep the first user turn when it carries the system prompt (injected above).
    let system_injected = conv.system.as_deref().is_some_and(|s| !s.is_empty());
    // An assistant-ended transcript also needs the synthesized current user.
    let max_turns =
        MAX_HISTORY_TURNS - usize::from(turns.last().is_some_and(|turn| turn.assistant));
    if turns.len() > max_turns {
        let drop_from = usize::from(system_injected);
        let mut drop_to = drop_from + turns.len() - max_turns;
        // Start the retained suffix on an assistant turn, never on the user
        // result of a removed call. With the protected first user this also
        // keeps the two user turns from becoming adjacent.
        if !turns[drop_to].assistant {
            drop_to += 1;
        }
        turns.drain(drop_from..drop_to);
    }

    // Kiro's current message is always a user turn. A transcript ending on the
    // assistant keeps that turn in history and asks for a continuation.
    let current_turn = match turns.last() {
        Some(t) if !t.assistant => turns.pop(),
        _ => None,
    };

    // Cache the prefix at the last stable user turn in history.
    // AWS CodeWhisperer caches everything up to the cache_point; the last user
    // turn in history is the freshest position that does NOT change on the next
    // request, so the cached prefix grows with the conversation instead of
    // being invalidated on every turn.
    // Single-turn (no history): cache_point stays on currentMessage (below).
    let last_user_idx =
        turns
            .iter()
            .enumerate()
            .rev()
            .find_map(|(i, t)| if t.assistant { None } else { Some(i) });
    let has_history_user = last_user_idx.is_some();
    let history_len = turns.len();
    // When a system prompt was injected, the first user turn carries it;
    // the aging pipeline must not truncate it (same protection as the drain above).
    let protected_first_user = if system_injected {
        turns
            .iter()
            .enumerate()
            .find_map(|(i, t)| if t.assistant { None } else { Some(i) })
    } else {
        None
    };
    let history: Vec<HistoryItem> = turns
        .into_iter()
        .enumerate()
        .map(|(i, turn)| {
            let distance_from_end = history_len.saturating_sub(i + 1);
            // Never age the first user turn when it carries the system prompt.
            let effective_distance = if protected_first_user == Some(i) {
                0
            } else {
                distance_from_end
            };
            history_item_aged(turn, Some(i) == last_user_idx, effective_distance)
        })
        .collect();

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
        conversation_id: conversation_id(conv),
        current_message: CurrentMessage {
            user_input_message: UserInput {
                content: if let Some(prefix) = &thinking_prefix {
                    format!("{prefix}\n\n{current_text}")
                } else {
                    current_text
                },
                model_id: Some(model_id.to_owned()),
                origin: Some(origin.to_owned()),
                // cache_point goes to last history user turn in multi-turn conversations.
                // In single-turn (no history), it falls back here so AWS can still cache.
                cache_point: if has_history_user {
                    None
                } else {
                    Some(CachePoint { kind: "default" })
                },
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

/// The Kiro `conversationId` for one request.
///
/// Precedence: an explicit client `session_id`; else a UUID derived from the
/// gateway's `cache_key`, so every turn of one gateway conversation names the
/// same Kiro conversation; else a fresh random UUID. The derived id is the first
/// 16 bytes of `SHA-256(cache_key)` with version-4 and RFC 4122 variant bits set,
/// so it is a valid UUID and reveals nothing beyond the (already hashed) key.
fn conversation_id(conv: &ConversationRequest) -> String {
    if let Some(id) = conv.session_id.as_deref().filter(|s| !s.is_empty()) {
        return id.to_owned();
    }
    match conv.cache_key.as_deref().filter(|k| !k.is_empty()) {
        Some(key) => {
            let digest = Sha256::digest(key.as_bytes());
            let mut bytes = [0u8; 16];
            bytes.copy_from_slice(&digest[..16]);
            uuid::Builder::from_random_bytes(bytes)
                .into_uuid()
                .to_string()
        }
        None => Uuid::new_v4().to_string(),
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
                        ContentBlock::Text { text, .. } if !text.is_empty() => {
                            turn.text.push(text.clone());
                        }
                        ContentBlock::ToolUse {
                            id, name, input, ..
                        } => {
                            turn.tool_uses.push(json!({
                                "toolUseId": id,
                                "name": wire_tool_name(name),
                                "input": tool_input_object(input),
                            }));
                        }
                        // Kiro's `toolResults` content is text or JSON; images have no slot.
                        ContentBlock::ToolResult {
                            tool_use_id,
                            content,
                            ..
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

/// Truncate a tool result's content for history turns.
///
/// Long results (file reads, grep output) can exceed 50 k bytes. Resending that
/// on every subsequent turn wastes tokens and defeats the upstream prompt cache.
/// The current turn is always sent in full; only history turns are truncated.
fn truncate_history_content(content: &str) -> String {
    if content.len() <= TOOL_RESULT_HISTORY_MAX {
        content.to_owned()
    } else {
        let total = content.len();
        let mut end = TOOL_RESULT_HISTORY_MAX;
        while !content.is_char_boundary(end) {
            end -= 1;
        }
        let mut s = content[..end].to_owned();
        use std::fmt::Write as _;
        let _ = write!(s, "...[truncated, {total} bytes total]");
        s
    }
}

fn history_item_with_cache(turn: Turn, place_cache_point: bool) -> HistoryItem {
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
                cache_point: if place_cache_point {
                    Some(CachePoint { kind: "default" })
                } else {
                    None
                },
                context: MessageContext {
                    tools: Vec::new(),
                    tool_results: turn
                        .tool_results
                        .iter()
                        .map(|(id, content)| {
                            tool_result_value(&(id.clone(), truncate_history_content(content)))
                        })
                        .collect(),
                },
            },
        }
    }
}

/// Truncate `content` to at most `max` bytes, appending "…" when cut.
///
/// Stays on a UTF-8 char boundary so the result is always valid UTF-8.
fn truncate_aged(content: &str, max: usize) -> String {
    if content.len() <= max {
        return content.to_owned();
    }
    // Walk backward from `max` to find the last char boundary.
    let end = (0..=max)
        .rev()
        .find(|&i| content.is_char_boundary(i))
        .unwrap_or(0);
    format!("{}…", &content[..end])
}

/// Produce a history item with content compressed according to how far the turn
/// is from the end of the conversation.
///
/// | distance_from_end | text limit | tool-result limit |
/// |-------------------|------------|-------------------|
/// | 0–3 (recent)      | 2 000 (verbatim, via history_item_with_cache) | 2 000 |
/// | 4–7 (mid)         | 500 chars  | 500 chars         |
/// | 8+  (distant)     | 120 chars (first line) | 50 chars |
fn history_item_aged(turn: Turn, place_cache_point: bool, distance_from_end: usize) -> HistoryItem {
    if distance_from_end <= 3 {
        return history_item_with_cache(turn, place_cache_point);
    }

    let (text_max, tool_max, first_line_only) = if distance_from_end <= 7 {
        (AGING_MID_TEXT_MAX, AGING_MID_TOOL_MAX, false)
    } else {
        (AGING_FAR_TEXT_MAX, AGING_FAR_TOOL_MAX, true)
    };

    let raw = turn.joined_text();
    let content = if first_line_only {
        let first_line = raw.lines().next().unwrap_or("");
        truncate_aged(first_line, text_max)
    } else {
        truncate_aged(&raw, text_max)
    };

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
                cache_point: if place_cache_point {
                    Some(CachePoint { kind: "default" })
                } else {
                    None
                },
                context: MessageContext {
                    tools: Vec::new(),
                    tool_results: turn
                        .tool_results
                        .iter()
                        .map(|(id, c)| tool_result_value(&(id.clone(), truncate_aged(c, tool_max))))
                        .collect(),
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

#[cfg(test)]
mod truncation_tests {
    use super::truncate_history_content;

    // A multibyte character straddling the byte limit must not panic or be split.
    #[test]
    fn history_tool_result_preserves_utf8_at_byte_limit() {
        let content = format!("{}→tail", "a".repeat(1999));
        assert_eq!(
            truncate_history_content(&content),
            format!("{}...[truncated, 2006 bytes total]", "a".repeat(1999))
        );
        let ascii = format!("{}tail", "a".repeat(2000));
        assert_eq!(
            truncate_history_content(&ascii),
            format!("{}...[truncated, 2004 bytes total]", "a".repeat(2000))
        );
    }
}

#[cfg(test)]
mod conversation_id_tests {
    use super::*;

    fn conv(session: Option<&str>, key: Option<&str>) -> ConversationRequest {
        ConversationRequest {
            session_id: session.map(str::to_owned),
            cache_key: key.map(str::to_owned),
            ..Default::default()
        }
    }

    const KEY_A: &str = "3f2a9c0d5b7e41a8c6d2e0f19b8a7c5d4e3f2a1b0c9d8e7f6a5b4c3d2e1f0a9b";
    const KEY_B: &str = "9b0a1f2e3d4c5b6a79881706f5e4d3c2b1a0f9e8d7c6b5a4938271605f4e3d2c";

    // Refutes a fresh UUID per request: one gateway conversation must map to one
    // Kiro conversation, and the id must be a well-formed v4 / RFC 4122 UUID.
    #[test]
    fn cache_key_derives_a_deterministic_valid_uuid() {
        let a1 = conversation_id(&conv(None, Some(KEY_A)));
        let a2 = conversation_id(&conv(None, Some(KEY_A)));
        assert_eq!(a1, a2);
        let parsed = Uuid::parse_str(&a1).unwrap();
        assert_eq!(parsed.get_version_num(), 4);
        assert_eq!(parsed.get_variant(), uuid::Variant::RFC4122);
        assert_ne!(a1, KEY_A);
    }

    #[test]
    fn different_cache_keys_give_different_ids() {
        assert_ne!(
            conversation_id(&conv(None, Some(KEY_A))),
            conversation_id(&conv(None, Some(KEY_B)))
        );
    }

    // Refutes the derived id overriding a client-named conversation.
    #[test]
    fn explicit_session_id_wins_over_cache_key() {
        assert_eq!(
            conversation_id(&conv(Some("client-session"), Some(KEY_A))),
            "client-session"
        );
    }

    // Refutes a constant id for requests that have nothing to anchor on.
    #[test]
    fn no_session_and_no_cache_key_is_random() {
        let a = conversation_id(&conv(None, None));
        let b = conversation_id(&conv(None, None));
        assert_ne!(a, b);
        assert!(Uuid::parse_str(&a).is_ok());
        assert_ne!(
            conversation_id(&conv(None, Some(""))),
            conversation_id(&conv(None, Some("")))
        );
    }
}
