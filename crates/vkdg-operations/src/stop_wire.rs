//! `StopReason` on the wire, in both directions and both dialects.

use crate::StopReason;

/// Anthropic `stop_reason`.
pub fn anthropic_stop_reason(reason: &StopReason) -> &'static str {
    match reason {
        StopReason::EndTurn | StopReason::Cancelled => "end_turn",
        StopReason::MaxTokens => "max_tokens",
        StopReason::ToolUse => "tool_use",
        StopReason::StopSequence => "stop_sequence",
    }
}

/// `OpenAI` `finish_reason`.
pub fn openai_finish_reason(reason: &StopReason) -> &'static str {
    match reason {
        StopReason::EndTurn | StopReason::Cancelled | StopReason::StopSequence => "stop",
        StopReason::MaxTokens => "length",
        StopReason::ToolUse => "tool_calls",
    }
}

/// An Anthropic `stop_reason` read from upstream. Values the gateway does not
/// know (`pause_turn`, `refusal`, future ones) end the turn: the client still
/// receives the content that arrived.
pub fn parse_anthropic_stop_reason(raw: &str) -> StopReason {
    match raw {
        "max_tokens" | "model_context_window_exceeded" => StopReason::MaxTokens,
        "tool_use" => StopReason::ToolUse,
        "stop_sequence" => StopReason::StopSequence,
        _ => StopReason::EndTurn,
    }
}

/// An `OpenAI` `finish_reason` read from upstream (`function_call` is the
/// legacy spelling of `tool_calls`).
pub fn parse_openai_finish_reason(raw: &str) -> StopReason {
    match raw {
        "length" => StopReason::MaxTokens,
        "tool_calls" | "function_call" => StopReason::ToolUse,
        _ => StopReason::EndTurn,
    }
}
