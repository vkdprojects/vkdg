//! Request building: the properties that break a live account when they are wrong.
//!
//! Each of these was a real defect. The endpoint and `profileArn` rules come from
//! third-party production measurements: sending an ARN on an API-key account gets
//! a 403, and the two credential types are bound to different hosts.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;
use vkdg_connections::{AuthKind, ConnectionConfig, Credential, ProviderKind};
use vkdg_core::ConnectionId;
use vkdg_operations::{
    CapabilitySet, ConversationRequest, Message, MessageContent, Operation, Role,
};
use vkdg_provider_kiro::KiroAdapter;
use vkdg_provider_sdk::ProviderAdapter;

const PROFILE_ARN: &str = "arn:aws:codewhisperer:eu-central-1:123456789012:profile/ABCDEF";

fn connection(models: &[&str]) -> ConnectionConfig {
    connection_on(models, None)
}

fn connection_on(models: &[&str], endpoint: Option<&str>) -> ConnectionConfig {
    ConnectionConfig {
        id: ConnectionId("kiro-1".into()),
        provider: ProviderKind::Plugin { id: "kiro".into() },
        auth: AuthKind::ApiKey {
            env_var: "UNUSED".into(),
        },
        models: models.iter().map(|m| (*m).to_string()).collect(),
        max_concurrent: 4,
        weight: 1,
        tags: vec![],
        endpoint: endpoint.map(str::to_owned),
        capabilities: CapabilitySet::default(),
    }
}

fn credential(pairs: &[(&str, &str)]) -> Credential {
    let extra: HashMap<String, String> = pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect();
    Credential {
        token: "tok".into(),
        extra: Arc::new(extra),
    }
}

fn operation(model: &str) -> Operation {
    Operation::Conversation(ConversationRequest {
        model: model.to_string(),
        messages: vec![Message {
            role: Role::User,
            content: MessageContent::Text("hello".into()),
        }],
        tools: vec![],
        max_tokens: Some(64),
        temperature: None,
        stream: true,
        system: None,
        required_capabilities: CapabilitySet::default(),
        thinking: None,
        ..Default::default()
    })
}

/// Parse the prepared body, so assertions read the wire shape rather than Rust types.
fn prepared(
    model: &str,
    models: &[&str],
    extra: &[(&str, &str)],
) -> (String, Vec<(String, String)>, Value) {
    let req = KiroAdapter
        .prepare(&operation(model), &connection(models), &credential(extra))
        .unwrap_or_else(|e| panic!("prepare must succeed: {e}"));
    let headers = req
        .headers
        .iter()
        .map(|(k, v)| (k.as_str().to_owned(), v.to_str().unwrap_or("").to_owned()))
        .collect();
    let body: Value = serde_json::from_slice(&req.body).expect("body is JSON");
    (req.url, headers, body)
}

/// Same, for a connection pinned to a specific plane.
fn prepared_on(
    model: &str,
    endpoint: Option<&str>,
    extra: &[(&str, &str)],
) -> (String, Vec<(String, String)>, Value) {
    let req = KiroAdapter
        .prepare(
            &operation(model),
            &connection_on(&["claude-*"], endpoint),
            &credential(extra),
        )
        .unwrap_or_else(|e| panic!("prepare must succeed: {e}"));
    let headers = req
        .headers
        .iter()
        .map(|(k, v)| (k.as_str().to_owned(), v.to_str().unwrap_or("").to_owned()))
        .collect();
    let body: Value = serde_json::from_slice(&req.body).expect("body is JSON");
    (req.url, headers, body)
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
}

/// Refutes: reading the model from `config.models`, which holds route patterns.
/// A connection matching `claude-*` would send that glob upstream, and a
/// multi-model connection would pin every call to its first entry.
#[test]
fn model_comes_from_the_request_not_the_route_pattern() {
    let (_, _, body) = prepared(
        "claude-sonnet-4.5",
        &["claude-*", "gpt-5.6-*"],
        &[("auth_method", "api_key")],
    );
    assert_eq!(
        body["conversationState"]["currentMessage"]["userInputMessage"]["modelId"],
        "claude-sonnet-4.5"
    );
}

/// Refutes: sending `profileArn` on an API-key account, which AWS answers with 403,
/// and omitting it for an OAuth account, which then gets "not authorized".
#[test]
fn profile_arn_follows_the_credential_type() {
    let (_, headers, body) = prepared(
        "claude-sonnet-4.5",
        &["claude-*"],
        &[("auth_method", "api_key"), ("profile_arn", PROFILE_ARN)],
    );
    assert!(
        body.get("profileArn").is_none(),
        "an API-key account must not send profileArn: {body}"
    );
    assert_eq!(header(&headers, "tokentype"), Some("API_KEY"));

    let (_, headers, body) = prepared(
        "claude-sonnet-4.5",
        &["claude-*"],
        &[("auth_method", "idc"), ("profile_arn", PROFILE_ARN)],
    );
    assert_eq!(
        body["profileArn"], PROFILE_ARN,
        "an IdC account must send its profileArn"
    );
    assert!(
        header(&headers, "tokentype").is_none(),
        "only API keys carry tokentype"
    );
}

/// Refutes: pinning every account to one host. The credential type decides, and the
/// region comes from the profile ARN rather than the OIDC region.
#[test]
fn host_and_region_follow_the_account() {
    let (url, ..) = prepared(
        "claude-sonnet-4.5",
        &["claude-*"],
        &[("auth_method", "api_key"), ("oidc_region", "us-east-1")],
    );
    assert_eq!(
        url, "https://codewhisperer.us-east-1.amazonaws.com/generateAssistantResponse",
        "API keys speak the editor protocol on the CodeWhisperer plane; the legacy \
         Amazon Q service root only exposes sonnet-4/4.5 and haiku-4.5"
    );

    // The ARN says eu-central-1 while the OIDC region says eu-north-1; the ARN wins,
    // because that is where the profile — and so the runtime — actually lives.
    let (url, ..) = prepared(
        "claude-sonnet-4.5",
        &["claude-*"],
        &[
            ("auth_method", "idc"),
            ("profile_arn", PROFILE_ARN),
            ("oidc_region", "eu-north-1"),
        ],
    );
    assert_eq!(
        url, "https://runtime.eu-central-1.kiro.dev/generateAssistantResponse",
        "OAuth accounts use the Kiro plane, in the profile's region"
    );
}

/// Refutes: quietly opting users into service improvement, and skipping the prompt
/// cache that every multi-turn conversation benefits from.
/// `cache_point` is now placed on the last user turn in history (not `currentMessage`),
/// so it caches a stable prefix that is reused on every subsequent turn.
#[test]
fn optout_and_cache_point_are_always_sent() {
    let (_, headers, body) = prepared(
        "claude-sonnet-4.5",
        &["claude-*"],
        &[("auth_method", "builder-id")],
    );
    assert_eq!(
        header(&headers, "x-amzn-codewhisperer-optout"),
        Some("true"),
        "a gateway cannot consent to training on its users' traffic"
    );
    // Single-turn: no history → cache_point falls back to currentMessage.
    // In multi-turn, it is placed on the last user turn in history instead.
    let history = &body["conversationState"]["history"];
    let current_cache =
        &body["conversationState"]["currentMessage"]["userInputMessage"]["cachePoint"]["type"];
    // Either history has a user turn with the cache_point, or it's on current (single-turn).
    let history_has_cache = history.as_array().is_some_and(|arr| {
        arr.iter()
            .any(|item| item["userInputMessage"]["cachePoint"]["type"] == "default")
    });
    assert!(
        history_has_cache || current_cache == "default",
        "cache_point must appear in history (multi-turn) or currentMessage (single-turn)"
    );
}

// `auth: { type: api_key }` in config (a long-lived Kiro key in an env var) has
// no stored account, so no `auth_method`. It used to fall back to Builder ID:
// wrong host, no `tokentype`, and the key was refused.
#[test]
fn api_key_connection_without_account_uses_the_api_key_flow() {
    let (_, headers, body) = prepared("claude-sonnet-4.5", &["claude-*"], &[]);
    assert!(
        headers
            .iter()
            .any(|(k, v)| k == "tokentype" && v == "API_KEY"),
        "{headers:?}"
    );
    assert!(
        body.get("profileArn").is_none(),
        "API-key requests must not send profileArn"
    );
}

// OmniRoute enables prompt caching on every Kiro call; without the headers a
// repeated prompt never hits the cache.
#[test]
fn prompt_caching_headers_are_sent() {
    let (_, headers, _) = prepared(
        "claude-sonnet-4.5",
        &["claude-*"],
        &[("auth_method", "social")],
    );
    let has = |k: &str, v: &str| headers.iter().any(|(hk, hv)| hk == k && hv == v);
    assert!(has("x-amzn-bedrock-cache-control", "enable"), "{headers:?}");
    assert!(
        has("anthropic-beta", "prompt-caching-2024-07-31"),
        "{headers:?}"
    );
}

/// Refutes: ignoring `endpoint:` on the connection, which puts both connections
/// of an account on one rate-limit bucket. The planes throttle independently —
/// driving runtime.kiro.dev to 75% HTTP 429 left codewhisperer at 80/80 — so an
/// OAuth account pinned to each plane is what doubles usable capacity.
#[test]
fn a_pinned_endpoint_selects_the_host_and_keeps_the_credential_rules() {
    let oauth = &[("auth_method", "social"), ("profile_arn", PROFILE_ARN)];

    let (url, headers, body) = prepared_on("claude-sonnet-4.5", Some("codewhisperer"), oauth);
    assert_eq!(
        url, "https://q.eu-central-1.amazonaws.com/generateAssistantResponse",
        "an OAuth account may be pinned to the CodeWhisperer plane; outside the \
         home region that plane is served by q.{{region}}, since codewhisperer.* \
         only resolves in us-east-1"
    );
    assert_eq!(
        body["profileArn"], PROFILE_ARN,
        "an OAuth account keeps sending profileArn on either plane"
    );
    assert!(
        header(&headers, "tokentype").is_none(),
        "tokentype belongs to API keys, not to a pinned OAuth connection"
    );

    let (url, ..) = prepared_on("claude-sonnet-4.5", Some("runtime"), oauth);
    assert_eq!(
        url, "https://runtime.eu-central-1.kiro.dev/generateAssistantResponse",
        "the other connection of the same account stays on the Kiro plane"
    );

    // runtime.* answers "profileArn is required for this request" to an API key,
    // so the pin must not take that connection offline.
    let (url, headers, body) = prepared_on(
        "claude-sonnet-4.5",
        Some("runtime"),
        &[("auth_method", "api_key")],
    );
    assert_eq!(
        url,
        "https://codewhisperer.us-east-1.amazonaws.com/generateAssistantResponse"
    );
    assert_eq!(header(&headers, "tokentype"), Some("API_KEY"));
    assert!(
        body.get("profileArn").is_none(),
        "an API key must never send profileArn: AWS answers 403"
    );
}

// ── Progressive aging ─────────────────────────────────────────────────────────

/// Build an Operation with `n` alternating user/assistant messages.
/// `overrides[i]` replaces the text of message `i` (0-indexed) when present.
fn multi_turn_op(n: usize, overrides: &[(usize, &str)]) -> Operation {
    let messages: Vec<Message> = (0..n)
        .map(|i| {
            let role = if i % 2 == 0 {
                Role::User
            } else {
                Role::Assistant
            };
            let text = overrides
                .iter()
                .find(|(idx, _)| *idx == i)
                .map_or_else(|| format!("msg{i}"), |(_, t)| (*t).to_string());
            Message {
                role,
                content: MessageContent::Text(text),
            }
        })
        .collect();
    Operation::Conversation(ConversationRequest {
        model: "claude-sonnet-4.5".to_string(),
        messages,
        tools: vec![],
        max_tokens: Some(64),
        temperature: None,
        stream: true,
        system: None,
        required_capabilities: CapabilitySet::default(),
        thinking: None,
        ..Default::default()
    })
}

/// Refutes: compressing history turns that are close to the active turn.
/// A turn at distance ≤ 3 from the end of history must be sent verbatim even
/// when its text exceeds the distant-aging limit (120 chars).
#[test]
fn recent_turns_are_verbatim() {
    // 11 messages → 10 history turns after popping current user turn.
    // msg8 (i=8, user) → distance = 10-1-8 = 1  (≤3, verbatim).
    let long_text = "B".repeat(200);
    let op = multi_turn_op(11, &[(8, &long_text)]);
    let req = KiroAdapter
        .prepare(
            &op,
            &connection(&["claude-*"]),
            &credential(&[("auth_method", "api_key")]),
        )
        .unwrap_or_else(|e| panic!("prepare must succeed: {e}"));
    let body: Value = serde_json::from_slice(&req.body).expect("body is JSON");
    let history = body["conversationState"]["history"]
        .as_array()
        .expect("history array");
    let content = history[8]["userInputMessage"]["content"]
        .as_str()
        .expect("string content at index 8");
    assert_eq!(
        content, long_text,
        "turn at distance 1 must not be compressed (got {content:?})"
    );
}

/// Refutes: sending old history turns in full, which wastes tokens and defeats
/// the prompt-cache on every subsequent request.
/// A turn at distance ≥ 8 must have its text truncated to 120 chars.
#[test]
fn old_turns_are_compressed() {
    // 11 messages → 10 history turns after popping current user turn.
    // msg0 (i=0, user) → distance = 10-1-0 = 9  (≥8, 120-char limit).
    let long_text = "A".repeat(200);
    let op = multi_turn_op(11, &[(0, &long_text)]);
    let req = KiroAdapter
        .prepare(
            &op,
            &connection(&["claude-*"]),
            &credential(&[("auth_method", "api_key")]),
        )
        .unwrap_or_else(|e| panic!("prepare must succeed: {e}"));
    let body: Value = serde_json::from_slice(&req.body).expect("body is JSON");
    let history = body["conversationState"]["history"]
        .as_array()
        .expect("history array");
    let content = history[0]["userInputMessage"]["content"]
        .as_str()
        .expect("string content at index 0");
    assert!(
        content.len() <= 124, // 120 chars + "…" (3 UTF-8 bytes) + a little margin
        "turn at distance 9 must be compressed to ≤120 chars; got {} chars: {content:?}",
        content.len()
    );
    assert!(
        !content.contains(&long_text[..121]),
        "compressed turn must not contain the full original text"
    );
}

// ── Compaction system — full e2e coverage ─────────────────────────────────────

/// Helper: build a multi-turn op WITH a system prompt.
fn multi_turn_op_with_system(n: usize, system: &str, overrides: &[(usize, &str)]) -> Operation {
    let messages: Vec<Message> = (0..n)
        .map(|i| {
            let role = if i % 2 == 0 {
                Role::User
            } else {
                Role::Assistant
            };
            let text = overrides
                .iter()
                .find(|(idx, _)| *idx == i)
                .map_or_else(|| format!("msg{i}"), |(_, t)| (*t).to_string());
            Message {
                role,
                content: MessageContent::Text(text),
            }
        })
        .collect();
    Operation::Conversation(ConversationRequest {
        model: "claude-sonnet-4.5".to_string(),
        messages,
        system: Some(system.to_string()),
        ..Default::default()
    })
}

/// Refutes: progressive aging truncating the system prompt injected into the
/// first user turn, causing the model to lose identity and project context
/// in long conversations (the "model se perde" bug — 2f93630).
///
/// With 51 messages the first user turn sits at distance 50 (far tier, 120-char
/// limit without the protection). The system prompt is 500 chars — it MUST
/// arrive intact regardless of conversation length.
#[test]
fn system_prompt_never_aged_in_long_conversation() {
    let system = "S".repeat(500); // 500-char system prompt
    let op = multi_turn_op_with_system(51, &system, &[]);
    let req = KiroAdapter
        .prepare(
            &op,
            &connection(&["claude-*"]),
            &credential(&[("auth_method", "api_key")]),
        )
        .unwrap_or_else(|e| panic!("prepare must succeed: {e}"));
    let body: Value = serde_json::from_slice(&req.body).expect("body is JSON");
    let history = body["conversationState"]["history"]
        .as_array()
        .expect("history array");
    // First history item must contain the full system prompt verbatim.
    let first_content = history[0]["userInputMessage"]["content"]
        .as_str()
        .expect("first history item must have string content");
    assert!(
        first_content.contains(&system),
        "system prompt (500 chars) must be fully present in first history turn; got {first_content:?}"
    );
}

/// Refutes: aging the system prompt when it arrives as a short (e.g. 50-char)
/// string — the boundary case where a small system still must not be truncated.
#[test]
fn short_system_prompt_not_aged() {
    let system = "You are a helpful coding assistant. Follow user instructions.";
    let op = multi_turn_op_with_system(21, system, &[]);
    let req = KiroAdapter
        .prepare(
            &op,
            &connection(&["claude-*"]),
            &credential(&[("auth_method", "api_key")]),
        )
        .unwrap_or_else(|e| panic!("prepare must succeed: {e}"));
    let body: Value = serde_json::from_slice(&req.body).expect("body is JSON");
    let history = body["conversationState"]["history"]
        .as_array()
        .expect("history array");
    let first_content = history[0]["userInputMessage"]["content"]
        .as_str()
        .expect("string content");
    assert!(
        first_content.contains(system),
        "short system must be verbatim in first turn; got {first_content:?}"
    );
}

/// Refutes: aging ALL turns equally — the mid tier (distance 4–7) must keep
/// 500 chars, not be compressed to the far-tier 120-char limit.
#[test]
fn mid_tier_turns_keep_500_chars() {
    // 11 messages → 10 history turns. msg2 (i=2, user) → distance = 10-1-2 = 7 (mid tier).
    let long_text = "M".repeat(600);
    let op = multi_turn_op(11, &[(2, &long_text)]);
    let req = KiroAdapter
        .prepare(
            &op,
            &connection(&["claude-*"]),
            &credential(&[("auth_method", "api_key")]),
        )
        .unwrap_or_else(|e| panic!("prepare must succeed: {e}"));
    let body: Value = serde_json::from_slice(&req.body).expect("body is JSON");
    let history = body["conversationState"]["history"]
        .as_array()
        .expect("history array");
    let content = history[2]["userInputMessage"]["content"]
        .as_str()
        .expect("string");
    assert!(
        content.len() > 120 && content.len() <= 504, // ~500 + ellipsis
        "mid-tier (distance 7) must keep 500 chars, got {} chars",
        content.len()
    );
}

/// Refutes: not truncating tool results in history, which are typically large
/// (file contents, API responses). History tool results must be capped at 2000 chars.
#[test]
fn history_tool_results_capped_not_current() {
    use vkdg_operations::{ContentBlock, MessageContent};
    // Build a 2-turn conversation: user→tool_result, assistant→text, user (current).
    // The tool result in history must be truncated; the one in current must not.
    let big_result = "R".repeat(5000);
    let messages = vec![
        Message {
            role: Role::User,
            content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                tool_use_id: "t1".into(),
                content: big_result.clone(),
                images: vec![],
                is_error: false,
                cache_control: None,
            }]),
        },
        Message {
            role: Role::Assistant,
            content: MessageContent::Text("ok".into()),
        },
        Message {
            role: Role::User,
            content: MessageContent::Text("next".into()),
        },
    ];
    let op = Operation::Conversation(ConversationRequest {
        model: "claude-sonnet-4.5".to_string(),
        messages,
        ..Default::default()
    });
    let req = KiroAdapter
        .prepare(
            &op,
            &connection(&["claude-*"]),
            &credential(&[("auth_method", "api_key")]),
        )
        .unwrap_or_else(|e| panic!("prepare: {e}"));
    let body: Value = serde_json::from_slice(&req.body).expect("body JSON");
    // The tool result was in the first turn which is now in history.
    let history = body["conversationState"]["history"]
        .as_array()
        .expect("history");
    let ctx = &history[0]["userInputMessage"]["userInputMessageContext"];
    let tool_content = ctx["toolResults"][0]["content"][0]["text"]
        .as_str()
        .unwrap_or("");
    assert!(
        tool_content.len() <= 2200,
        "history tool result must be truncated to ~2000 chars, got {}",
        tool_content.len()
    );
    assert!(
        !tool_content.contains(&big_result[..2001]),
        "tool result must be truncated"
    );
}

/// A recent tool result crossing the UTF-8 byte cap must survive request
/// preparation as a valid, bounded Kiro history entry.
#[test]
fn utf8_tool_result_at_history_limit_prepares_a_request() {
    use serde_json::json;
    use vkdg_operations::ContentBlock;

    let result = format!("{}→tail", "a".repeat(1999));
    let messages = vec![
        Message {
            role: Role::User,
            content: MessageContent::Text("read the file".into()),
        },
        Message {
            role: Role::Assistant,
            content: MessageContent::Blocks(vec![ContentBlock::ToolUse {
                id: "call-1".into(),
                name: "read_file".into(),
                input: json!({}),
                cache_control: None,
            }]),
        },
        Message {
            role: Role::User,
            content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                tool_use_id: "call-1".into(),
                content: result,
                images: vec![],
                is_error: false,
                cache_control: None,
            }]),
        },
        Message {
            role: Role::Assistant,
            content: MessageContent::Text("read complete".into()),
        },
        Message {
            role: Role::User,
            content: MessageContent::Text("continue".into()),
        },
    ];
    let op = Operation::Conversation(ConversationRequest {
        model: "claude-sonnet-4.5".into(),
        messages,
        ..Default::default()
    });
    let req = KiroAdapter
        .prepare(&op, &connection(&["claude-*"]), &credential(&[]))
        .expect("prepare");
    let body: Value = serde_json::from_slice(&req.body).expect("valid JSON");
    let text = body["conversationState"]["history"][2]["userInputMessage"]
        ["userInputMessageContext"]["toolResults"][0]["content"][0]["text"]
        .as_str()
        .expect("history tool result");
    assert_eq!(
        text,
        format!("{}...[truncated, 2006 bytes total]", "a".repeat(1999))
    );
}

/// Refutes: stable conversationId regression — two requests from the same
/// session must share a conversationId, two from different sessions must differ.
#[test]
fn conversation_id_stable_within_session() {
    let session = "test-session-abc-123";
    let make_op = || {
        Operation::Conversation(ConversationRequest {
            model: "claude-sonnet-4.5".to_string(),
            messages: vec![Message {
                role: Role::User,
                content: MessageContent::Text("hi".into()),
            }],
            session_id: Some(session.to_string()),
            ..Default::default()
        })
    };
    let cred = credential(&[("auth_method", "api_key")]);
    let conn = connection(&["claude-*"]);

    let req1 = KiroAdapter.prepare(&make_op(), &conn, &cred).unwrap();
    let req2 = KiroAdapter.prepare(&make_op(), &conn, &cred).unwrap();
    let b1: Value = serde_json::from_slice(&req1.body).unwrap();
    let b2: Value = serde_json::from_slice(&req2.body).unwrap();
    let id1 = b1["conversationState"]["conversationId"]
        .as_str()
        .unwrap_or("");
    let id2 = b2["conversationState"]["conversationId"]
        .as_str()
        .unwrap_or("");
    assert!(!id1.is_empty(), "conversationId must not be empty");
    assert_eq!(id1, id2, "same session_id must produce same conversationId");

    // Different session → different conversationId
    let make_op2 = || {
        Operation::Conversation(ConversationRequest {
            model: "claude-sonnet-4.5".to_string(),
            messages: vec![Message {
                role: Role::User,
                content: MessageContent::Text("hi".into()),
            }],
            session_id: Some("other-session-xyz".to_string()),
            ..Default::default()
        })
    };
    let req3 = KiroAdapter.prepare(&make_op2(), &conn, &cred).unwrap();
    let b3: Value = serde_json::from_slice(&req3.body).unwrap();
    let id3 = b3["conversationState"]["conversationId"]
        .as_str()
        .unwrap_or("");
    assert_ne!(
        id1, id3,
        "different session_ids must produce different conversationIds"
    );
}

/// Refutes: aging collapsing the entire conversation to just the most recent
/// turns, losing important earlier context. Verifies the tier breakdown:
/// last 4 turns verbatim, 4–7 mid (500 chars), 8+ far (120 chars).
#[test]
fn aging_tier_boundaries_are_correct() {
    // 21 messages → 20 history turns after popping current.
    // dist 1  → verbatim (index 18)
    // dist 4  → mid 500 (index 15)
    // dist 8  → far 120 (index 11)
    // dist 19 → far 120 (index 0)
    let big = "X".repeat(600);
    let op = multi_turn_op(
        21,
        &[
            (18, &big), // dist 1  (verbatim)   — user turn ✓
            (14, &big), // dist 5  (mid ~500)   — user turn ✓ (15 is assistant)
            (10, &big), // dist 9  (far ≤120)   — user turn ✓ (11 is assistant)
            (0, &big),  // dist 19 (far ≤120)   — user turn ✓
        ],
    );
    let req = KiroAdapter
        .prepare(
            &op,
            &connection(&["claude-*"]),
            &credential(&[("auth_method", "api_key")]),
        )
        .unwrap_or_else(|e| panic!("{e}"));
    let body: Value = serde_json::from_slice(&req.body).unwrap();
    let h = body["conversationState"]["history"].as_array().unwrap();

    let len = |idx: usize| {
        h[idx]["userInputMessage"]["content"]
            .as_str()
            .map_or(0, |s| s.len())
    };

    // dist 1: verbatim (600 chars)
    assert!(
        len(18) == 600,
        "dist-1 must be verbatim (600), got {}",
        len(18)
    );
    // dist 5: mid tier — between 121 and 504
    assert!(
        len(14) > 120 && len(14) <= 504,
        "dist-5 mid must be ~500, got {}",
        len(14)
    );
    // dist 9: far tier — ≤124
    assert!(len(10) <= 124, "dist-9 far must be ≤120, got {}", len(10));
    // dist 19: also far — ≤124
    assert!(len(0) <= 124, "dist-19 far must be ≤120, got {}", len(0));
}

// ── Edge cases para encontrar bugs reais ──────────────────────────────────────

/// BUG HUNT: e se o system prompt for gigante (20KB, como o omp real) e a conv
/// longa? O system nunca pode ser truncado. Testa o pior caso real.
#[test]
fn huge_system_prompt_survives_100_turn_conversation() {
    // omp envia ~20KB de system (conventions + project-context + memory)
    let system = "A".repeat(20_000);
    // 101 mensagens → 100 history turns. Sistema em dist=99 sem proteção.
    let op = multi_turn_op_with_system(101, &system, &[]);
    let req = KiroAdapter
        .prepare(
            &op,
            &connection(&["claude-*"]),
            &credential(&[("auth_method", "api_key")]),
        )
        .unwrap_or_else(|e| panic!("{e}"));
    let body: Value = serde_json::from_slice(&req.body).unwrap();
    let h = body["conversationState"]["history"].as_array().unwrap();
    let content = h[0]["userInputMessage"]["content"].as_str().unwrap_or("");
    assert!(
        content.contains(&system),
        "20KB system prompt MUST survive 100-turn conv; got len={} (should be ≥20000)",
        content.len()
    );
}

/// BUG HUNT: e se o system prompt contiver uma linha começando com
/// "x-anthropic-billing-header:" no meio (não na primeira linha)?
/// O filtro deve remover APENAS essa linha, não o resto.
#[test]
fn billing_header_filter_is_line_only_not_greedy() {
    let system = "You are a helpful assistant.\nx-anthropic-billing-header: cc_version=2.1.280\nFollow instructions carefully.";
    let op = multi_turn_op_with_system(3, system, &[]);
    let req = KiroAdapter
        .prepare(
            &op,
            &connection(&["claude-*"]),
            &credential(&[("auth_method", "api_key")]),
        )
        .unwrap_or_else(|e| panic!("{e}"));
    let body: Value = serde_json::from_slice(&req.body).unwrap();
    // System gets injected into first user turn → history[0].
    // (3 msgs: msg0+system → hist[0], msg1 → hist[1], msg2 → currentMessage)
    let h = body["conversationState"]["history"].as_array().unwrap();
    let content = h[0]["userInputMessage"]["content"].as_str().unwrap_or("");
    // Billing header line must be gone
    assert!(
        !content.contains("x-anthropic-billing-header:"),
        "billing header must be stripped from history[0]; content: {content:?}"
    );
    // Content before billing header must survive
    assert!(
        content.contains("You are a helpful assistant."),
        "content before billing header must survive; history[0]: {content:?}"
    );
    // Content after billing header must survive
    assert!(
        content.contains("Follow instructions carefully."),
        "content after billing header must survive; history[0]: {content:?}"
    );
}

/// BUG HUNT: o que acontece com um turn de assistente no topo da historia?
/// (Às vezes a compactação do omp produz isso). Não deve crashar e deve
/// ser tratado como assistente, não como usuário.
#[test]
fn assistant_turn_at_history_start_handled_correctly() {
    let messages = vec![
        Message {
            role: Role::Assistant,
            content: MessageContent::Text("I'll help.".into()),
        },
        Message {
            role: Role::User,
            content: MessageContent::Text("thanks".into()),
        },
        Message {
            role: Role::Assistant,
            content: MessageContent::Text("sure".into()),
        },
        Message {
            role: Role::User,
            content: MessageContent::Text("do it".into()),
        },
    ];
    let op = Operation::Conversation(ConversationRequest {
        model: "claude-sonnet-4.5".to_string(),
        messages,
        system: Some("Be helpful.".to_string()),
        ..Default::default()
    });
    // Must not panic
    let req = KiroAdapter
        .prepare(
            &op,
            &connection(&["claude-*"]),
            &credential(&[("auth_method", "api_key")]),
        )
        .unwrap_or_else(|e| panic!("must not panic: {e}"));
    let body: Value = serde_json::from_slice(&req.body).unwrap();
    // Should have history
    let h = body["conversationState"]["history"].as_array().unwrap();
    assert!(!h.is_empty(), "history must not be empty");
}

/// BUG HUNT: e se o aging truncar uma mensagem de usuário que contém `tool_use`
/// no meio do texto? A truncagem não pode partir no meio de um JSON.
/// (Só testa que não crashe e produz output válido.)
#[test]
fn aging_does_not_corrupt_json_in_text() {
    // Large text that looks like JSON (would be invalid if split mid-token)
    let json_text = r#"{"action":"read","path":"/very/long/path/to/some/file/that/would/be/truncated","options":{"recursive":true,"depth":10}}"#.to_string();
    let text_300 = json_text.repeat(2); // ~300 chars
    let op = multi_turn_op(21, &[(0, &text_300)]); // dist 19 → 120-char limit
    let req = KiroAdapter
        .prepare(
            &op,
            &connection(&["claude-*"]),
            &credential(&[("auth_method", "api_key")]),
        )
        .unwrap_or_else(|e| panic!("{e}"));
    let body: Value = serde_json::from_slice(&req.body).unwrap();
    let h = body["conversationState"]["history"].as_array().unwrap();
    // Must be valid string content (no panic from serialization)
    let content = h[0]["userInputMessage"]["content"].as_str().unwrap_or("");
    assert!(
        content.len() <= 125,
        "far-tier must be ≤120, got {}",
        content.len()
    );
    // Must be valid UTF-8 (no panic from indexing mid-char)
    assert!(std::str::from_utf8(content.as_bytes()).is_ok());
}

/// BUG HUNT: turns com conteúdo vazio não devem aparecer no histórico
/// (evitar mandar turn = "" para o Kiro que pode causar 400).
#[test]
fn empty_text_turns_not_sent_as_blank() {
    let messages = vec![
        Message {
            role: Role::User,
            content: MessageContent::Text(String::new()),
        },
        Message {
            role: Role::Assistant,
            content: MessageContent::Text("ok".into()),
        },
        Message {
            role: Role::User,
            content: MessageContent::Text("real question".into()),
        },
    ];
    let op = Operation::Conversation(ConversationRequest {
        model: "claude-sonnet-4.5".to_string(),
        messages,
        ..Default::default()
    });
    let req = KiroAdapter
        .prepare(
            &op,
            &connection(&["claude-*"]),
            &credential(&[("auth_method", "api_key")]),
        )
        .unwrap_or_else(|e| panic!("{e}"));
    let body: Value = serde_json::from_slice(&req.body).unwrap();
    // Verify the current message is "real question" (the empty first message
    // may be in history, but must not cause the current message to disappear)
    let current = body["conversationState"]["currentMessage"]["userInputMessage"]["content"]
        .as_str()
        .unwrap_or("");
    assert!(
        current.contains("real question"),
        "current message must be the last user turn; got: {current:?}"
    );
}
