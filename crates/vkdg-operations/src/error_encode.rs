//! The one client-dialect error renderer.
//!
//! Pipeline errors, mid-stream failures and the stream encoders all render
//! through here, so a given [`VkdgError`] has exactly one status, one error type
//! and one body per client dialect. Only an `OpenAI` Chat client gets the
//! `OpenAI` shape; every other dialect gets Anthropic's (the same fallback as
//! [`stream_encoder_for`](crate::stream_encoder_for)).

use serde::Serialize;
use vkdg_core::{ApiType, VkdgError};

/// Status, `retry-after` and JSON body of an error response in the client's dialect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientError {
    pub status: u16,
    /// Seconds the upstream asked the client to wait; only upstream errors carry one.
    pub retry_after: Option<u32>,
    /// `application/json`.
    pub body: Vec<u8>,
}

/// How one error looks in both dialects.
struct Mapped {
    status: u16,
    anthropic: &'static str,
    openai: &'static str,
    openai_code: Option<&'static str>,
}

impl Mapped {
    const fn new(status: u16, anthropic: &'static str, openai: &'static str) -> Self {
        Self {
            status,
            anthropic,
            openai,
            openai_code: None,
        }
    }
}

fn map(err: &VkdgError) -> Mapped {
    match err {
        VkdgError::Unauthenticated => {
            Mapped::new(401, "authentication_error", "authentication_error")
        }
        VkdgError::Unauthorized => Mapped::new(403, "permission_error", "permission_error"),
        VkdgError::AdmissionRejected { .. } => Mapped::new(503, "overloaded_error", "server_error"),
        VkdgError::CapabilityUnsupported { .. } | VkdgError::ConfigInvalid { .. } => {
            Mapped::new(400, "invalid_request_error", "invalid_request_error")
        }
        VkdgError::NoEligibleConnection | VkdgError::NoRouteMatched => {
            Mapped::new(502, "api_error", "server_error")
        }
        VkdgError::PluginError { .. } | VkdgError::Internal(_) => {
            Mapped::new(500, "api_error", "server_error")
        }
        VkdgError::BodyTooLarge { .. } => {
            Mapped::new(413, "request_too_large", "invalid_request_error")
        }
        VkdgError::CredentialRevoked { .. } => {
            Mapped::new(401, "authentication_error", "authentication_error")
        }
        VkdgError::BudgetExceeded { .. } => {
            Mapped::new(402, "budget_exceeded_error", "budget_exceeded")
        }
        VkdgError::UpstreamError {
            code: 400, message, ..
        } if is_context_length_rejection(message) => Mapped {
            openai_code: Some("context_length_exceeded"),
            ..map_upstream(400)
        },
        VkdgError::UpstreamError { code, .. } => map_upstream(*code),
    }
}

/// Kiro reports the same rejection as an HTTP JSON body or an in-stream error.
/// Inspect the reason field, not arbitrary error text that may quote a prompt.
fn is_context_length_rejection(message: &str) -> bool {
    message.starts_with("CONTENT_LENGTH_EXCEEDS_THRESHOLD:")
        || serde_json::from_str::<serde_json::Value>(message).is_ok_and(|body| {
            body.get("reason").and_then(serde_json::Value::as_str)
                == Some("CONTENT_LENGTH_EXCEEDS_THRESHOLD")
        })
}

/// A gateway-made protocol failure (`message` is shown verbatim, not as a
/// [`VkdgError`]): the encoders use it when the event stream itself is malformed.
pub fn protocol_error_frame(api_type: &ApiType, message: &str) -> Vec<u8> {
    frame(
        api_type,
        &Mapped::new(502, "api_error", "server_error"),
        message,
    )
}

/// Anthropic's 529 "overloaded" reaches the client as 429; anything that is not
/// an error status becomes a 502.
fn map_upstream(code: u16) -> Mapped {
    let status = match code {
        529 => 429,
        400..=599 => code,
        _ => 502,
    };
    match status {
        401 => Mapped::new(status, "authentication_error", "authentication_error"),
        402 => Mapped::new(status, "billing_error", "invalid_request_error"),
        403 => Mapped::new(status, "permission_error", "permission_error"),
        404 => Mapped::new(status, "not_found_error", "invalid_request_error"),
        413 => Mapped::new(status, "request_too_large", "invalid_request_error"),
        429 => Mapped {
            openai_code: Some("rate_limit_exceeded"),
            ..Mapped::new(status, "rate_limit_error", "rate_limit_error")
        },
        504 => Mapped::new(status, "timeout_error", "server_error"),
        400..=499 => Mapped::new(status, "invalid_request_error", "invalid_request_error"),
        _ => Mapped::new(status, "api_error", "server_error"),
    }
}

fn is_openai_chat(api_type: &ApiType) -> bool {
    matches!(api_type, ApiType::OpenAiChatCompletions)
}

// Field order is the wire order: serde keeps struct fields in declaration order.

#[derive(Serialize)]
struct AnthropicError<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    error: AnthropicErrorBody<'a>,
}

#[derive(Serialize)]
struct AnthropicErrorBody<'a> {
    #[serde(rename = "type")]
    kind: &'a str,
    message: &'a str,
}

#[derive(Serialize)]
struct OpenAiError<'a> {
    error: OpenAiErrorBody<'a>,
}

#[derive(Serialize)]
struct OpenAiErrorBody<'a> {
    message: &'a str,
    #[serde(rename = "type")]
    kind: &'a str,
    param: Option<&'a str>,
    code: Option<&'a str>,
}

fn body(api_type: &ApiType, mapped: &Mapped, message: &str) -> Vec<u8> {
    let json = if is_openai_chat(api_type) {
        serde_json::to_vec(&OpenAiError {
            error: OpenAiErrorBody {
                message,
                kind: mapped.openai,
                param: None,
                code: mapped.openai_code,
            },
        })
    } else {
        serde_json::to_vec(&AnthropicError {
            kind: "error",
            error: AnthropicErrorBody {
                kind: mapped.anthropic,
                message,
            },
        })
    };
    // Plain strings and integers always serialize.
    json.unwrap_or_default()
}

/// The error response a client of `api_type` receives for `err`.
pub fn client_error(api_type: &ApiType, err: &VkdgError) -> ClientError {
    let mapped = map(err);
    let retry_after = match err {
        VkdgError::UpstreamError { retry_after, .. } => *retry_after,
        _ => None,
    };
    ClientError {
        status: mapped.status,
        retry_after,
        body: body(api_type, &mapped, &err.to_string()),
    }
}

/// The frames that end a stream that fails after the response was committed:
/// Anthropic's `event: error`, or `OpenAI`'s error object followed by `[DONE]`.
/// Ends on a `\n\n` boundary.
pub fn stream_error_frame(api_type: &ApiType, err: &VkdgError) -> Vec<u8> {
    frame(api_type, &map(err), &err.to_string())
}

fn frame(api_type: &ApiType, mapped: &Mapped, message: &str) -> Vec<u8> {
    let json = body(api_type, mapped, message);
    let mut out = Vec::with_capacity(json.len() + 32);
    if is_openai_chat(api_type) {
        out.extend_from_slice(b"data: ");
        out.extend_from_slice(&json);
        out.extend_from_slice(b"\n\ndata: [DONE]\n\n");
    } else {
        out.extend_from_slice(b"event: error\ndata: ");
        out.extend_from_slice(&json);
        out.extend_from_slice(b"\n\n");
    }
    out
}

#[cfg(test)]
mod tests;
