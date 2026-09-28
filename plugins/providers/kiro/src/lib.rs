//! VKDG provider plugin: kiro — Amazon Q Developer / CodeWhisperer.
//!
//! Request shape follows AWS's own Smithy-generated client
//! (`aws/amazon-q-developer-cli`, crate `amzn-codewhisperer-streaming-client`).
//! Which host a request goes to depends on how the account authenticates; see
//! [`endpoint`].

use http::{HeaderMap, HeaderValue};
use serde::Serialize;
use uuid::Uuid;
use vkdg_connections::ConnectionConfig;
use vkdg_operations::Operation;
use vkdg_provider_sdk::{
    ConversationStreamDecoder, Credential, OAuthProvider, PreparedRequest, ProviderAdapter,
    ProviderError,
};

pub mod auth;
pub mod decode;
pub mod endpoint;
pub mod eventstream;
pub mod models;
pub mod region;
mod request;
pub mod stream_decoder;
mod thinking;

use crate::auth::{AUTH_API_KEY, AUTH_BUILDER_ID, AUTH_EXTERNAL_IDP};
use crate::endpoint::EndpointKind;

/// Identifies this gateway upstream.
const USER_AGENT: &str = "vkdg/0.1.0";

pub struct KiroAdapter;

impl ProviderAdapter for KiroAdapter {
    fn id(&self) -> &str {
        "kiro"
    }

    fn display_name(&self) -> &str {
        "Kiro / Amazon Q"
    }

    fn meta(&self) -> vkdg_provider_sdk::ProviderMeta {
        vkdg_provider_sdk::ProviderMeta {
            icon_char: 'K',
            icon_color: "#FF9900",
            category: vkdg_provider_sdk::ProviderCategory::OauthIde,
            site_url: Some("https://kiro.dev"),
            description: Some("Amazon Q / Kiro — AI coding assistant powered by AWS."),
        }
    }

    fn prepare(
        &self,
        operation: &Operation,
        config: &ConnectionConfig,
        credential: &Credential,
    ) -> Result<PreparedRequest, ProviderError> {
        let Operation::Conversation(conv) = operation else {
            return Err(ProviderError::UnsupportedOperation);
        };

        let extra = credential.extra.as_ref();
        // A stored account records which login issued its token. A connection
        // authenticated by `auth: { type: api_key }` (a long-lived Kiro key in an
        // env var) has no account, so it is the API-key flow, not Builder ID.
        let auth_method =
            extra
                .get("auth_method")
                .map(String::as_str)
                .unwrap_or(match config.auth {
                    vkdg_connections::AuthKind::ApiKey { .. } => AUTH_API_KEY,
                    _ => AUTH_BUILDER_ID,
                });
        let profile_arn = extra.get("profile_arn").map(String::as_str);

        // The runtime region lives in the profile ARN; the OIDC region is only a
        // fallback, and only when it can host a profile at all.
        let region =
            region::runtime_region(profile_arn, extra.get("oidc_region").map(String::as_str));
        let kind = EndpointKind::for_auth_method(auth_method);

        // The model the client asked for. The connection's `models` list holds
        // route patterns, so falling back to it would send a glob upstream.
        let model_id = models::resolve_model_id(Some(conv.model.as_str()));

        // Thinking / reasoning, gated on the exact model allowlist from OmniRoute's
        // adaptiveThinking.ts. Sending `output_config` to non-whitelisted models
        // (e.g. claude-sonnet-4.5) causes a Bedrock 400 even though those models
        // support thinking on Anthropic's direct API.
        let additional_fields = conv
            .thinking
            .as_ref()
            .and_then(|t| thinking::build_fields(&model_id, t));

        let body = KiroRequestBody {
            conversation_state: request::build_conversation_state(conv, &model_id, kind.origin()),
            // API-key accounts must not send profileArn: AWS answers 403.
            profile_arn: kind
                .sends_profile_arn()
                .then(|| profile_arn.map(str::to_owned))
                .flatten(),
            additional_fields,
        };

        let mut headers = HeaderMap::new();
        headers.insert(
            http::header::CONTENT_TYPE,
            HeaderValue::from_static(kind.content_type()),
        );
        headers.insert(
            http::header::ACCEPT,
            HeaderValue::from_static("application/json"),
        );
        headers.insert(
            http::header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", credential.token))
                .map_err(|_| ProviderError::Http("credential is not a valid header".into()))?,
        );
        // Operation routing is either the URL path (IDE) or this header, never both.
        if let Some(target) = kind.amz_target() {
            headers.insert("x-amz-target", HeaderValue::from_static(target));
        }
        if kind.sends_api_key_token_type() {
            headers.insert("tokentype", HeaderValue::from_static("API_KEY"));
        }
        if auth_method == AUTH_EXTERNAL_IDP {
            headers.insert("TokenType", HeaderValue::from_static("EXTERNAL_IDP"));
        }
        // Opt out of service improvement: a gateway cannot consent for its users.
        headers.insert(
            "x-amzn-codewhisperer-optout",
            HeaderValue::from_static("true"),
        );
        headers.insert("x-amzn-kiro-agent-mode", HeaderValue::from_static("vibe"));
        headers.insert(
            "amz-sdk-invocation-id",
            HeaderValue::from_str(&Uuid::new_v4().to_string())
                .map_err(|_| ProviderError::Http("invalid invocation id".into()))?,
        );
        // Retries belong to the gateway, so each upstream call is a single attempt.
        headers.insert(
            "amz-sdk-request",
            HeaderValue::from_static("attempt=1; max=1"),
        );
        headers.insert("x-amz-user-agent", HeaderValue::from_static(USER_AGENT));
        headers.insert(
            http::header::USER_AGENT,
            HeaderValue::from_static(USER_AGENT),
        );
        // Prompt caching, as OmniRoute's KiroExecutor sends it. Without these a
        // repeated Claude Code prompt is billed and timed as fresh input.
        headers.insert(
            "x-amzn-bedrock-cache-control",
            HeaderValue::from_static("enable"),
        );
        headers.insert(
            "anthropic-beta",
            HeaderValue::from_static("prompt-caching-2024-07-31"),
        );

        Ok(PreparedRequest {
            // An explicit base_url overrides host selection (private deploys, tests).
            url: match &config.provider {
                vkdg_connections::ProviderKind::Custom { base_url } => base_url.clone(),
                _ => kind.url(&region),
            },
            headers,
            body: serde_json::to_vec(&body)
                .map_err(|e| ProviderError::Serialization(e.to_string()))?
                .into(),
            is_streaming: true,
        })
    }

    fn oauth(&self) -> Option<&dyn OAuthProvider> {
        Some(self)
    }

    fn stream_decoder(&self) -> Option<Box<dyn ConversationStreamDecoder>> {
        Some(Box::new(stream_decoder::KiroStreamDecoder::new()))
    }
}

// ── Kiro request body types ───────────────────────────────────────────────────

#[derive(Serialize)]
struct KiroRequestBody {
    #[serde(rename = "conversationState")]
    conversation_state: request::ConversationState,
    /// Names the Q Developer profile that owns the call. Required for OAuth
    /// accounts; API-key accounts must omit it, or AWS answers 403.
    #[serde(rename = "profileArn", skip_serializing_if = "Option::is_none")]
    profile_arn: Option<String>,
    /// Thinking / reasoning controls, gated on the model allowlist.
    #[serde(
        rename = "additionalModelRequestFields",
        skip_serializing_if = "Option::is_none"
    )]
    additional_fields: Option<thinking::AdditionalFields>,
}
