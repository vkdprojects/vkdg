//! VKDG provider plugin: kiro
//! Amazon Q / CodeWhisperer streaming protocol.

use http::HeaderMap;
use serde::Serialize;
use uuid::Uuid;
use vkdg_operations::{MessageContent, Role, Message, Operation, ConversationRequest};
use vkdg_connections::ConnectionConfig;
use vkdg_provider_sdk::{
    ConversationStreamDecoder, PreparedRequest,
    ProviderAdapter, ProviderError,
};

pub mod decode;
pub mod eventstream;
pub mod stream_decoder;

const KIRO_API_URL: &str = "https://q.us-east-1.amazonaws.com/";
const KIRO_API_TARGET: &str = "AmazonCodeWhispererStreamingService.GenerateAssistantResponse";

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
        token: &str,
    ) -> Result<PreparedRequest, ProviderError> {
        let Operation::Conversation(conv) = operation else {
            return Err(ProviderError::UnsupportedOperation);
        };

        // Get model from config (first model or default to auto)
        let model_id = config.models.first()
            .map(|m| m.replace('-', "."))
            .unwrap_or_else(|| "auto".to_string());

        // Build request body with real Kiro schema
        let body = KiroRequestBody {
            conversation_state: build_conversation_state(conv, &model_id),
            agent_mode: "vibe".to_string(),
        };

        let mut headers = HeaderMap::new();
        headers.insert(http::header::CONTENT_TYPE, http::HeaderValue::from_static("application/x-amz-json-1.0"));
        headers.insert("x-amz-target", http::HeaderValue::from_static(KIRO_API_TARGET));
        headers.insert(
            http::header::AUTHORIZATION, 
            http::HeaderValue::from_str(&format!("Bearer {}", token))
                .map_err(|_| ProviderError::Http("Invalid authorization token".to_string()))?
        );
        headers.insert("tokentype", http::HeaderValue::from_static("API_KEY"));
        headers.insert(http::header::ACCEPT, http::HeaderValue::from_static("application/json"));
        headers.insert("x-amzn-codewhisperer-optout", http::HeaderValue::from_static("true"));
        headers.insert("amz-sdk-invocation-id", http::HeaderValue::from_str(&Uuid::new_v4().to_string())
            .map_err(|_| ProviderError::Http("Invalid UUID".to_string()))?);
        headers.insert("amz-sdk-request", http::HeaderValue::from_static("attempt=1; max=1"));
        headers.insert("x-amzn-kiro-agent-mode", http::HeaderValue::from_static("vibe"));
        headers.insert("x-amz-user-agent", http::HeaderValue::from_static("vkdg/0.1.0"));

        Ok(PreparedRequest {
            url: KIRO_API_URL.to_string(),
            headers,
            body: serde_json::to_vec(&body).map_err(|e| ProviderError::Serialization(e.to_string()))?.into(),
            is_streaming: true,
        })
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
    #[serde(rename = "agentMode")]
    agent_mode: String,
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
        user_input_message: KiroUserInput 
    },
}

#[derive(Serialize)]
struct KiroUserInput {
    content: String,
    #[serde(rename = "modelId")]
    model_id: String,
    origin: String,
}

#[derive(Serialize)]
#[serde(untagged)]
enum KiroHistoryItem {
    User { 
        #[serde(rename = "userInputMessage")]
        user_input_message: KiroUserInputContent 
    },
    Assistant { 
        #[serde(rename = "assistantResponseMessage")]
        assistant_response_message: KiroAssistantContent 
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

fn build_conversation_state(conv: &ConversationRequest, model_id: &str) -> KiroConversationState {
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
        let hist: Vec<KiroHistoryItem> = messages_with_system[..messages_with_system.len()-1]
            .iter()
            .map(|msg| {
                let content = extract_text_content(&msg.content);
                match msg.role {
                    vkdg_operations::Role::User => {
                        KiroHistoryItem::User { 
                            user_input_message: KiroUserInputContent { content } 
                        }
                    }
                    vkdg_operations::Role::Assistant => {
                        KiroHistoryItem::Assistant { 
                            assistant_response_message: KiroAssistantContent { content } 
                        }
                    }
                    vkdg_operations::Role::System | vkdg_operations::Role::Tool => {
                        // System should have been prepended; Tool role treated as user context
                        KiroHistoryItem::User { 
                            user_input_message: KiroUserInputContent { content } 
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
            origin: "AI_EDITOR".to_string(),
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
                    (vkdg_operations::MessageContent::Text(a), vkdg_operations::MessageContent::Text(b)) => {
                        vkdg_operations::MessageContent::Text(format!("{}\n{}", a, b))
                    }
                    (vkdg_operations::MessageContent::Text(a), vkdg_operations::MessageContent::Blocks(blocks)) => {
                        let b = blocks.iter()
                            .filter_map(|bl| match bl {
                                vkdg_operations::ContentBlock::Text { text } => Some(text.clone()),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        vkdg_operations::MessageContent::Text(format!("{}\n{}", a, b))
                    }
                    (vkdg_operations::MessageContent::Blocks(blocks), vkdg_operations::MessageContent::Text(b)) => {
                        let a = blocks.iter()
                            .filter_map(|bl| match bl {
                                vkdg_operations::ContentBlock::Text { text } => Some(text.clone()),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        vkdg_operations::MessageContent::Text(format!("{}\n{}", a, b))
                    }
                    (vkdg_operations::MessageContent::Blocks(a), vkdg_operations::MessageContent::Blocks(b)) => {
                        let mut combined = a.clone();
                        combined.extend(b.clone());
                        vkdg_operations::MessageContent::Blocks(combined)
                    }
                    _ => msg.content.clone(),
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
fn prepend_system_to_first_user(messages: &[vkdg_operations::Message], system: &str) -> Vec<vkdg_operations::Message> {
    let mut result = messages.to_vec();
    if let Some(first_user_idx) = result.iter().position(|m| m.role == vkdg_operations::Role::User) {
        let existing = extract_text_content(&result[first_user_idx].content);
        result[first_user_idx].content = vkdg_operations::MessageContent::Text(format!("{}\n\n{}", system, existing));
    }
    result
}

fn extract_text_content(content: &vkdg_operations::MessageContent) -> String {
    match content {
        vkdg_operations::MessageContent::Text(s) => s.clone(),
        vkdg_operations::MessageContent::Blocks(blocks) => {
            blocks.iter()
                .filter_map(|b| match b {
                    vkdg_operations::ContentBlock::Text { text } => Some(text.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
    }
}