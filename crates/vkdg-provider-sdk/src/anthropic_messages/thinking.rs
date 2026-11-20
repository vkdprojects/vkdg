//! `max_tokens`, sampling and extended-thinking parameters, reconciled with the
//! rules the Anthropic API enforces so a valid client request is never refused
//! for how the gateway spelled it.

use serde_json::{json, Value};
use vkdg_operations::{ContentBlock, ConversationRequest, ThinkingRequest, ToolChoice};

use super::DEFAULT_MAX_TOKENS;
use crate::turns::{tool_use_ids, Piece, Turn};

/// Smallest `budget_tokens` Anthropic accepts.
const MIN_THINKING_BUDGET: u32 = 1024;

/// `budget_tokens` an effort-only request maps to.
const LOW_BUDGET: u32 = 4096;
const MEDIUM_BUDGET: u32 = 16_384;
const HIGH_BUDGET: u32 = 32_768;

/// With thinking on, Anthropic only accepts a `top_p` of at least this.
const MIN_THINKING_TOP_P: f32 = 0.95;

/// With thinking on, the only `temperature` Anthropic accepts.
const THINKING_TEMPERATURE: f32 = 1.0;

pub(super) struct Sampling {
    pub(super) max_tokens: u32,
    pub(super) temperature: Option<f32>,
    pub(super) top_p: Option<f32>,
    pub(super) thinking: Option<Value>,
    /// `output_config.effort`, Anthropic's depth control for models that have it.
    pub(super) effort: Option<&'static str>,
}

/// An effort level, ordered by how much the model is asked to think.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Effort {
    Low,
    Medium,
    High,
    XHigh,
    Max,
}

impl Effort {
    fn wire(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::XHigh => "xhigh",
            Self::Max => "max",
        }
    }

    /// `budget_tokens` for a model that takes one. `high` is the floor for
    /// `xhigh`/`max`, which grow to three quarters of the client's `max_tokens`
    /// when it leaves that much room.
    fn budget(self, max_tokens: u32) -> u32 {
        match self {
            Self::Low => LOW_BUDGET,
            Self::Medium => MEDIUM_BUDGET,
            Self::High => HIGH_BUDGET,
            Self::XHigh | Self::Max => HIGH_BUDGET.max(max_tokens / 4 * 3),
        }
    }
}

/// What the client asked for, in its own words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ask {
    /// `reasoning_effort: none` / `off`.
    Off,
    Level(Effort),
}

fn parse_level(raw: &str) -> Option<Ask> {
    Some(match raw.trim().to_ascii_lowercase().as_str() {
        "none" | "off" => Ask::Off,
        "minimal" | "min" | "low" => Ask::Level(Effort::Low),
        "medium" => Ask::Level(Effort::Medium),
        "high" => Ask::Level(Effort::High),
        "xhigh" => Ask::Level(Effort::XHigh),
        "max" => Ask::Level(Effort::Max),
        _ => return None,
    })
}

/// The explicit effort, else the level the budget stands for. `None` when the
/// client gave neither (an `adaptive` request without an effort).
fn explicit_effort(request: &ThinkingRequest) -> Option<Ask> {
    request.effort.as_deref().and_then(parse_level)
}

fn ask_of(request: &ThinkingRequest) -> Option<Ask> {
    explicit_effort(request).or_else(|| {
        request
            .budget_tokens
            .and_then(|_| parse_level(request.effort_level()))
    })
}

const LOW_TO_HIGH: &[Effort] = &[Effort::Low, Effort::Medium, Effort::High];
const LOW_TO_MAX: &[Effort] = &[Effort::Low, Effort::Medium, Effort::High, Effort::Max];
const LOW_TO_XHIGH_MAX: &[Effort] = &[
    Effort::Low,
    Effort::Medium,
    Effort::High,
    Effort::XHigh,
    Effort::Max,
];

/// What a Claude model accepts for thinking depth, from its id. Unknown ids get
/// the legacy treatment: a `budget_tokens` and no effort.
#[derive(Debug, Default, Clone, Copy)]
struct Caps {
    /// The levels `output_config.effort` accepts; empty when the model has none.
    levels: &'static [Effort],
    /// Refuses `thinking: enabled` + `budget_tokens` (400); depth is the effort.
    adaptive_only: bool,
    /// Thinks even without a `thinking` field.
    always_thinks: bool,
}

impl Caps {
    /// `claude-opus-5-5`, `claude-opus-5.5`, `claude-opus-4-5-20251101`, ...
    /// (<https://platform.claude.com/docs/en/build-with-claude/effort> and
    /// `/thinking`: the per-model tables).
    fn of(model: &str) -> Self {
        let id = model.replace('.', "-");
        let mut parts = id.split('-');
        if parts.next() != Some("claude") {
            return Self::default();
        }
        let (Some(family), Some(Ok(major))) = (parts.next(), parts.next().map(str::parse::<u32>))
        else {
            return Self::default();
        };
        // A third numeric part of one or two digits is the minor version; a date
        // suffix (`20251101`) is longer.
        let minor = parts
            .next()
            .filter(|p| p.len() <= 2)
            .and_then(|p| p.parse::<u32>().ok())
            .unwrap_or(0);
        let version = (major, minor);
        let claude5 = major >= 5;
        let levels = match family {
            "opus" if version >= (4, 7) => LOW_TO_XHIGH_MAX,
            "opus" if version >= (4, 6) => LOW_TO_MAX,
            "opus" if version >= (4, 5) => LOW_TO_HIGH,
            "sonnet" if claude5 => LOW_TO_XHIGH_MAX,
            "sonnet" if version >= (4, 6) => LOW_TO_MAX,
            "haiku" | "fable" | "mythos" if claude5 => LOW_TO_XHIGH_MAX,
            _ => &[],
        };
        let adaptive_only = match family {
            "opus" => version >= (4, 7),
            "sonnet" | "haiku" | "fable" | "mythos" => claude5,
            _ => false,
        };
        Self {
            levels,
            adaptive_only,
            always_thinks: claude5 && !levels.is_empty(),
        }
    }

    /// The highest level the model has at or below `level`.
    fn clamp(self, level: Effort) -> Option<Effort> {
        self.levels.iter().copied().rfind(|l| *l <= level)
    }
}

/// Reconciles the client's output limit, sampling and thinking request with
/// Anthropic's constraints for `model`.
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
/// - Depth follows the client's request: an effort (`low`..`max`) or a budget.
///   Models that refuse `budget_tokens` (Opus 4.7+, every Claude 5) get
///   `thinking: {type: adaptive, display: summarized}` (Claude 5 defaults to
///   `omitted`, which would hide the reasoning the client asked for) and the effort as `output_config.effort`, the
///   only depth control they have; a budget there is read as the effort it stands
///   for. Other models get `thinking: enabled` with `budget_tokens`: the client's
///   own, else the one the effort maps to (low 4096, medium 16384, high 32768,
///   `xhigh`/`max` more when `max_tokens` allows). An effort the client sent is
///   also passed as `output_config.effort` where the model accepts that level
///   (a lower level where it does not), with or without thinking. `none`/`off`
///   sends no thinking; a model that always thinks gets the lowest effort.
/// - With a budget: it is at least 1024 and below `max_tokens` (when the client's
///   `max_tokens` would not leave room, the budget is added to it, so the client
///   still gets the output it asked for). With thinking, `temperature` must be 1
///   and `top_p` at least 0.95, otherwise they are dropped.
pub(super) fn sampling(
    req: &ConversationRequest,
    model: &str,
    forced_tool: bool,
    blocked_by_history: bool,
) -> Sampling {
    let mut max_tokens = req.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS);
    let mut temperature = req.temperature;
    let mut top_p = req.top_p;

    let caps = Caps::of(model);
    let request = req.thinking.as_ref();
    let ask = request.and_then(ask_of);

    let thinking = match request {
        Some(request) if ask != Some(Ask::Off) && !forced_tool && !blocked_by_history => {
            temperature = temperature.filter(|t| (*t - THINKING_TEMPERATURE).abs() < f32::EPSILON);
            top_p = top_p.filter(|p| *p >= MIN_THINKING_TOP_P);
            if caps.adaptive_only {
                Some(json!({ "type": "adaptive", "display": "summarized" }))
            } else {
                let budget = request
                    .budget_tokens
                    .or_else(|| match ask {
                        Some(Ask::Level(level)) => Some(level.budget(max_tokens)),
                        _ => None,
                    })
                    .unwrap_or(0)
                    .max(MIN_THINKING_BUDGET);
                if max_tokens <= budget {
                    max_tokens = max_tokens.saturating_add(budget);
                }
                Some(json!({ "type": "enabled", "budget_tokens": budget }))
            }
        }
        _ => None,
    };
    if temperature.is_some() {
        top_p = None;
    }

    let effort = match (request, ask) {
        (Some(_), Some(Ask::Off)) if caps.always_thinks => Some(Effort::Low),
        (Some(request), Some(Ask::Level(level)))
            if caps.adaptive_only || explicit_effort(request).is_some() =>
        {
            caps.clamp(level)
        }
        _ => None,
    };

    Sampling {
        max_tokens,
        temperature,
        top_p,
        thinking,
        effort: effort.map(Effort::wire),
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
