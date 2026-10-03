//! Anthropic Messages request body, shared by every provider that speaks it.
//!
//! Adapters (`anthropic`, `claude-code`) own URL, headers and identity; this
//! module owns what the body means: block shapes ([`blocks`]) and the parameter
//! rules the API enforces ([`thinking`]). Turn arrangement is shared with the
//! `OpenAI` builder in [`crate::turns`].

mod blocks;
mod thinking;

use bytes::Bytes;
use serde_json::{json, Map, Value};
use vkdg_operations::{ConversationRequest, ToolChoice};

use crate::sampling_json::insert_f32;
use crate::turns::arrange;

/// `max_tokens` for a client that sent none. Anthropic requires the field and
/// has no default. 8192 is large enough for a coding-agent reply and within the
/// output limit of every current Sonnet, Opus and Haiku 4.x/3.5 model. It is NOT
/// within Claude 3 Haiku/Opus (4096): callers routed there must send their own.
/// The repo carries no per-model output limits to clamp against.
pub const DEFAULT_MAX_TOKENS: u32 = 8192;

/// Serializes `req` as a `/v1/messages` JSON body for `model`.
///
/// `system_preamble`, when given, becomes the first `system` block and the
/// client's system prompt follows it unchanged as its own block (the Claude Code
/// subscription gate needs exactly that). Without it `system` is one string.
///
/// Tool history is repaired to what Anthropic validates (see
/// [`crate::turns::arrange`]) and `max_tokens`, sampling and `thinking` are
/// reconciled with its rules (see [`thinking::sampling`]).
pub fn messages_body(
    req: &ConversationRequest,
    model: &str,
    system_preamble: Option<&str>,
) -> Bytes {
    let conversation = arrange(&req.messages);
    let tool_choice = effective_tool_choice(req);
    let sampling = thinking::sampling(
        req,
        thinking::forces_tool(tool_choice),
        thinking::continues_tool_turn_without_thinking(&conversation.turns),
    );

    let mut body = Map::new();
    body.insert("model".into(), Value::String(model.into()));
    body.insert("max_tokens".into(), json!(sampling.max_tokens));
    body.insert(
        "messages".into(),
        Value::Array(
            conversation
                .turns
                .iter()
                .map(blocks::turn_to_wire)
                .collect(),
        ),
    );

    let system = system_text(req, &conversation.system_text);
    match (system_preamble, system) {
        (Some(preamble), system) => {
            let mut blocks = vec![json!({ "type": "text", "text": preamble })];
            blocks.extend(system.map(|s| json!({ "type": "text", "text": s })));
            body.insert("system".into(), Value::Array(blocks));
        }
        (None, Some(system)) => {
            body.insert("system".into(), Value::String(system));
        }
        (None, None) => {}
    }

    if let Some(t) = sampling.temperature {
        insert_f32(&mut body, "temperature", t);
    }
    if let Some(p) = sampling.top_p {
        insert_f32(&mut body, "top_p", p);
    }
    if !req.stop_sequences.is_empty() {
        body.insert("stop_sequences".into(), json!(req.stop_sequences));
    }
    if req.stream {
        body.insert("stream".into(), Value::Bool(true));
    }
    if has_tools(req) {
        let mut tools: Vec<Value> = req
            .tools
            .iter()
            .map(|t| {
                let mut tool = Map::new();
                tool.insert("name".into(), Value::String(t.name.clone()));
                if let Some(desc) = &t.description {
                    tool.insert("description".into(), Value::String(desc.clone()));
                }
                tool.insert("input_schema".into(), t.input_schema.clone());
                Value::Object(tool)
            })
            .collect();
        // Provider-run tools go as the client declared them.
        tools.extend(req.server_tools.iter().map(|t| t.declaration.clone()));
        body.insert("tools".into(), Value::Array(tools));
    }
    if let Some(choice) = tool_choice_wire(req, tool_choice) {
        body.insert("tool_choice".into(), choice);
    }
    if let Some(thinking) = sampling.thinking {
        body.insert("thinking".into(), thinking);
    }

    // A `Value` tree always serializes; the empty fallback only satisfies the type.
    Bytes::from(serde_json::to_vec(&Value::Object(body)).unwrap_or_default())
}

/// Whether the request declares anything for a tool call to act on.
fn has_tools(req: &ConversationRequest) -> bool {
    !req.tools.is_empty() || !req.server_tools.is_empty()
}

/// The client's `tool_choice` when there are tools for it to act on.
fn effective_tool_choice(req: &ConversationRequest) -> Option<&ToolChoice> {
    req.tool_choice.as_ref().filter(|_| has_tools(req))
}

/// Anthropic's `tool_choice` object. The parallel-call limit lives inside it, so
/// a client that sent only the limit still gets an explicit `auto`.
fn tool_choice_wire(req: &ConversationRequest, choice: Option<&ToolChoice>) -> Option<Value> {
    if !has_tools(req) {
        return None;
    }
    let mut wire = match choice {
        Some(ToolChoice::Disabled) => return Some(json!({ "type": "none" })),
        Some(ToolChoice::Auto) => json!({ "type": "auto" }),
        Some(ToolChoice::Required) => json!({ "type": "any" }),
        Some(ToolChoice::Named(name)) => json!({ "type": "tool", "name": name }),
        None if req.disable_parallel_tool_use => json!({ "type": "auto" }),
        None => return None,
    };
    if req.disable_parallel_tool_use {
        wire["disable_parallel_tool_use"] = Value::Bool(true);
    }
    Some(wire)
}

/// The client's system prompt followed by any system-role messages found in the
/// history, which Anthropic accepts only at the top level.
fn system_text(req: &ConversationRequest, from_history: &[&str]) -> Option<String> {
    let parts: Vec<&str> = req
        .system
        .as_deref()
        .into_iter()
        .chain(from_history.iter().copied())
        .filter(|s| !s.trim().is_empty())
        .collect();
    (!parts.is_empty()).then(|| parts.join("\n\n"))
}

#[cfg(test)]
mod tests;
