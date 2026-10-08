use std::time::{SystemTime, UNIX_EPOCH};

use axum::response::Response;
use http::header;
use vkdg_core::{ApiType, VkdgError};
use vkdg_operations::{ContentBlock, MessageContent, Role};

// ── Cache bypass helpers ──────────────────────────────────────────────────────

/// True when the conversation has more than one assistant turn in its history.
/// Multi-turn conversations are never cached: the response depends on prior
/// assistant outputs that may not be stable across retries.
pub(super) fn is_multiturn(op: &vkdg_operations::ConversationRequest) -> bool {
    op.messages
        .iter()
        .filter(|m| matches!(m.role, Role::Assistant))
        .count()
        > 1
}

/// True when any message in the conversation contains `tool_use` or `tool_result`
/// content blocks.  Tool-call conversations are non-deterministic.
pub(super) fn has_tool_calls(op: &vkdg_operations::ConversationRequest) -> bool {
    op.messages.iter().any(|m| {
        matches!(
            &m.content,
            MessageContent::Blocks(blocks) if blocks.iter().any(|b|
                matches!(b, ContentBlock::ToolUse { .. } | ContentBlock::ToolResult { .. })
            )
        )
    })
}

pub(super) fn unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

// ── Failover classification ───────────────────────────────────────────────────

/// Upstream statuses that put the connection that answered into cooldown: rate
/// limit (429), out of credits (402), auth failures (401, 403) and server-side
/// failures (5xx).
pub(super) fn cools_connection(code: u16) -> bool {
    matches!(code, 401 | 402 | 403 | 429) || code >= 500
}

/// Upstream statuses worth one transparent retry on a sibling connection before
/// any byte reaches the client: rate limit (429), overloaded (529) and out of
/// credits (402) say "this account cannot serve right now", not "this request is
/// bad". Other 5xx cool the connection but are not retried.
pub(super) fn fails_over(code: u16) -> bool {
    matches!(code, 402 | 429 | 529)
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// The error response a client of `api_type` receives for `err`: status, JSON body
/// and `retry-after` come from the one mapper in `vkdg-operations`, so the pipeline,
/// the stream encoders and the ingress crates agree.
pub(super) fn error_response(api_type: &ApiType, err: &VkdgError) -> Response {
    use axum::response::IntoResponse;
    use http::{HeaderMap, HeaderValue, StatusCode};

    let out = vkdg_operations::client_error(api_type, err);
    let status = StatusCode::from_u16(out.status).unwrap_or(StatusCode::BAD_GATEWAY);
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    if let Some(secs) = out.retry_after {
        headers.insert(header::RETRY_AFTER, HeaderValue::from(secs));
    }
    (status, headers, out.body).into_response()
}

#[cfg(test)]
mod error_response_tests {
    use http::header::RETRY_AFTER;
    use vkdg_core::{ApiType, VkdgError};

    use super::error_response;

    async fn parts(
        api_type: &ApiType,
        err: &VkdgError,
    ) -> (u16, Option<String>, serde_json::Value) {
        let resp = error_response(api_type, err);
        let status = resp.status().as_u16();
        let retry = resp
            .headers()
            .get(RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let body = axum::body::to_bytes(resp.into_body(), 64 * 1024)
            .await
            .unwrap_or_default();
        (
            status,
            retry,
            serde_json::from_slice(&body).unwrap_or_default(),
        )
    }

    fn upstream(code: u16, retry_after: Option<u32>) -> VkdgError {
        VkdgError::UpstreamError {
            code,
            message: "slow down".into(),
            retry_after,
        }
    }

    // Refutes: rendering every pipeline error as an Anthropic body. An OpenAI SDK
    // reads `error.message` / `error.type` / `error.code`; the Anthropic shape
    // (`{"type":"error",...}`) surfaces as an opaque parse failure.
    #[tokio::test]
    async fn openai_client_gets_an_openai_error_body() {
        let (status, _, body) = parts(
            &ApiType::OpenAiChatCompletions,
            &VkdgError::NoEligibleConnection,
        )
        .await;
        assert_eq!(status, 502);
        assert!(
            body.get("type").is_none(),
            "Anthropic envelope leaked: {body}"
        );
        assert_eq!(body["error"]["message"], "no eligible connection", "{body}");
        assert!(body["error"]["type"].is_string(), "{body}");
        assert!(body["error"].get("code").is_some(), "{body}");
    }

    // Refutes: touching the Anthropic client's shape while fixing the OpenAI one.
    #[tokio::test]
    async fn anthropic_client_keeps_the_anthropic_error_body() {
        let (status, _, body) = parts(
            &ApiType::AnthropicMessages,
            &VkdgError::NoEligibleConnection,
        )
        .await;
        assert_eq!(status, 502);
        assert_eq!(body["type"], "error", "{body}");
        assert_eq!(body["error"]["message"], "no eligible connection", "{body}");
    }

    // Refutes: losing the status mapping (529 is the upstream's "overloaded", clients
    // retry 429) or `retry-after` when the dialect changes.
    #[tokio::test]
    async fn upstream_status_and_retry_after_survive_in_both_dialects() {
        for api_type in [ApiType::AnthropicMessages, ApiType::OpenAiChatCompletions] {
            let (status, retry, _) = parts(&api_type, &upstream(529, Some(30))).await;
            assert_eq!(
                (status, retry.as_deref()),
                (429, Some("30")),
                "{api_type:?}"
            );
            let (status, retry, _) = parts(&api_type, &upstream(503, None)).await;
            assert_eq!((status, retry), (503, None), "{api_type:?}");
        }
    }
}
