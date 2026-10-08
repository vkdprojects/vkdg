//! Anthropic provider adapter — implements [`ProviderAdapter`] for the Anthropic
//! Messages wire format.  The pipeline core has no knowledge of Anthropic
//! specifics; it only calls `prepare()` and uses the returned [`PreparedRequest`].

use bytes::Bytes;
use http::HeaderMap;
use vkdg_connections::{ConnectionConfig, ProviderKind};
use vkdg_core::pricing::ModelPrice;
use vkdg_operations::{ConversationRequest, Operation};
use vkdg_provider_sdk::{Credential, PreparedRequest, ProviderAdapter, ProviderError};

pub struct AnthropicAdapter;

/// Anthropic list prices (USD per million tokens, as microdollars), specific
/// patterns first. Opus 4 and 4.1 are $15/$75, later Opus 4.x $5/$25. Names
/// are matched with `.` read as `-`, so `claude-opus-4.5` is `claude-opus-4-5`.
const PRICES: &[ModelPrice] = &[
    ModelPrice::new("claude-opus-4-1*", 15_000_000, 75_000_000),
    // Dated Opus 4 ids (`claude-opus-4-20250514`), not an "Opus 4.2".
    ModelPrice::new("claude-opus-4-2025*", 15_000_000, 75_000_000),
    ModelPrice::new("claude-opus-4-*", 5_000_000, 25_000_000),
    ModelPrice::new("claude-opus-4*", 15_000_000, 75_000_000),
    ModelPrice::new("claude-3-opus*", 15_000_000, 75_000_000),
    ModelPrice::new("claude-sonnet-*", 3_000_000, 15_000_000),
    ModelPrice::new("claude-3-7-sonnet*", 3_000_000, 15_000_000),
    ModelPrice::new("claude-3-5-sonnet*", 3_000_000, 15_000_000),
    ModelPrice::new("claude-haiku-4*", 1_000_000, 5_000_000),
    ModelPrice::new("claude-3-5-haiku*", 800_000, 4_000_000),
    ModelPrice::new("claude-3-haiku*", 250_000, 1_250_000),
];

impl ProviderAdapter for AnthropicAdapter {
    fn id(&self) -> &'static str {
        "anthropic"
    }

    fn prices(&self) -> &[ModelPrice] {
        PRICES
    }

    fn display_name(&self) -> &'static str {
        "Anthropic"
    }

    fn wire_format(&self, _config: &ConnectionConfig) -> Option<vkdg_operations::WireFormat> {
        Some(vkdg_operations::WireFormat::AnthropicMessages)
    }

    fn meta(&self) -> vkdg_provider_sdk::ProviderMeta {
        vkdg_provider_sdk::ProviderMeta {
            icon_char: 'A',
            icon_color: "#D97706",
            category: vkdg_provider_sdk::ProviderCategory::LlmApi,
            site_url: Some("https://console.anthropic.com"),
            description: Some("Anthropic Claude — safety-focused frontier models."),
        }
    }

    fn prepare(
        &self,
        operation: &Operation,
        config: &ConnectionConfig,
        credential: &Credential,
    ) -> Result<PreparedRequest, ProviderError> {
        let token = credential.token.as_str();
        let Operation::Conversation(req) = operation else {
            return Err(ProviderError::UnsupportedOperation);
        };

        let body = build_body(req);
        let url = format!("{}/v1/messages", base_url(config));

        let mut headers = HeaderMap::new();
        headers.insert(
            "x-api-key",
            token
                .parse()
                .unwrap_or_else(|_| http::HeaderValue::from_static("invalid")),
        );
        headers.insert(
            "anthropic-version",
            http::HeaderValue::from_static("2023-06-01"),
        );
        headers.insert(
            http::header::CONTENT_TYPE,
            http::HeaderValue::from_static("application/json"),
        );
        // Interleaved thinking requires the beta header so Anthropic routes
        // thinking blocks back alongside text blocks.
        if req.thinking.is_some() {
            headers.insert(
                "anthropic-beta",
                http::HeaderValue::from_static("interleaved-thinking-2025-05-14"),
            );
        }

        Ok(PreparedRequest {
            url,
            headers,
            body,
            is_streaming: req.stream,
        })
    }
}

fn base_url(config: &ConnectionConfig) -> String {
    match &config.provider {
        ProviderKind::Anthropic | ProviderKind::Plugin { .. } => "https://api.anthropic.com".into(),
        ProviderKind::OpenAI => "https://api.openai.com".into(),
        ProviderKind::Google => "https://generativelanguage.googleapis.com".into(),
        ProviderKind::Custom { base_url } | ProviderKind::AnthropicCompat { base_url } => {
            base_url.clone()
        }
    }
}

fn build_body(req: &ConversationRequest) -> Bytes {
    let model = vkdg_provider_sdk::upstream_model(req, "claude-3-5-sonnet-20241022");
    vkdg_provider_sdk::anthropic_messages::messages_body(req, model, None)
}

#[cfg(test)]
mod body_tests {
    use super::*;
    use serde_json::{json, Value};
    use vkdg_operations::{ContentBlock, Message, MessageContent, Role, ToolChoice};

    fn body(req: &ConversationRequest) -> Value {
        serde_json::from_slice(&build_body(req)).unwrap()
    }

    // Defeat: a route pattern (`claude-*`) sent upstream as the model id, or a
    // missing max_tokens (Anthropic answers 400 "max_tokens: Field required" to
    // an OpenAI client that omitted it).
    #[test]
    fn glob_model_falls_back_and_max_tokens_is_always_present() {
        let req = ConversationRequest {
            model: "claude-*".into(),
            messages: vec![Message {
                role: Role::User,
                content: MessageContent::Text("hi".into()),
            }],
            ..Default::default()
        };
        let v = body(&req);
        assert_eq!(v["model"], "claude-3-5-sonnet-20241022");
        assert_eq!(v["max_tokens"], 8192);
        assert!(v["system"].is_null(), "no preamble for the API-key adapter");
    }

    // Defeat: any one of thinking+forced-tool (400), results split across user
    // turns (400/ambiguity), or the client's temperature lost with thinking gone.
    #[test]
    fn an_openai_style_agent_turn_becomes_a_valid_messages_request() {
        let req = ConversationRequest {
            model: "claude-sonnet-4-5".into(),
            messages: vec![
                Message {
                    role: Role::User,
                    content: MessageContent::Text("go".into()),
                },
                Message {
                    role: Role::Assistant,
                    content: MessageContent::Blocks(vec![
                        ContentBlock::ToolUse {
                            id: "call_1".into(),
                            name: "a".into(),
                            input: json!({}),
                            cache_control: None,
                        },
                        ContentBlock::ToolUse {
                            id: "call_2".into(),
                            name: "b".into(),
                            input: json!({}),
                            cache_control: None,
                        },
                    ]),
                },
                Message {
                    role: Role::Tool,
                    content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                        tool_use_id: "call_1".into(),
                        content: "one".into(),
                        images: vec![],
                        is_error: false,
                        cache_control: None,
                    }]),
                },
                Message {
                    role: Role::Tool,
                    content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                        tool_use_id: "call_2".into(),
                        content: "two".into(),
                        images: vec![],
                        is_error: false,
                        cache_control: None,
                    }]),
                },
            ],
            tools: vec![vkdg_operations::Tool {
                name: "a".into(),
                description: None,
                input_schema: json!({"type": "object"}),
                cache_control: None,
            }],
            tool_choice: Some(ToolChoice::Required),
            thinking: Some(vkdg_operations::ThinkingRequest {
                budget_tokens: None,
                effort: Some("high".into()),
            }),
            temperature: Some(0.2),
            ..Default::default()
        };
        let v = body(&req);
        assert_eq!(v["tool_choice"], json!({"type": "any"}));
        assert!(v.get("thinking").is_none());
        assert_eq!(v["temperature"], json!(0.2));
        assert_eq!(v["messages"].as_array().unwrap().len(), 3);
        assert_eq!(v["messages"][2]["content"][1]["tool_use_id"], "call_2");
    }
}

#[cfg(test)]
mod price_tests {
    use super::*;

    // Clients send dotted names; a miss here bills Opus 4.5 at 3x.
    #[test]
    fn list_prices_for_dotted_dashed_and_dated_names() {
        let p = |m: &str| {
            vkdg_core::pricing::price_for(PRICES, m)
                .map(|p| (p.input_per_mtok / 1_000, p.output_per_mtok / 1_000))
        };
        assert_eq!(p("claude-opus-4.5"), Some((5_000, 25_000)));
        assert_eq!(p("claude-opus-4-5-20251101"), Some((5_000, 25_000)));
        assert_eq!(p("claude-opus-4-20250514"), Some((15_000, 75_000)));
        assert_eq!(p("claude-opus-4-1"), Some((15_000, 75_000)));
        assert_eq!(p("claude-sonnet-4.5"), Some((3_000, 15_000)));
        assert_eq!(p("claude-haiku-4.5"), Some((1_000, 5_000)));
        assert_eq!(p("claude-3-5-haiku-20241022"), Some((800, 4_000)));
        assert_eq!(p("gpt-4o"), None);
    }
}
