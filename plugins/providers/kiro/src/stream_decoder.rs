//! Kiro stream decoder: AWS EventStream framing + Kiro event mapping.

use bytes::Bytes;
use vkdg_operations::ConversationEvent;
use vkdg_provider_sdk::ConversationStreamDecoder;

use crate::decode::KiroEventDecoder;
use crate::eventstream::EventStreamParser;

#[derive(Debug, Default)]
pub struct KiroStreamDecoder {
    parser: EventStreamParser,
    events: KiroEventDecoder,
}

impl KiroStreamDecoder {
    pub fn new() -> Self {
        Self::default()
    }
}

impl ConversationStreamDecoder for KiroStreamDecoder {
    fn feed(&mut self, chunk: Bytes) -> Vec<ConversationEvent> {
        let mut out = Vec::new();
        for frame in self.parser.feed(&chunk) {
            match frame {
                Ok(frame) => self.events.on_frame(&frame, &mut out),
                Err(err) => self.events.on_frame_error(err, &mut out),
            }
        }
        out
    }

    fn finish(&mut self) -> Vec<ConversationEvent> {
        let mut out = Vec::new();
        match self.parser.finish() {
            Ok(()) => self.events.finish(&mut out),
            Err(err) => self.events.on_frame_error(err, &mut out),
        }
        out
    }
}
