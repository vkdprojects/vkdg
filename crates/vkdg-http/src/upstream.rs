// Upstream HTTP client — wraps reqwest for forwarding requests to AI providers.

use std::pin::Pin;
use std::time::Duration;

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
    /// Bounds TCP connect + TLS handshake + DNS resolution — the phase before
    /// any upstream byte has been sent. It intentionally does NOT bound the
    /// request as a whole: this gateway streams SSE responses that can
    /// legitimately run for minutes, so a blanket `.timeout()` would sever a
    /// healthy stream mid-flight. nginx in front of this gateway makes the
    /// same choice (`proxy_connect_timeout 30s`, `proxy_read_timeout 3600s`).
    ///
    /// Without this, a stalled DNS resolution or unreachable host falls back
    /// to the OS/libc default (glibc's resolver gives up after ~5s per
    /// nameserver) — unbounded from vkdg's point of view, and unobserved. 10s
    /// is generous for connect+TLS to a healthy provider and still fails fast
    /// when the network is broken.
    pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

    pub fn new() -> Self {
        Self::with_connect_timeout(Self::DEFAULT_CONNECT_TIMEOUT)
    }

    /// Same as [`Self::new`] with an explicit connect timeout. Exists so
    /// tests can use a short bound instead of waiting out the production
    /// default against an unroutable address.
    pub fn with_connect_timeout(connect_timeout: Duration) -> Self {
        let inner = reqwest::Client::builder()
            .use_rustls_tls()
            .connect_timeout(connect_timeout)
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
                message: format!("{}: {e}", connect_error_kind(&e)),
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

/// Classifies a failed `send()` so operators see *why* the connection never
/// opened instead of an opaque "connection error". Distinguishes the phase
/// before any upstream byte arrived: DNS/connect refusal, a connect/handshake
/// timeout (including the `connect_timeout` configured above), or some other
/// transport failure (e.g. TLS negotiation). Deliberately does not invent a
/// new HTTP status — callers still map this through `UpstreamError{code: 0}`.
fn connect_error_kind(e: &reqwest::Error) -> &'static str {
    if e.is_timeout() {
        "connect timeout"
    } else if e.is_connect() {
        "dns/connect failure"
    } else {
        "connection error"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Refutes an implementation that leaves the reqwest client with no
    /// connect timeout (today's production bug): against an address that
    /// never responds (RFC 5737 documentation range, guaranteed unroutable
    /// from this process), `send()` must fail at approximately the configured
    /// bound rather than hanging indefinitely or waiting out an OS/libc
    /// default the test does not control.
    #[tokio::test(flavor = "multi_thread")]
    async fn connect_timeout_bounds_an_unroutable_address() {
        let client = HttpClient::with_connect_timeout(Duration::from_millis(300));
        let req = UpstreamRequest {
            method: Method::POST,
            // TEST-NET-1: reserved for documentation, never routed on the
            // public internet or in CI sandboxes — connection attempts hang
            // rather than being refused, which is what makes it a faithful
            // stand-in for a stalled DNS/connect phase.
            url: "http://192.0.2.1:8443/v1/messages".to_string(),
            headers: HeaderMap::new(),
            body: Bytes::new(),
        };

        let started = std::time::Instant::now();
        let result = client.send(req, false).await;
        let elapsed = started.elapsed();

        match result {
            Err(VkdgError::UpstreamError { code, message }) => {
                assert!(
                    elapsed < Duration::from_secs(2),
                    "connect_timeout(300ms) must bound the attempt; took {elapsed:?} \
                     instead — this would be an implementation with no connect timeout \
                     at all, which is the bug this test exists to catch"
                );
                assert_eq!(code, 0, "a connect failure never reaches an upstream status");
                assert!(
                    message.contains("connect timeout") || message.contains("dns/connect"),
                    "message should classify the failure instead of a bare \
                     opaque string: {message:?}"
                );
            }
            Err(other) => panic!("expected UpstreamError, got {other:?}"),
            Ok(_) => panic!("connecting to an unroutable address must fail, not succeed"),
        }
    }

    /// Refutes an implementation that applies a blanket total-request timeout
    /// (the dangerous regression this change must avoid): a slow *body*
    /// arriving well past the connect timeout, after the connection and
    /// headers succeed immediately, must still be read to completion.
    #[tokio::test(flavor = "multi_thread")]
    async fn slow_streaming_body_is_not_cut_by_connect_timeout() {
        use std::convert::Infallible;

        use http_body_util::StreamBody;
        use hyper::body::Frame;
        use hyper::service::service_fn;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let connect_timeout = Duration::from_millis(300);
        // Total delay across the streamed chunks comfortably exceeds the
        // connect timeout: a regression that wraps the whole request in
        // `.timeout(connect_timeout)` would sever this stream early.
        let chunk_delay = connect_timeout * 4;

        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let io = hyper_util::rt::TokioIo::new(stream);
            let service = service_fn(move |_req: hyper::Request<hyper::body::Incoming>| {
                let chunk_delay = chunk_delay;
                async move {
                    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Frame<Bytes>, Infallible>>(4);
                    tokio::spawn(async move {
                        for chunk in [&b"chunk-one"[..], &b"chunk-two"[..], &b"chunk-three"[..]] {
                            tokio::time::sleep(chunk_delay).await;
                            if tx.send(Ok(Frame::data(Bytes::from_static(chunk)))).await.is_err() {
                                return;
                            }
                        }
                    });
                    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
                    Ok::<_, Infallible>(hyper::Response::new(StreamBody::new(stream)))
                }
            });
            let _ = hyper::server::conn::http1::Builder::new()
                .serve_connection(io, service)
                .await;
        });

        let client = HttpClient::with_connect_timeout(connect_timeout);
        let req = UpstreamRequest {
            method: Method::GET,
            url: format!("http://{addr}/stream"),
            headers: HeaderMap::new(),
            body: Bytes::new(),
        };

        let started = std::time::Instant::now();
        let resp = client
            .send(req, true)
            .await
            .expect("connect succeeds immediately; only the body trickles in slowly");

        let collected = match resp {
            UpstreamResponse::Streaming { body, .. } => {
                use futures::StreamExt;
                body.map(|r| r.expect("stream chunk"))
                    .collect::<Vec<Bytes>>()
                    .await
            }
            UpstreamResponse::Complete { .. } => panic!("expected a streaming response"),
        };
        let elapsed = started.elapsed();

        let full: Vec<u8> = collected.into_iter().flatten().collect();
        assert_eq!(
            full,
            b"chunk-onechunk-twochunk-three",
            "the full slow stream must arrive intact"
        );
        assert!(
            elapsed >= chunk_delay * 3 - Duration::from_millis(50),
            "the stream must not be cut short by the connect timeout; only \
             {elapsed:?} elapsed for a body that takes {:?} to fully arrive",
            chunk_delay * 3
        );
    }
}
