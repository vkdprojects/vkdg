//! Decode Kiro EventStream frames to ConversationEvent.

use serde_json;
use vkdg_operations::{ConversationEvent, StopReason};
use crate::eventstream::Frame;

/// Maps Kiro EventStream frames to ConversationEvent.
pub fn frame_to_event(frame: &Frame) -> Option<ConversationEvent> {
    // Extract event-type: first from headers, fallback to payload JSON
    let event_type = extract_event_type(frame)?;
    
    match event_type.as_str() {
        "messageStart" | "message_start" => {
            Some(ConversationEvent::Started { request_id: vkdg_core::RequestId::new() })
        }
        "contentBlockDelta" | "content_block_delta" => {
            // Extract text from delta.text nested path
            let delta = extract_delta_text(&frame.payload)?;
            let index = extract_json_usize(&frame.payload, "index").unwrap_or(0) as u32;
            Some(ConversationEvent::OutputDelta { delta, index })
        }
        "messageDelta" | "message_delta" => {
            let stop_reason = extract_nested_string(&frame.payload, "delta", "stop_reason")
                .and_then(|s| match s.as_str() {
                    "end_turn" => Some(StopReason::EndTurn),
                    "max_tokens" => Some(StopReason::MaxTokens),
                    "stop_sequence" => Some(StopReason::StopSequence),
                    "tool_use" => Some(StopReason::ToolUse),
                    _ => None,
                })
                .unwrap_or(StopReason::EndTurn);
            Some(ConversationEvent::Completed { stop_reason })
        }
        "metadataEvent" | "metadata" => {
            let stop_reason = extract_json_string(&frame.payload, "stopReason")
                .and_then(|s| match s.as_str() {
                    "END_TURN" => Some(StopReason::EndTurn),
                    "MAX_TOKENS" => Some(StopReason::MaxTokens),
                    "STOP_SEQUENCE" => Some(StopReason::StopSequence),
                    "TOOL_USE" => Some(StopReason::ToolUse),
                    _ => None,
                })
                .unwrap_or(StopReason::EndTurn);
            Some(ConversationEvent::Completed { stop_reason })
        }
        "initial-response" => {
            Some(ConversationEvent::Started { request_id: vkdg_core::RequestId::new() })
        }
        "assistantResponseEvent" => {
            let content = extract_json_string(&frame.payload, "content")?;
            Some(ConversationEvent::OutputDelta { delta: content, index: 0 })
        }
        "contextUsageEvent" => {
            let usage_pct = extract_json_f64(&frame.payload, "contextUsagePercentage")?;
            let output_tokens = (usage_pct * 1000.0) as u32;
            Some(ConversationEvent::Usage {
                input_tokens: vkdg_operations::UsageCount::Estimated(0),
                output_tokens: vkdg_operations::UsageCount::Estimated(output_tokens),
            })
        }
        "meteringEvent" => {
            // Metering events don't map to conversation events
            None
        }
        _ => None,
    }
}

/// Extract event-type from frame (headers first, fallback to payload)
fn extract_event_type(frame: &Frame) -> Option<String> {
    // Try headers first
    if let Some(header) = frame.headers.iter().find(|h| h.name == ":event-type") {
        if let crate::eventstream::HeaderValue::String(s) = &header.value {
            return Some(s.clone());
        }
        if let crate::eventstream::HeaderValue::ByteArray(b) = &header.value {
            if let Ok(s) = std::str::from_utf8(b) {
                return Some(s.to_string());
            }
        }
    }
    
    // Fallback: extract from payload JSON
    let value: serde_json::Value = serde_json::from_slice(&frame.payload).ok()?;
    value.get("type")?.as_str().map(|s| s.to_string())
}

/// Extract delta.text from content_block_delta payload
fn extract_delta_text(payload: &[u8]) -> Option<String> {
    let value: serde_json::Value = serde_json::from_slice(payload).ok()?;
    
    // Try nested path: delta.text
    if let Some(delta) = value.get("delta") {
        if let Some(text) = delta.get("text").and_then(|t| t.as_str()) {
            return Some(text.to_string());
        }
    }
    
    // Fallback: direct text field
    value.get("text").and_then(|t| t.as_str()).map(|s| s.to_string())
}

fn extract_json_string(payload: &[u8], key: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_slice(payload).ok()?;
    value.get(key).and_then(|v| v.as_str()).map(String::from)
}

fn extract_nested_string(payload: &[u8], parent: &str, child: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_slice(payload).ok()?;
    value.get(parent).and_then(|p| p.get(child)).and_then(|v| v.as_str()).map(String::from)
}

fn extract_json_usize(payload: &[u8], key: &str) -> Option<usize> {
    let value: serde_json::Value = serde_json::from_slice(payload).ok()?;
    value.get(key).and_then(|v| v.as_u64()).map(|v| v as usize)
}

fn extract_json_f64(payload: &[u8], key: &str) -> Option<f64> {
    let value: serde_json::Value = serde_json::from_slice(payload).ok()?;
    value.get(key).and_then(|v| v.as_f64())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes;

    #[test]
    fn test_extract_delta_text() {
        let payload = br#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hello"}}"#;
        let result = extract_delta_text(payload);
        assert_eq!(result, Some("hello".to_string()));
    }

    #[test]
    fn test_metadata_event_end_turn() {
        let payload = br#"{"stopReason":"END_TURN"}"#;
        let result = extract_json_string(payload, "stopReason");
        assert_eq!(result, Some("END_TURN".to_string()));
    }

    /// End-to-end test: parse real fixture through EventStreamParser -> frame_to_event -> ConversationEvent
    #[test]
    fn test_e2e_fixture_parsing() {
        let fixture = include_bytes!("../tests/fixtures/generate_assistant_response.eventstream");
        
        // Parse frames through EventStreamParser
        let mut parser = crate::eventstream::EventStreamParser::new();
        let frames = parser.feed(bytes::Bytes::from(fixture.to_vec()));
        
        // Should parse all 6 frames
        assert_eq!(frames.len(), 6, "Expected 6 frames from fixture");
        
        // Map frames to ConversationEvent using frame_to_event
        let events: Vec<_> = frames.iter()
            .filter_map(frame_to_event)
            .collect();
        
        // Should get at least the assistant responses and completion
        let output_deltas: Vec<_> = events.iter()
            .filter_map(|e| match e {
                ConversationEvent::OutputDelta { delta, .. } => Some(delta.clone()),
                _ => None,
            })
            .collect();
        
        // Should have 2 OutputDelta events: "hello" and " from kiro"
        assert_eq!(output_deltas.len(), 2, "Expected 2 OutputDelta events, got {:?}", output_deltas);
        assert_eq!(output_deltas[0], "hello");
        assert_eq!(output_deltas[1], " from kiro");
        
        // Should have at least 1 Completed event (messageDelta and/or metadataEvent)
        let completed_count = events.iter().filter(|e| matches!(e, ConversationEvent::Completed { .. })).count();
        assert!(completed_count >= 1, "Expected at least 1 Completed event, got {}", completed_count);
    }
}
