use std::net::IpAddr;
use std::sync::Arc;

use uuid::Uuid;

use vkdg_core::net::IpNet;
use vkdg_core::{ApiType, RequestEnvelope, RequestId, VkdgError};

use crate::admission::AdmissionGuard;

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub listen_addr: String,
    pub max_body_bytes: u64,
    pub max_concurrent_requests: usize,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            listen_addr: "0.0.0.0:8080".to_string(),
            // Anthropic's own Messages limit. Coding agents send long histories
            // and screenshots; 4 MiB rejected ordinary sessions.
            max_body_bytes: 32 * 1024 * 1024,
            max_concurrent_requests: 1000,
        }
    }
}

/// Read a request body, capped at `limit` bytes.
///
/// Over the cap is [`VkdgError::BodyTooLarge`] (413), so a client can tell
/// "too big" from "malformed"; any other read failure is `ConfigInvalid` (400).
pub async fn read_body(body: axum::body::Body, limit: u64) -> Result<bytes::Bytes, VkdgError> {
    use http_body_util::BodyExt;
    let cap = usize::try_from(limit).unwrap_or(usize::MAX);
    match http_body_util::Limited::new(body, cap).collect().await {
        Ok(collected) => Ok(collected.to_bytes()),
        Err(e) if e.is::<http_body_util::LengthLimitError>() => {
            Err(VkdgError::BodyTooLarge { limit_bytes: limit })
        }
        Err(e) => Err(VkdgError::ConfigInvalid {
            field: "body".into(),
            message: format!("unreadable body: {e}"),
        }),
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

/// Extract X-VKDG-* per-request override headers into a [`RequestEnvelope`].
/// Call this after building the envelope from the protocol-specific fields.
/// `client_ip` is not set here: it comes from [`resolve_client_ip`], which needs
/// the socket address the headers alone cannot provide.
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
        .is_some_and(|s| s.eq_ignore_ascii_case("none"));
    envelope.include_think_tags = headers
        .get("x-vkdg-think-tags")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|s| s.eq_ignore_ascii_case("include"));
}

/// The client's address as the gateway should trust it. See
/// [`vkdg_core::net::resolve_client_ip`]; this reads `X-Forwarded-For` from the
/// request headers.
pub fn resolve_client_ip(
    peer: Option<IpAddr>,
    headers: &http::HeaderMap,
    trusted_proxies: &[IpNet],
) -> Option<IpAddr> {
    vkdg_core::net::resolve_client_ip(
        peer,
        headers
            .get_all("x-forwarded-for")
            .iter()
            .filter_map(|v| v.to_str().ok()),
        trusted_proxies,
    )
}

#[cfg(test)]
mod client_ip_tests {
    use super::*;

    fn xff(v: &str) -> http::HeaderMap {
        let mut h = http::HeaderMap::new();
        h.insert("x-forwarded-for", v.parse().unwrap());
        h
    }
    #[allow(clippy::unnecessary_wraps)] // returns Option<IpAddr> to match resolve_client_ip() signature
    fn ip(s: &str) -> Option<IpAddr> {
        Some(s.parse().unwrap())
    }
    fn nets(entries: &[&str]) -> Vec<IpNet> {
        entries.iter().map(|e| e.parse().unwrap()).collect()
    }

    // The bypass: any client could claim an allowlisted address.
    #[test]
    fn spoofed_forwarded_for_from_untrusted_peer_is_ignored() {
        let got = resolve_client_ip(ip("203.0.113.9"), &xff("10.0.0.1"), &[]);
        assert_eq!(got, ip("203.0.113.9"));
        let got = resolve_client_ip(ip("203.0.113.9"), &xff("10.0.0.1"), &nets(&["127.0.0.1"]));
        assert_eq!(got, ip("203.0.113.9"));
    }

    // Behind a trusted proxy, the client is the rightmost hop that is not a
    // proxy. The leftmost entry is whatever the client wrote.
    #[test]
    fn trusted_proxy_chain_is_walked_from_the_right() {
        let trusted = nets(&["127.0.0.1", "10.0.0.0/8"]);
        let got = resolve_client_ip(
            ip("127.0.0.1"),
            &xff("6.6.6.6, 198.51.100.7, 10.1.2.3"),
            &trusted,
        );
        assert_eq!(got, ip("198.51.100.7"));
    }

    #[test]
    fn trusted_proxy_without_header_or_all_trusted_falls_back_sensibly() {
        let trusted = nets(&["127.0.0.1", "10.0.0.0/8"]);
        let none = http::HeaderMap::new();
        assert_eq!(
            resolve_client_ip(ip("127.0.0.1"), &none, &trusted),
            ip("127.0.0.1")
        );
        // Every hop is a proxy: the leftmost is the best remaining guess.
        assert_eq!(
            resolve_client_ip(ip("127.0.0.1"), &xff("10.0.0.2, 10.0.0.3"), &trusted),
            ip("10.0.0.2")
        );
        // No socket address (in-process tests): nothing to trust.
        assert_eq!(resolve_client_ip(None, &xff("10.0.0.1"), &trusted), None);
    }
}
