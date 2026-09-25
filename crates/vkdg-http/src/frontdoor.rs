use std::sync::Arc;

use uuid::Uuid;

use vkdg_core::{ApiType, RequestId};

use crate::admission::AdmissionGuard;

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub listen_addr: String,
    pub max_body_bytes: u64,
    pub request_timeout_secs: u64,
    pub max_concurrent_requests: usize,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            listen_addr: "0.0.0.0:8080".to_string(),
            max_body_bytes: 4 * 1024 * 1024, // 4 MB
            request_timeout_secs: 120,
            max_concurrent_requests: 1000,
        }
    }
}

pub struct FrontDoor {
    pub server_config: ServerConfig,
    pub admission: Arc<AdmissionGuard>,
}

impl FrontDoor {
    pub fn new(config: ServerConfig) -> Self {
        let limit = config.max_concurrent_requests;
        Self {
            admission: Arc::new(AdmissionGuard::new(limit)),
            server_config: config,
        }
    }

    pub fn assign_request_id() -> RequestId {
        RequestId(Uuid::new_v4())
    }

    pub fn extract_api_type<B>(req: &hyper::Request<B>) -> ApiType
    where
        B: hyper::body::Body,
    {
        let path = req.uri().path();
        if path.starts_with("/v1/messages") {
            ApiType::AnthropicMessages
        } else if path.starts_with("/v1/chat/completions") {
            ApiType::OpenAiChatCompletions
        } else if path.starts_with("/v1/responses") {
            ApiType::OpenAiResponses
        } else {
            ApiType::VkdgNative
        }
    }
}
