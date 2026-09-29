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
    assert_eq!(
        body["conversationState"]["currentMessage"]["userInputMessage"]["cachePoint"]["type"],
        "default"
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
