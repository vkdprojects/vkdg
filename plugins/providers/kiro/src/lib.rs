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
use vkdg_operations::{ConversationRequest, Operation};
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
pub mod stream_decoder;

use crate::auth::{AUTH_BUILDER_ID, AUTH_EXTERNAL_IDP};
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
        let auth_method = extra
            .get("auth_method")
            .map(String::as_str)
            .unwrap_or(AUTH_BUILDER_ID);
        let profile_arn = extra.get("profile_arn").map(String::as_str);

        // The runtime region lives in the profile ARN; the OIDC region is only a
        // fallback, and only when it can host a profile at all.
        let region =
            region::runtime_region(profile_arn, extra.get("oidc_region").map(String::as_str));
        let kind = EndpointKind::for_auth_method(auth_method);

        // The model the client asked for. The connection's `models` list holds
        // route patterns, so falling back to it would send a glob upstream.
        let model_id = models::resolve_model_id(Some(conv.model.as_str()));

        let body = KiroRequestBody {
            conversation_state: build_conversation_state(conv, &model_id, kind.origin()),
            // API-key accounts must not send profileArn: AWS answers 403.
            profile_arn: kind
                .sends_profile_arn()
                .then(|| profile_arn.map(str::to_owned))
                .flatten(),
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
    conversation_state: KiroConversationState,
    /// Names the Q Developer profile that owns the call. Required for OAuth
    /// accounts; API-key accounts must omit it, or AWS answers 403.
    #[serde(rename = "profileArn", skip_serializing_if = "Option::is_none")]
    profile_arn: Option<String>,
}

#[derive(Serialize)]
struct KiroConversationState {
    #[serde(rename = "chatTriggerType")]
    chat_trigger_type: String,
    #[serde(rename = "agentTaskType")]
    agent_task_type: String,
    #[serde(rename = "conversationId")]
    conversation_id: String,
    #[serde(rename = "currentMessage")]
    current_message: KiroMessage,
    #[serde(skip_serializing_if = "Option::is_none")]
    history: Option<Vec<KiroHistoryItem>>,
}

#[derive(Serialize)]
#[serde(untagged)]
enum KiroMessage {
    User {
        #[serde(rename = "userInputMessage")]
        user_input_message: KiroUserInput,
    },
}

#[derive(Serialize)]
struct KiroUserInput {
    content: String,
    #[serde(rename = "modelId")]
    model_id: String,
    origin: String,
    /// Asks the service to cache the prompt prefix up to and including this
    /// message. Only `{"type":"default"}` exists today; omitted for models that
    /// do not advertise `supportsPromptCache`.
    #[serde(rename = "cachePoint", skip_serializing_if = "Option::is_none")]
    cache_point: Option<KiroCachePoint>,
}

#[derive(Serialize)]
struct KiroCachePoint {
    #[serde(rename = "type")]
    kind: &'static str,
}

impl KiroCachePoint {
    /// The only cache-point type the service defines.
    fn default_point() -> Self {
        Self { kind: "default" }
    }
}

#[derive(Serialize)]
#[serde(untagged)]
enum KiroHistoryItem {
    User {
        #[serde(rename = "userInputMessage")]
        user_input_message: KiroUserInputContent,
    },
    Assistant {
        #[serde(rename = "assistantResponseMessage")]
        assistant_response_message: KiroAssistantContent,
    },
}

#[derive(Serialize)]
struct KiroUserInputContent {
    content: String,
}

#[derive(Serialize)]
struct KiroAssistantContent {
    content: String,
}

fn build_conversation_state(
    conv: &ConversationRequest,
    model_id: &str,
    origin: &str,
) -> KiroConversationState {
    let conversation_id = Uuid::new_v4().to_string();
    let model_id = model_id.to_string();

    // Extract system prompt if present
    let system_prompt = conv.system.clone();

    // Merge consecutive messages of the same role
    let merged = merge_consecutive_roles(&conv.messages);

    // Prepend system prompt to first user message
    let messages_with_system = if let Some(sys) = system_prompt {
        prepend_system_to_first_user(&merged, &sys)
    } else {
        merged
    };

    // Split: all except last go to history, last becomes currentMessage
    let (history, current_msg) = if messages_with_system.len() > 1 {
        let hist: Vec<KiroHistoryItem> = messages_with_system[..messages_with_system.len() - 1]
            .iter()
            .map(|msg| {
                let content = extract_text_content(&msg.content);
                match msg.role {
                    vkdg_operations::Role::User => KiroHistoryItem::User {
                        user_input_message: KiroUserInputContent { content },
                    },
                    vkdg_operations::Role::Assistant => KiroHistoryItem::Assistant {
                        assistant_response_message: KiroAssistantContent { content },
                    },
                    vkdg_operations::Role::System | vkdg_operations::Role::Tool => {
                        // System should have been prepended; Tool role treated as user context
                        KiroHistoryItem::User {
                            user_input_message: KiroUserInputContent { content },
                        }
                    }
                }
            })
            .collect();
        (Some(hist), messages_with_system.last())
    } else {
        (None, messages_with_system.first())
    };

    // Build current message
    let current_content = current_msg
        .map(|msg| extract_text_content(&msg.content))
        .unwrap_or_default();

    let current_message = KiroMessage::User {
        user_input_message: KiroUserInput {
            content: current_content,
            model_id: model_id.clone(),
            origin: origin.to_owned(),
            // Cache the prefix through this turn: multi-turn conversations resend
            // the same history, so this is a direct token saving.
            cache_point: Some(KiroCachePoint::default_point()),
        },
    };

    KiroConversationState {
        chat_trigger_type: "MANUAL".to_string(),
        agent_task_type: "vibe".to_string(),
        conversation_id,
        current_message,
        history,
    }
}

/// Merge consecutive messages of the same role into one
fn merge_consecutive_roles(messages: &[vkdg_operations::Message]) -> Vec<vkdg_operations::Message> {
    let mut merged: Vec<vkdg_operations::Message> = Vec::new();
    for msg in messages {
        if let Some(last) = merged.last_mut() {
            if last.role == msg.role {
                // Merge content
                let new_content = match (&last.content, &msg.content) {
                    (
                        vkdg_operations::MessageContent::Text(a),
                        vkdg_operations::MessageContent::Text(b),
                    ) => vkdg_operations::MessageContent::Text(format!("{}\n{}", a, b)),
                    (
                        vkdg_operations::MessageContent::Text(a),
                        vkdg_operations::MessageContent::Blocks(blocks),
                    ) => {
                        let b = blocks
                            .iter()
                            .filter_map(|bl| match bl {
                                vkdg_operations::ContentBlock::Text { text } => Some(text.clone()),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        vkdg_operations::MessageContent::Text(format!("{}\n{}", a, b))
                    }
                    (
                        vkdg_operations::MessageContent::Blocks(blocks),
                        vkdg_operations::MessageContent::Text(b),
                    ) => {
                        let a = blocks
                            .iter()
                            .filter_map(|bl| match bl {
                                vkdg_operations::ContentBlock::Text { text } => Some(text.clone()),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        vkdg_operations::MessageContent::Text(format!("{}\n{}", a, b))
                    }
                    (
                        vkdg_operations::MessageContent::Blocks(a),
                        vkdg_operations::MessageContent::Blocks(b),
                    ) => {
                        let mut combined = a.clone();
                        combined.extend(b.clone());
                        vkdg_operations::MessageContent::Blocks(combined)
                    }
                };
                last.content = new_content;
                continue;
            }
        }
        merged.push(msg.clone());
    }
    merged
}

/// Prepend system prompt to first user message
fn prepend_system_to_first_user(
    messages: &[vkdg_operations::Message],
    system: &str,
) -> Vec<vkdg_operations::Message> {
    let mut result = messages.to_vec();
    if let Some(first_user_idx) = result
        .iter()
        .position(|m| m.role == vkdg_operations::Role::User)
    {
        let existing = extract_text_content(&result[first_user_idx].content);
        result[first_user_idx].content =
            vkdg_operations::MessageContent::Text(format!("{}\n\n{}", system, existing));
    }
    result
}

fn extract_text_content(content: &vkdg_operations::MessageContent) -> String {
    match content {
        vkdg_operations::MessageContent::Text(s) => s.clone(),
        vkdg_operations::MessageContent::Blocks(blocks) => blocks
            .iter()
            .filter_map(|b| match b {
                vkdg_operations::ContentBlock::Text { text } => Some(text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
    }
}
