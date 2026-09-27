//! Kiro stream decoder: wraps EventStreamParser and maps frames to ConversationEvent.

use bytes::Bytes;
use vkdg_operations::ConversationEvent;
use vkdg_provider_sdk::ConversationStreamDecoder;

use crate::decode::frame_to_event;
use crate::eventstream::EventStreamParser;

/// Kiro's stream decoder that converts AWS Event Stream frames to ConversationEvent.
pub struct KiroStreamDecoder {
    parser: EventStreamParser,
}

impl KiroStreamDecoder {
    pub fn new() -> Self {
        Self {
            parser: EventStreamParser::new(),
        }
    }
}

impl Default for KiroStreamDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl ConversationStreamDecoder for KiroStreamDecoder {
    fn feed(&mut self, chunk: Bytes) -> Vec<ConversationEvent> {
        // Get frames from parser
        let frames = self.parser.feed(chunk);

        // Map frames to events
        let mut events = Vec::new();
        for frame in frames {
            if let Some(event) = frame_to_event(&frame) {
                events.push(event);
            }
        }

        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decoder_creation() {
        let decoder = KiroStreamDecoder::new();
        let _ = decoder;
    }

    #[test]
    fn test_feed_returns_empty_for_invalid() {
        let mut decoder = KiroStreamDecoder::new();
        let events = decoder.feed(Bytes::from(b"invalid data".to_vec()));
        assert!(events.is_empty());
    }
}
