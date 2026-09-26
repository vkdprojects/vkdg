// Upstream HTTP client — wraps reqwest for forwarding requests to AI providers.

use std::pin::Pin;

use bytes::Bytes;
use futures::Stream;
use http::{HeaderMap, Method};

use vkdg_core::VkdgError;

// ── Request / Response types ──────────────────────────────────────────────────

pub struct UpstreamRequest {
    pub method: Method,
    pub url: String,
    pub headers: HeaderMap,
    pub body: Bytes,
}

pub enum UpstreamResponse {
    Complete {
        status: u16,
        body: Bytes,
    },
    Streaming {
        status: u16,
        body: Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>>,
    },
}

// ── Client ────────────────────────────────────────────────────────────────────

pub struct HttpClient {
    inner: reqwest::Client,
}

impl HttpClient {
    pub fn new() -> Self {
        let inner = reqwest::Client::builder()
            .use_rustls_tls()
            .build()
            .expect("failed to build reqwest client");
        Self { inner }
    }

    /// Send an upstream request.
    ///
    /// - `streaming = false`: collect the entire body into `UpstreamResponse::Complete`.
    /// - `streaming = true`:  return `UpstreamResponse::Streaming` with the body as a
    ///   lazy byte stream; the caller is responsible for driving it.
    ///
    /// Non-2xx responses always return `Err(VkdgError::UpstreamError)`.
    pub async fn send(
        &self,
        req: UpstreamRequest,
        streaming: bool,
    ) -> Result<UpstreamResponse, VkdgError> {
        // Convert http::HeaderMap → reqwest::header::HeaderMap.
        // Both are re-exports of the same `http` crate type in reqwest 0.12, so
        // we can convert field-by-field to avoid any version mismatch.
        let mut reqwest_headers = reqwest::header::HeaderMap::new();
        for (name, value) in &req.headers {
            let name_str = name.as_str();
            if let (Ok(k), Ok(v)) = (
                reqwest::header::HeaderName::from_bytes(name_str.as_bytes()),
                reqwest::header::HeaderValue::from_bytes(value.as_bytes()),
            ) {
                reqwest_headers.insert(k, v);
            }
        }

        let reqwest_method = reqwest::Method::from_bytes(req.method.as_str().as_bytes())
            .map_err(|e| VkdgError::Internal(format!("invalid method: {e}")))?;

        let resp = self
            .inner
            .request(reqwest_method, &req.url)
            .headers(reqwest_headers)
            .body(req.body)
            .send()
            .await
            .map_err(|e| VkdgError::UpstreamError {
                code: 0,
                message: format!("connection error: {e}"),
            })?;

        let status = resp.status().as_u16();

        if !(200..300).contains(&status) {
            // Collect error body for the message (bounded read — 4 KiB).
            let body_bytes = resp.bytes().await.unwrap_or_default();
            let message =
                String::from_utf8_lossy(&body_bytes[..body_bytes.len().min(4096)]).into_owned();
            return Err(VkdgError::UpstreamError {
                code: status,
                message,
            });
        }

        if streaming {
            use futures::StreamExt;

            let byte_stream = resp
                .bytes_stream()
                .map(|r| r.map_err(|e| std::io::Error::other(e.to_string())));
            Ok(UpstreamResponse::Streaming {
                status,
                body: Box::pin(byte_stream),
            })
        } else {
            let body = resp.bytes().await.map_err(|e| VkdgError::UpstreamError {
                code: status,
                message: format!("body read error: {e}"),
            })?;
            Ok(UpstreamResponse::Complete { status, body })
        }
    }
}

impl Default for HttpClient {
    fn default() -> Self {
        Self::new()
    }
}
