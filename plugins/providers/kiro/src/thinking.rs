//! Thinking / reasoning controls for Kiro (AWS CodeWhisperer).
//!
//! Two coordinated signals steer reasoning on the Kiro surface, ported from
//! OmniRoute's `adaptiveThinking.ts` and `kiroThinking.ts`:
//!
//! 1. **`additionalModelRequestFields`** with `output_config.effort` +
//!    `thinking:{type:"adaptive"}` (or `reasoning.effort` for GPT-5.6 models).
//!    Gated on exact model allowlists — sending it to `claude-sonnet-4.5` causes
//!    a Bedrock 400 even though that model supports thinking on the direct API.
//! 2. **`<thinking_mode>enabled</thinking_mode><max_thinking_length>N</max_thinking_length>`**
//!    prepended to the user message, which makes Claude emit reasoning inline as
//!    `<thinking>…</thinking>` blocks instead of separate `reasoningContentEvent`
//!    frames. The decoder splits them back out.

use vkdg_operations::ThinkingRequest;

const ADAPTIVE_MODELS: &[&str] = &["claude-opus-5", "claude-sonnet-5"];
const NATIVE_REASONING_MODELS: &[&str] = &["gpt-5.6-sol", "gpt-5.6-terra", "gpt-5.6-luna"];
const KIRO_EFFORT_LEVELS: &[&str] = &["low", "medium", "high", "xhigh", "max"];

fn supports_adaptive(model: &str) -> bool {
    ADAPTIVE_MODELS.contains(&model)
}
fn supports_native_reasoning(model: &str) -> bool {
    NATIVE_REASONING_MODELS.contains(&model)
}

fn effort_from_budget(budget: u32) -> &'static str {
    if budget >= 32_000 {
        "high"
    } else if budget >= 16_000 {
        "medium"
    } else {
        "low"
    }
}

fn thinking_length_for_effort(effort: &str) -> u32 {
    match effort {
        "max" => 120_000,
        "xhigh" => 64_000,
        "high" => 32_000,
        "medium" => 16_000,
        _ => 8_000,
    }
}

pub(crate) fn resolve_effort(t: &ThinkingRequest) -> &'static str {
    match t.effort.as_deref() {
        Some("minimal") => "low",
        Some(e) if !e.is_empty() => KIRO_EFFORT_LEVELS
            .iter()
            .copied()
            .find(|&k| k == e)
            .unwrap_or(""),
        _ => effort_from_budget(t.budget_tokens.unwrap_or(0)),
    }
}

/// `additionalModelRequestFields` for the Kiro body. `None` = model not on allowlist.
pub(crate) fn build_fields(model: &str, t: &ThinkingRequest) -> Option<AdditionalFields> {
    let effort = resolve_effort(t);
    if effort.is_empty() {
        return None;
    }
    if supports_native_reasoning(model) {
        Some(AdditionalFields::NativeReasoning {
            reasoning: EffortField {
                effort: effort.into(),
            },
        })
    } else if supports_adaptive(model) {
        Some(AdditionalFields::Adaptive {
            output_config: EffortField {
                effort: effort.into(),
            },
            thinking: AdaptiveThinking {
                kind: "adaptive",
                display: "summarized",
            },
        })
    } else {
        None
    }
}

/// `<thinking_mode>` directive to prepend to the user message. `None` = not applicable.
pub(crate) fn directive(model: &str, t: &ThinkingRequest) -> Option<String> {
    if !supports_adaptive(model) {
        return None;
    }
    let effort = resolve_effort(t);
    if effort.is_empty() {
        return None;
    }
    let len = thinking_length_for_effort(effort);
    Some(format!(
        "<thinking_mode>enabled</thinking_mode><max_thinking_length>{len}</max_thinking_length>"
    ))
}

#[derive(serde::Serialize)]
#[serde(untagged)]
pub(crate) enum AdditionalFields {
    NativeReasoning {
        reasoning: EffortField,
    },
    Adaptive {
        output_config: EffortField,
        thinking: AdaptiveThinking,
    },
}

#[derive(serde::Serialize)]
pub(crate) struct EffortField {
    pub(crate) effort: String,
}

#[derive(serde::Serialize)]
pub(crate) struct AdaptiveThinking {
    #[serde(rename = "type")]
    pub(crate) kind: &'static str,
    pub(crate) display: &'static str,
}

#[cfg(test)]
mod tests {
    use super::*;
    use vkdg_operations::ThinkingRequest;

    fn budget(b: u32) -> ThinkingRequest {
        ThinkingRequest {
            budget_tokens: Some(b),
            effort: None,
        }
    }
    fn eff(e: &str) -> ThinkingRequest {
        ThinkingRequest {
            budget_tokens: None,
            effort: Some(e.into()),
        }
    }

    #[test]
    fn non_allowlisted_model_gets_nothing() {
        assert!(build_fields("claude-sonnet-4.5", &budget(50_000)).is_none());
        assert!(build_fields("claude-haiku-4.5", &budget(10_000)).is_none());
    }

    #[test]
    fn adaptive_model_gets_output_config_and_directive() {
        let fields = build_fields("claude-opus-5", &budget(32_000)).unwrap();
        let j = serde_json::to_value(&fields).unwrap();
        assert_eq!(j["output_config"]["effort"], "high");
        assert_eq!(j["thinking"]["type"], "adaptive");
        let d = directive("claude-opus-5", &budget(32_000)).unwrap();
        assert!(d.contains("<thinking_mode>enabled</thinking_mode>"), "{d}");
        assert!(
            d.contains("<max_thinking_length>32000</max_thinking_length>"),
            "{d}"
        );
    }

    #[test]
    fn gpt56_gets_reasoning_field_not_output_config() {
        let fields = build_fields("gpt-5.6-sol", &eff("high")).unwrap();
        let j = serde_json::to_value(&fields).unwrap();
        assert!(j.get("output_config").is_none(), "{j}");
        assert_eq!(j["reasoning"]["effort"], "high");
        assert!(directive("gpt-5.6-sol", &eff("high")).is_none());
    }

    #[test]
    fn effort_string_beats_budget() {
        assert_eq!(
            resolve_effort(&ThinkingRequest {
                budget_tokens: Some(100),
                effort: Some("max".into())
            }),
            "max"
        );
    }

    #[test]
    fn unknown_effort_produces_nothing() {
        assert!(build_fields("claude-opus-5", &eff("ultra")).is_none());
    }
}
