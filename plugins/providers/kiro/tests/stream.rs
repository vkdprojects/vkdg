//! Kiro stream decoding against frames produced by the official AWS encoder.

use aws_smithy_eventstream::frame::write_message_to;
use aws_smithy_types::event_stream::{Header, HeaderValue as AwsValue, Message};
use aws_smithy_types::DateTime;
use bytes::Bytes;
use serde_json::{json, Value};
use vkdg_core::VkdgError;
use vkdg_operations::{ConversationEvent, StopReason, UsageCount};
use vkdg_provider_kiro::eventstream::{EventStreamParser, FrameError, HeaderValue};
use vkdg_provider_kiro::stream_decoder::KiroStreamDecoder;
use vkdg_provider_sdk::ConversationStreamDecoder;

fn encode(message: &Message) -> Vec<u8> {
    let mut buf = Vec::new();
    write_message_to(message, &mut buf).expect("encode");
    buf
}

/// A real-shaped Kiro event frame.
fn event(event_type: &'static str, payload: &Value) -> Vec<u8> {
    encode(
        &Message::new(payload.to_string())
            .add_header(Header::new(
                ":message-type",
                AwsValue::String("event".into()),
            ))
            .add_header(Header::new(
                ":event-type",
                AwsValue::String(event_type.into()),
            ))
            .add_header(Header::new(
                ":content-type",
                AwsValue::String("application/json".into()),
            )),
    )
}

fn exception(kind: &'static str, message: &str) -> Vec<u8> {
    encode(
        &Message::new(json!({ "message": message }).to_string())
            .add_header(Header::new(
                ":message-type",
                AwsValue::String("exception".into()),
            ))
            .add_header(Header::new(
                ":exception-type",
                AwsValue::String(kind.into()),
            ))
            .add_header(Header::new(
                ":content-type",
                AwsValue::String("application/json".into()),
            )),
    )
}

fn text(s: &str) -> Vec<u8> {
    event("assistantResponseEvent", &json!({ "content": s }))
}

/// Feeds `bytes` in `chunk`-sized pieces, then ends the stream.
fn decode(bytes: &[u8], chunk: usize) -> Vec<ConversationEvent> {
    let mut decoder = KiroStreamDecoder::new();
    let mut out: Vec<_> = bytes
        .chunks(chunk)
        .flat_map(|c| decoder.feed(Bytes::copy_from_slice(c)))
        .collect();
    out.extend(decoder.finish());
    out
}

fn texts(events: &[ConversationEvent]) -> String {
    events
        .iter()
        .filter_map(|e| match e {
            ConversationEvent::OutputDelta { delta, .. } => Some(delta.as_str()),
            _ => None,
        })
        .collect()
}

fn failure(events: &[ConversationEvent]) -> Option<(u16, String)> {
    events.iter().find_map(|e| match e {
        ConversationEvent::Failed {
            error: VkdgError::UpstreamError { code, message, .. },
        } => Some((*code, message.clone())),
        _ => None,
    })
}

fn stop_reason(events: &[ConversationEvent]) -> Option<&StopReason> {
    events.iter().find_map(|e| match e {
        ConversationEvent::Completed { stop_reason } => Some(stop_reason),
        _ => None,
    })
}

// Refutes: header parsing that skips the u16 string length, or reads a fixed-width
// type at the wrong size.
#[test]
fn parses_every_aws_header_type() {
    let bytes = encode(
        &Message::new(&b"{}"[..])
            .add_header(Header::new("t", AwsValue::Bool(true)))
            .add_header(Header::new("f", AwsValue::Bool(false)))
            .add_header(Header::new("b", AwsValue::Byte(-2)))
            .add_header(Header::new("s", AwsValue::Int16(-300)))
            .add_header(Header::new("i", AwsValue::Int32(70_000)))
            .add_header(Header::new("l", AwsValue::Int64(-5_000_000_000)))
            .add_header(Header::new(
                "a",
                AwsValue::ByteArray(Bytes::from_static(b"\x00\xff")),
            ))
            .add_header(Header::new("str", AwsValue::String("héllo".into())))
            .add_header(Header::new(
                "ts",
                AwsValue::Timestamp(DateTime::from_millis(1_700_000_000_123)),
            ))
            .add_header(Header::new(
                "u",
                AwsValue::Uuid(0x0102_0304_0506_0708_090a_0b0c_0d0e_0f10),
            )),
    );
    let mut parser = EventStreamParser::new();
    let frames = parser.feed(&bytes);
    assert_eq!(frames.len(), 1);
    let frame = frames.into_iter().next().unwrap().unwrap();
    let values: Vec<_> = frame
        .headers
        .iter()
        .map(|h| (h.name.as_str(), h.value.clone()))
        .collect();
    assert_eq!(
        values,
        vec![
            ("t", HeaderValue::Bool(true)),
            ("f", HeaderValue::Bool(false)),
            ("b", HeaderValue::Byte(-2)),
            ("s", HeaderValue::Int16(-300)),
            ("i", HeaderValue::Int32(70_000)),
            ("l", HeaderValue::Int64(-5_000_000_000)),
            ("a", HeaderValue::ByteArray(Bytes::from_static(b"\x00\xff"))),
            ("str", HeaderValue::String("héllo".into())),
            ("ts", HeaderValue::Timestamp(1_700_000_000_123)),
            (
                "u",
                HeaderValue::Uuid(core::array::from_fn(|i| u8::try_from(i).unwrap_or(0) + 1))
            ),
        ]
    );
    assert_eq!(&frame.payload[..], b"{}");
    assert_eq!(parser.finish(), Ok(()));
}

// Refutes: off-by-12 total_len handling and state that breaks across chunk cuts.
#[test]
fn multi_frame_stream_decodes_identically_at_any_chunking() {
    let bytes = [text("Hello"), text(", "), text("wörld")].concat();
    for chunk in [1, 7, bytes.len()] {
        let events = decode(&bytes, chunk);
        assert!(
            matches!(events.first(), Some(ConversationEvent::Started { .. })),
            "chunk={chunk}"
        );
        assert_eq!(texts(&events), "Hello, wörld", "chunk={chunk}");
        assert!(
            matches!(stop_reason(&events), Some(StopReason::EndTurn)),
            "chunk={chunk}"
        );
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, ConversationEvent::Started { .. }))
                .count(),
            1
        );
        assert!(failure(&events).is_none());
    }
}

// Refutes: skipping CRC validation (silent desync on corrupted bytes).
#[test]
fn corrupted_payload_fails_with_message_crc_error() {
    let good = text("ok");
    let mut bad = text("corrupt me");
    let at = bad.len() - 6; // inside the payload
    bad[at] ^= 0x01;
    let events = decode(&[good, bad, text("never")].concat(), 1);
    assert_eq!(texts(&events), "ok");
    let (code, message) = failure(&events).expect("Failed event");
    assert_eq!(code, 502);
    assert!(message.contains("message CRC"), "{message}");
    assert!(stop_reason(&events).is_none(), "no Completed after failure");
}

#[test]
fn corrupted_prelude_is_rejected_before_waiting_for_length() {
    let mut bad = text("x");
    bad[0] = 0x7f; // absurd total_len; prelude CRC must catch it immediately
    let mut parser = EventStreamParser::new();
    let frames = parser.feed(&bad[..12]);
    assert!(
        matches!(frames.as_slice(), [Err(FrameError::PreludeCrc { .. })]),
        "{frames:?}"
    );
    assert!(parser.feed(&bad[12..]).is_empty(), "parser stays poisoned");
}

#[test]
fn truncated_stream_fails_at_finish() {
    let bytes = [text("partial"), text("cut")].concat();
    let events = decode(&bytes[..bytes.len() - 3], 5);
    assert_eq!(texts(&events), "partial");
    let (_, message) = failure(&events).expect("Failed event");
    assert!(message.contains("ended inside a frame"), "{message}");
    assert!(stop_reason(&events).is_none());
}

// Refutes: emitting a new tool call per fragment, dropping fragments, or EndTurn after tools.
#[test]
fn tool_use_fragments_accumulate_into_one_call() {
    let tool = |input: &str, stop: bool| {
        let mut p = json!({ "toolUseId": "tooluse_1", "name": "get_weather", "input": input });
        if stop {
            p["stop"] = json!(true);
        }
        event("toolUseEvent", &p)
    };
    let bytes = [
        text("Checking."),
        tool("{\"city\":", false),
        tool("\"Paris\"", false),
        tool("}", true),
        event(
            "contextUsageEvent",
            &json!({ "contextUsagePercentage": 1.5 }),
        ),
    ]
    .concat();
    let events = decode(&bytes, 3);

    let calls: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            ConversationEvent::ToolCallDelta {
                tool_use_id,
                name,
                input_delta,
                index,
            } => Some((
                tool_use_id.as_str(),
                name.as_str(),
                input_delta.as_str(),
                *index,
            )),
            _ => None,
        })
        .collect();
    assert_eq!(
        calls[0],
        ("tooluse_1", "get_weather", "", 0),
        "start carries the name once"
    );
    assert!(calls[1..]
        .iter()
        .all(|(id, name, _, idx)| *id == "tooluse_1" && name.is_empty() && *idx == 0));
    let input: String = calls[1..].iter().map(|c| c.2).collect();
    assert_eq!(
        serde_json::from_str::<Value>(&input).unwrap(),
        json!({ "city": "Paris" })
    );
    assert!(matches!(stop_reason(&events), Some(StopReason::ToolUse)));

    // OmniRoute ensureKiroUsage: output = 9 chars / 4 = 2; total = 1.5% of 200k = 3000.
    let usage = events.iter().find_map(|e| match e {
        ConversationEvent::Usage {
            input_tokens,
            output_tokens,
            ..
        } => Some((input_tokens, output_tokens)),
        _ => None,
    });
    assert!(
        matches!(
            usage,
            Some((UsageCount::Estimated(2998), UsageCount::Estimated(2)))
        ),
        "{usage:?}"
    );
    assert!(
        matches!(events.last(), Some(ConversationEvent::Completed { .. })),
        "Completed is terminal"
    );
}

#[test]
fn reported_metadata_usage_wins_over_estimate() {
    let bytes = [
        text("hi"),
        event(
            "metadataEvent",
            &json!({ "usage": { "inputTokens": 42, "outputTokens": 7 } }),
        ),
        event(
            "contextUsageEvent",
            &json!({ "contextUsagePercentage": 50.0 }),
        ),
    ]
    .concat();
    let events = decode(&bytes, bytes.len());
    assert!(events.iter().any(|e| matches!(
        e,
        ConversationEvent::Usage {
            input_tokens: UsageCount::Reported(42),
            output_tokens: UsageCount::Reported(7),
            ..
        }
    )));
}

// Refutes: treating exception frames as ordinary events (or ignoring them).
#[test]
fn exception_frame_fails_with_mapped_status() {
    let bytes = [
        text("partial"),
        exception("ThrottlingException", "slow down"),
        text("after"),
    ]
    .concat();
    let events = decode(&bytes, 4);
    assert_eq!(texts(&events), "partial");
    let (code, message) = failure(&events).expect("Failed event");
    assert_eq!(code, 429);
    assert!(
        message.contains("ThrottlingException") && message.contains("slow down"),
        "{message}"
    );
    assert!(
        stop_reason(&events).is_none(),
        "no Completed after exception"
    );
}

#[test]
fn invalid_state_event_fails() {
    let events = decode(
        &event(
            "invalidStateEvent",
            &json!({ "reason": "INVALID_TASK_ASSIST_PLAN", "message": "bad" }),
        ),
        1,
    );
    assert!(failure(&events).is_some_and(|(_, m)| m.contains("invalidStateEvent")));
    assert!(stop_reason(&events).is_none());
}

// Refutes: merging reasoning text into the visible answer, or dropping it
// entirely. It has its own field name (`text`, not `content`) and its own event.
#[test]
fn reasoning_is_its_own_channel_and_metadata_never_leaks() {
    let bytes = [
        event("meteringEvent", &json!({ "unit": "credit", "usage": 0.1 })),
        event("codeReferenceEvent", &json!({ "references": [] })),
        event(
            "supplementaryWebLinksEvent",
            &json!({ "supplementaryWebLinks": [] }),
        ),
        event(
            "reasoningContentEvent",
            &json!({ "text": "secret thoughts" }),
        ),
        text("answer"),
    ]
    .concat();
    let events = decode(&bytes, 11);
    assert_eq!(
        texts(&events),
        "answer",
        "reasoning must not join the answer"
    );

    let reasoning: String = events
        .iter()
        .filter_map(|e| match e {
            ConversationEvent::ReasoningDelta { delta, .. } => Some(delta.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        reasoning, "secret thoughts",
        "reasoning must reach the client on its own channel: {events:?}"
    );
    assert!(matches!(stop_reason(&events), Some(StopReason::EndTurn)));
}

#[test]
fn empty_stream_fails_instead_of_completing() {
    let events = decode(&[], 1);
    assert!(failure(&events).is_some());
    assert!(stop_reason(&events).is_none());
}

/// Usage as Debug text (`UsageCount` has no `PartialEq`): [input, output, cache read, cache write].
fn usage_of(events: &[ConversationEvent]) -> [String; 4] {
    events
        .iter()
        .find_map(|e| match e {
            ConversationEvent::Usage {
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_creation_tokens,
            } => Some([
                format!("{input_tokens:?}"),
                format!("{output_tokens:?}"),
                format!("{cache_read_tokens:?}"),
                format!("{cache_creation_tokens:?}"),
            ]),
            _ => None,
        })
        .expect("a usage event")
}

// Cache counts were hardcoded to Unknown, so a cached Claude Code prompt was
// metered and priced as fresh input.
#[test]
fn cache_tokens_from_metadata_usage_are_reported() {
    let mut bytes = text("ok");
    bytes.extend(event(
        "metadataEvent",
        &json!({ "usage": { "inputTokens": 1200, "outputTokens": 3,
                           "cacheReadInputTokens": 1000, "cacheWriteInputTokens": 150 } }),
    ));
    let u = usage_of(&decode(&bytes, 7));
    assert_eq!(
        u,
        [
            "Reported(1200)",
            "Reported(3)",
            "Reported(1000)",
            "Reported(150)"
        ]
    );
}

// OmniRoute also reads `usageEvent`; cache counts can come on a frame with no totals.
#[test]
fn usage_event_and_cache_only_frames_are_read() {
    let mut bytes = text("ok");
    bytes.extend(event(
        "usageEvent",
        &json!({ "inputTokens": 40, "outputTokens": 2 }),
    ));
    bytes.extend(event("metricsEvent", &json!({ "cacheReadTokens": 30 })));
    let u = usage_of(&decode(&bytes, 64));
    assert_eq!(
        u,
        ["Reported(40)", "Reported(2)", "Reported(30)", "Unknown"]
    );
}

// The <thinking_mode> directive makes Claude emit reasoning inline rather than
// as separate reasoningContentEvent frames. Tags can span frame boundaries.
#[test]
fn inline_thinking_tags_split_into_reasoning_and_content() {
    let mut bytes = event(
        "assistantResponseEvent",
        &json!({ "content": "<thinking>some reasoning</thinking>answer" }),
    );
    bytes.extend(text("more"));
    let events = decode(&bytes, 64);
    let reasoning: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            ConversationEvent::ReasoningDelta { delta, .. } => Some(delta.as_str()),
            _ => None,
        })
        .collect();
    let content: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            ConversationEvent::OutputDelta { delta, .. } => Some(delta.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(reasoning, ["some reasoning"], "reasoning extracted");
    assert!(
        content.contains(&"answer"),
        "answer in content: {content:?}"
    );
    assert!(
        !content.iter().any(|c| c.contains("<thinking>")),
        "tags stripped from content"
    );
}

#[test]
fn inline_thinking_tag_split_across_frames() {
    // "</thinking" arrives in one frame, ">" in the next.
    let mut bytes = event(
        "assistantResponseEvent",
        &json!({ "content": "<thinking>think</think" }),
    );
    bytes.extend(event(
        "assistantResponseEvent",
        &json!({ "content": "ing>answer" }),
    ));
    let events = decode(&bytes, 128);
    let reasoning: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            ConversationEvent::ReasoningDelta { delta, .. } => Some(delta.as_str()),
            _ => None,
        })
        .collect();
    let content: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            ConversationEvent::OutputDelta { delta, .. } => Some(delta.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(reasoning, ["think"], "cross-frame tag recognised");
    assert!(
        content.contains(&"answer"),
        "answer in content: {content:?}"
    );
}

// Plausible wrong impl: finish() doesn't call flush_thinking(), so a partial
// <thinking> tag buffered in thinking_pending is silently dropped at end of
// stream, producing an incomplete reasoning delta or empty response.
#[test]
fn finish_flushes_partial_thinking_pending() {
    // Stream that ends while thinking_pending still holds a partial closing tag
    // ("</thinking" without ">"). With the fix, the partial content must be
    // emitted; without it, it is silently dropped.
    let bytes = event(
        "assistantResponseEvent",
        &json!({ "content": "<thinking>partial</think" }),
    );
    // No closing "ing>" frame — stream ends abruptly here.
    let events = decode(&bytes, 128);

    // At least a ReasoningDelta or OutputDelta must carry the word "partial".
    let has_partial = events.iter().any(|e| match e {
        ConversationEvent::ReasoningDelta { delta, .. } => delta.contains("partial"),
        ConversationEvent::OutputDelta { delta, .. } => delta.contains("partial"),
        _ => false,
    });
    assert!(
        has_partial,
        "partial thinking content must not be silently dropped on stream end; events={events:?}"
    );
}
