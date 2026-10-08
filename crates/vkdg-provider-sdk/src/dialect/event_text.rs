//! Test support: renders canonical events as one comparable line each.
//!
//! `ConversationEvent` has no `PartialEq` (and `Started` carries a random id), so tests
//! compare these hand-written lines instead:
//!
//! ```text
//! started
//! text[0] "Hello"                       reasoning[0] "hm"
//! tool[1] id="toolu_1" name="f" args="{}"      tool_end[1]
//! usage in=R25 out=R1 cache_read=- cache_create=-    (R reported, E estimated, - unknown)
//! completed EndTurn
//! failed 502 "message"
//! ```

use bytes::Bytes;
use vkdg_core::VkdgError;
use vkdg_operations::{ConversationEvent, UsageCount};

use crate::ConversationStreamDecoder;

fn count(c: &UsageCount) -> String {
    match c {
        UsageCount::Reported(n) => format!("R{n}"),
        UsageCount::Estimated(n) => format!("E{n}"),
        UsageCount::Unknown => "-".to_owned(),
    }
}

pub(super) fn render(event: &ConversationEvent) -> String {
    match event {
        ConversationEvent::Started { .. } => "started".to_owned(),
        ConversationEvent::OutputDelta { delta, index } => format!("text[{index}] {delta:?}"),
        ConversationEvent::ReasoningDelta { delta, index } => {
            format!("reasoning[{index}] {delta:?}")
        }
        ConversationEvent::ToolCallDelta {
            tool_use_id,
            name,
            input_delta,
            index,
        } => {
            format!("tool[{index}] id={tool_use_id:?} name={name:?} args={input_delta:?}")
        }
        ConversationEvent::ToolCallEnd { index } => format!("tool_end[{index}]"),
        ConversationEvent::Usage {
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_creation_tokens,
        } => format!(
            "usage in={} out={} cache_read={} cache_create={}",
            count(input_tokens),
            count(output_tokens),
            count(cache_read_tokens),
            count(cache_creation_tokens)
        ),
        ConversationEvent::Completed { stop_reason } => format!("completed {stop_reason:?}"),
        ConversationEvent::Failed {
            error: VkdgError::UpstreamError { code, message, .. },
        } => {
            format!("failed {code} {message:?}")
        }
        ConversationEvent::Failed { error } => format!("failed {error:?}"),
    }
}

pub(super) fn render_all(events: &[ConversationEvent]) -> Vec<String> {
    events.iter().map(render).collect()
}

/// Feeds `chunks` then signals EOF; returns every event rendered.
pub(super) fn decode<'a>(
    decoder: &mut dyn ConversationStreamDecoder,
    chunks: impl IntoIterator<Item = &'a [u8]>,
) -> Vec<String> {
    let mut events = Vec::new();
    for chunk in chunks {
        events.extend(decoder.feed(Bytes::copy_from_slice(chunk)));
    }
    events.extend(decoder.finish());
    render_all(&events)
}

/// Asserts that `input` decodes to exactly `expected`, unsplit, split in two at every byte
/// boundary, and delivered one byte at a time.
pub(super) fn assert_events_at_every_boundary(
    make: fn() -> Box<dyn ConversationStreamDecoder>,
    input: &[u8],
    expected: &[&str],
) {
    let expected: Vec<String> = expected.iter().map(|&line| line.to_owned()).collect();
    assert_eq!(decode(make().as_mut(), [input]), expected, "unsplit");
    for cut in 0..=input.len() {
        let head = &input[..cut];
        let tail = &input[cut..];
        assert_eq!(
            decode(make().as_mut(), [head, tail]),
            expected,
            "split at byte {cut}"
        );
    }
    assert_eq!(
        decode(make().as_mut(), input.chunks(1)),
        expected,
        "one byte at a time"
    );
}

/// Builds SSE text from `(event, data)` frames; an empty event name omits the `event:` line.
pub(super) fn sse(frames: &[(&str, &str)]) -> String {
    let mut out = String::new();
    for (event, data) in frames {
        if !event.is_empty() {
            out.push_str("event: ");
            out.push_str(event);
            out.push('\n');
        }
        out.push_str("data: ");
        out.push_str(data);
        out.push_str("\n\n");
    }
    out
}
