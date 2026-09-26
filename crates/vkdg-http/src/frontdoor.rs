use std::sync::Arc;

use uuid::Uuid;

use vkdg_core::{ApiType, RequestEnvelope, RequestId};

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

/// Extract the client's IP address from request headers.
///
/// Prefers `X-Forwarded-For` (first entry, behind a proxy), then `X-Real-IP`.
/// Returns `None` when neither header is present or parseable.
pub fn extract_client_ip(headers: &http::HeaderMap) -> Option<String> {
    if let Some(xff) = headers.get("x-forwarded-for") {
        if let Ok(val) = xff.to_str() {
            if let Some(first) = val.split(',').next() {
                let ip = first.trim().to_string();
                if !ip.is_empty() {
                    return Some(ip);
                }
            }
        }
    }
    if let Some(xri) = headers.get("x-real-ip") {
        if let Ok(val) = xri.to_str() {
            let ip = val.trim().to_string();
            if !ip.is_empty() {
                return Some(ip);
            }
        }
    }
    None
}

/// Extract X-VKDG-* per-request override headers into a [`RequestEnvelope`].
/// Call this after building the envelope from the protocol-specific fields.
pub fn extract_vkdg_overrides(headers: &http::HeaderMap, envelope: &mut RequestEnvelope) {
    envelope.mode_pack_override = headers
        .get("x-vkdg-mode")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    envelope.compression_override = headers
        .get("x-vkdg-compression")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    envelope.cache_bypass = headers
        .get("x-vkdg-cache")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.eq_ignore_ascii_case("none"))
        .unwrap_or(false);
    envelope.include_think_tags = headers
        .get("x-vkdg-think-tags")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.eq_ignore_ascii_case("include"))
        .unwrap_or(false);
    envelope.client_ip = extract_client_ip(headers);
}
