//! `max_tokens`, sampling and extended-thinking parameters, reconciled with the
//! rules the Anthropic API enforces so a valid client request is never refused
//! for how the gateway spelled it.

use serde_json::{json, Value};
use vkdg_operations::{ContentBlock, ConversationRequest, ToolChoice};

use super::DEFAULT_MAX_TOKENS;
use crate::turns::{tool_use_ids, Piece, Turn};

/// Smallest `budget_tokens` Anthropic accepts.
const MIN_THINKING_BUDGET: u32 = 1024;

/// With thinking on, Anthropic only accepts a `top_p` of at least this.
const MIN_THINKING_TOP_P: f32 = 0.95;

/// With thinking on, the only `temperature` Anthropic accepts.
const THINKING_TEMPERATURE: f32 = 1.0;

pub(super) struct Sampling {
    pub(super) max_tokens: u32,
    pub(super) temperature: Option<f32>,
    pub(super) top_p: Option<f32>,
    pub(super) thinking: Option<Value>,
}

/// Reconciles the client's output limit, sampling and thinking request with
/// Anthropic's constraints.
///
/// - `max_tokens` is required upstream; a client that sent none gets
///   [`DEFAULT_MAX_TOKENS`].
/// - `temperature` and `top_p` together are refused by Claude 4.5 and later, so
///   `temperature` wins (as with `OmniRoute`'s `openai-to-claude`).
/// - Thinking is not sent when `forced_tool` (Anthropic refuses thinking with a
///   forced tool) or when the request continues a tool turn whose assistant
///   message has no thinking block (`blocked_by_history`). The client asked for
///   a tool call or sent history the gateway cannot sign; honouring that beats
///   honouring the thinking request.
/// - With thinking: `budget_tokens` is at least 1024 and below `max_tokens`
///   (when the client's `max_tokens` would not leave room, the budget is added to
///   it, so the client still gets the output it asked for); `temperature` must be
///   1 and `top_p` at least 0.95, otherwise they are dropped.
pub(super) fn sampling(
    req: &ConversationRequest,
    forced_tool: bool,
    blocked_by_history: bool,
) -> Sampling {
    let mut max_tokens = req.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS);
    let mut temperature = req.temperature;
    let mut top_p = req.top_p;

    let active = req
        .thinking
        .as_ref()
        .filter(|_| !forced_tool && !blocked_by_history);
    let thinking = if let Some(request) = active {
        let budget = request.budget_tokens.unwrap_or(0).max(MIN_THINKING_BUDGET);
        if max_tokens <= budget {
            max_tokens = max_tokens.saturating_add(budget);
        }
        temperature = temperature.filter(|t| (*t - THINKING_TEMPERATURE).abs() < f32::EPSILON);
        top_p = top_p.filter(|p| *p >= MIN_THINKING_TOP_P);
        Some(json!({ "type": "enabled", "budget_tokens": budget }))
    } else {
        None
    };
    if temperature.is_some() {
        top_p = None;
    }

    Sampling {
        max_tokens,
        temperature,
        top_p,
        thinking,
    }
}

/// The request ends mid tool turn (the last user turn answers tool calls) and the
/// assistant turn that made the calls has no thinking block. Anthropic then
/// refuses `thinking` ("Expected `thinking` or `redacted_thinking`, but found
/// `tool_use`"): the block it wants is one this gateway cannot forge.
pub(super) fn continues_tool_turn_without_thinking(turns: &[Turn<'_>]) -> bool {
    let [.., assistant, user] = turns else {
        return false;
    };
    assistant.assistant
        && !user.assistant
        && !tool_use_ids(assistant).is_empty()
        && !assistant.pieces.iter().any(|p| {
            matches!(
                p,
                Piece::Block(ContentBlock::Thinking { .. } | ContentBlock::RedactedThinking { .. })
            )
        })
}

/// `tool_choice` that forces a call.
pub(super) fn forces_tool(choice: Option<&ToolChoice>) -> bool {
    matches!(choice, Some(ToolChoice::Required | ToolChoice::Named(_)))
}
