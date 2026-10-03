//! Expected statuses and error types are written by hand from the Anthropic and `OpenAI`
//! error documentation and from the pipeline's pre-existing mapping.

use serde_json::{json, Value};
use vkdg_core::{ApiType, VkdgError};

use super::{client_error, stream_error_frame};

fn upstream(code: u16) -> VkdgError {
    VkdgError::UpstreamError {
        code,
        message: "m".into(),
        retry_after: None,
    }
}

struct Case {
    err: VkdgError,
    message: &'static str,
    status: u16,
    anthropic_type: &'static str,
    openai_type: &'static str,
    openai_code: Option<&'static str>,
}

fn case(
    err: VkdgError,
    message: &'static str,
    status: u16,
    anthropic_type: &'static str,
    openai_type: &'static str,
) -> Case {
    Case {
        err,
        message,
        status,
        anthropic_type,
        openai_type,
        openai_code: None,
    }
}

fn cases() -> Vec<Case> {
    vec![
        case(
            VkdgError::Unauthenticated,
            "unauthenticated",
            401,
            "authentication_error",
            "authentication_error",
        ),
        case(
            VkdgError::Unauthorized,
            "unauthorized",
            403,
            "permission_error",
            "permission_error",
        ),
        case(
            VkdgError::AdmissionRejected {
                reason: "full".into(),
            },
            "admission rejected: full",
            503,
            "overloaded_error",
            "server_error",
        ),
        case(
            VkdgError::CapabilityUnsupported {
                capability: "vision".into(),
            },
            "capability unsupported: vision",
            400,
            "invalid_request_error",
            "invalid_request_error",
        ),
        case(
            VkdgError::NoEligibleConnection,
            "no eligible connection",
            502,
            "api_error",
            "server_error",
        ),
        case(
            VkdgError::NoRouteMatched,
            "no route matches the requested model",
            502,
            "api_error",
            "server_error",
        ),
        case(
            VkdgError::PluginError {
                plugin_id: "p".into(),
                message: "boom".into(),
            },
            "plugin error (p): boom",
            500,
            "api_error",
            "server_error",
        ),
        case(
            VkdgError::ConfigInvalid {
                field: "f".into(),
                message: "bad".into(),
            },
            "config invalid: f: bad",
            400,
            "invalid_request_error",
            "invalid_request_error",
        ),
        case(
            VkdgError::BodyTooLarge { limit_bytes: 10 },
            "request body exceeds 10 bytes",
            413,
            "request_too_large",
            "invalid_request_error",
        ),
        case(
            VkdgError::CredentialRevoked {
                status: 400,
                message: "revoked".into(),
            },
            "credential revoked (400): revoked",
            401,
            "authentication_error",
            "authentication_error",
        ),
        case(
            VkdgError::Internal("x".into()),
            "internal: x",
            500,
            "api_error",
            "server_error",
        ),
        case(
            VkdgError::BudgetExceeded {
                estimated_usd: 2.0,
                limit_usd: 1.0,
            },
            "budget exceeded: request estimated cost 2 exceeds limit 1",
            402,
            "budget_exceeded_error",
            "budget_exceeded",
        ),
        // Upstream errors follow their effective status.
        case(
            upstream(400),
            "upstream error 400: m",
            400,
            "invalid_request_error",
            "invalid_request_error",
        ),
        case(
            upstream(401),
            "upstream error 401: m",
            401,
            "authentication_error",
            "authentication_error",
        ),
        case(
            upstream(402),
            "upstream error 402: m",
            402,
            "billing_error",
            "invalid_request_error",
        ),
        case(
            upstream(403),
            "upstream error 403: m",
            403,
            "permission_error",
            "permission_error",
        ),
        case(
            upstream(404),
            "upstream error 404: m",
            404,
            "not_found_error",
            "invalid_request_error",
        ),
        case(
            upstream(413),
            "upstream error 413: m",
            413,
            "request_too_large",
            "invalid_request_error",
        ),
        case(
            upstream(422),
            "upstream error 422: m",
            422,
            "invalid_request_error",
            "invalid_request_error",
        ),
        Case {
            openai_code: Some("rate_limit_exceeded"),
            ..case(
                upstream(429),
                "upstream error 429: m",
                429,
                "rate_limit_error",
                "rate_limit_error",
            )
        },
        case(
            upstream(500),
            "upstream error 500: m",
            500,
            "api_error",
            "server_error",
        ),
        case(
            upstream(503),
            "upstream error 503: m",
            503,
            "api_error",
            "server_error",
        ),
        case(
            upstream(504),
            "upstream error 504: m",
            504,
            "timeout_error",
            "server_error",
        ),
        // 529 (Anthropic overloaded) reaches the client as 429.
        Case {
            openai_code: Some("rate_limit_exceeded"),
            ..case(
                upstream(529),
                "upstream error 529: m",
                429,
                "rate_limit_error",
                "rate_limit_error",
            )
        },
        // Statuses that are not error statuses fall back to 502.
        case(
            upstream(0),
            "upstream error 0: m",
            502,
            "api_error",
            "server_error",
        ),
        case(
            upstream(200),
            "upstream error 200: m",
            502,
            "api_error",
            "server_error",
        ),
        case(
            upstream(700),
            "upstream error 700: m",
            502,
            "api_error",
            "server_error",
        ),
    ]
}

// Refutes: a table that drifts per dialect, a status/type pair that disagrees with the
// pipeline's pre-existing mapping, or upstream errors all labelled overloaded_error.
#[test]
fn every_error_maps_to_the_documented_status_type_and_message() {
    for c in cases() {
        let anth = client_error(&ApiType::AnthropicMessages, &c.err);
        assert_eq!(anth.status, c.status, "anthropic status for {}", c.message);
        assert_eq!(
            serde_json::from_slice::<Value>(&anth.body).unwrap(),
            json!({"type":"error","error":{"type":c.anthropic_type,"message":c.message}}),
            "anthropic body for {}",
            c.message
        );

        let oai = client_error(&ApiType::OpenAiChatCompletions, &c.err);
        assert_eq!(oai.status, c.status, "openai status for {}", c.message);
        assert_eq!(
            serde_json::from_slice::<Value>(&oai.body).unwrap(),
            json!({"error":{"message":c.message,"type":c.openai_type,
                            "param":null,"code":c.openai_code}}),
            "openai body for {}",
            c.message
        );
    }
}

// Refutes: dialect chosen by anything but the client's api type (the Responses and native
// dialects fall back to the Anthropic shape, as stream_encoder_for does).
#[test]
fn only_openai_chat_clients_get_the_openai_shape() {
    let err = VkdgError::Unauthenticated;
    for api in [
        ApiType::AnthropicMessages,
        ApiType::VkdgNative,
        ApiType::OpenAiResponses,
    ] {
        let body: Value = serde_json::from_slice(&client_error(&api, &err).body).unwrap();
        assert_eq!(body["type"], "error", "{api:?}");
    }
    let body: Value =
        serde_json::from_slice(&client_error(&ApiType::OpenAiChatCompletions, &err).body).unwrap();
    assert!(body.get("type").is_none() && body["error"]["type"] == "authentication_error");
}

// Refutes: dropping the upstream's retry-after so clients hammer a rate-limited account.
#[test]
fn retry_after_is_surfaced_only_for_upstream_errors() {
    let with = VkdgError::UpstreamError {
        code: 529,
        message: "busy".into(),
        retry_after: Some(30),
    };
    for api in [ApiType::AnthropicMessages, ApiType::OpenAiChatCompletions] {
        assert_eq!(client_error(&api, &with).retry_after, Some(30));
        assert_eq!(client_error(&api, &upstream(429)).retry_after, None);
        assert_eq!(
            client_error(&api, &VkdgError::Unauthorized).retry_after,
            None
        );
    }
}

// Refutes: a body that is not exactly Anthropic's documented key order / shape.
#[test]
fn anthropic_body_bytes_are_exact() {
    let out = client_error(&ApiType::AnthropicMessages, &VkdgError::Unauthenticated);
    assert_eq!(
        std::str::from_utf8(&out.body).unwrap(),
        r#"{"type":"error","error":{"type":"authentication_error","message":"unauthenticated"}}"#
    );
}

// Refutes: a termination frame that differs from what the stream encoders emit for Failed, an
// OpenAI error frame without the closing [DONE], or output off the `\n\n` boundary.
#[test]
fn stream_error_frames_are_byte_exact_per_dialect() {
    let err = VkdgError::UpstreamError {
        code: 502,
        message: "upstream stream ended before completion".into(),
        retry_after: None,
    };
    assert_eq!(
        std::str::from_utf8(&stream_error_frame(&ApiType::AnthropicMessages, &err)).unwrap(),
        "event: error\n\
         data: {\"type\":\"error\",\"error\":{\"type\":\"api_error\",\
         \"message\":\"upstream error 502: upstream stream ended before completion\"}}\n\n"
    );
    assert_eq!(
        std::str::from_utf8(&stream_error_frame(&ApiType::OpenAiChatCompletions, &err)).unwrap(),
        "data: {\"error\":{\"message\":\"upstream error 502: upstream stream ended before completion\",\
         \"type\":\"server_error\",\"param\":null,\"code\":null}}\n\n\
         data: [DONE]\n\n"
    );
}
