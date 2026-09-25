// Contract: decode_request parses valid Anthropic JSON into (model, Operation::Conversation);
//           invalid JSON returns Err(VkdgError::ConfigInvalid { field: "body", .. }).
// Phase B: pipeline is wired — 501 stub behavior no longer exists.

use axum::body::to_bytes;
use bytes::Bytes;
use http::{Method, Request, StatusCode};
use tower::ServiceExt;
use vkdg_core::VkdgError;
use vkdg_http::{AppState, ServerConfig, build_router};
use vkdg_ingress_anthropic::decode_request;
use vkdg_operations::Operation;

/// Contract: valid Anthropic request body decodes to Conversation operation.
/// PASSES.
#[tokio::test]
async fn decode_valid_messages_request() {
    let body = Bytes::from(
        r#"{
            "model": "claude-3-5-sonnet-20241022",
            "max_tokens": 1024,
            "messages": [
                {"role": "user", "content": "Hello, world!"}
            ]
        }"#,
    );

    let result = decode_request(body);
    assert!(result.is_ok(), "decode_request must succeed on valid body; got {:?}", result);

    let (model, op) = result.unwrap();
    assert_eq!(model, "claude-3-5-sonnet-20241022");

    match op {
        Operation::Conversation(req) => {
            assert_eq!(req.messages.len(), 1);
            assert!(!req.stream, "stream defaults to false when absent");
        }
        other => panic!("Expected Operation::Conversation, got {:?}", other),
    }
}

/// Contract: valid request with stream=true preserves the flag.
/// PASSES.
#[tokio::test]
async fn decode_valid_stream_flag_preserved() {
    let body = Bytes::from(
        r#"{
            "model": "claude-3-opus-20240229",
            "max_tokens": 512,
            "stream": true,
            "messages": [{"role": "user", "content": "hi"}]
        }"#,
    );

    let (_, op) = decode_request(body).unwrap();
    match op {
        Operation::Conversation(req) => {
            assert!(req.stream, "stream=true must be preserved through decode");
        }
        other => panic!("Expected Conversation, got {:?}", other),
    }
}

/// Contract: invalid JSON body returns Err(VkdgError::ConfigInvalid { field: "body" }).
/// PASSES.
#[tokio::test]
async fn decode_invalid_json_returns_config_invalid() {
    let body = Bytes::from("not json at all }{");
    let result = decode_request(body);

    assert!(
        matches!(result, Err(VkdgError::ConfigInvalid { ref field, .. }) if field == "body"),
        "invalid JSON must produce ConfigInvalid{{field:\"body\"}}; got {:?}",
        result
    );
}

/// Contract: empty body returns Err(VkdgError::ConfigInvalid { field: "body" }).
/// PASSES.
#[tokio::test]
async fn decode_empty_body_returns_config_invalid() {
    let result = decode_request(Bytes::new());
    assert!(
        matches!(result, Err(VkdgError::ConfigInvalid { ref field, .. }) if field == "body"),
        "empty body must produce ConfigInvalid{{field:\"body\"}}; got {:?}",
        result
    );
}

