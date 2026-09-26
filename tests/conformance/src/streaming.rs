// Phase B streaming conformance tests.
//
// Test-first: each test documents the plausible wrong implementation it catches.
// SseParser returns Vec<SseEvent> (raw SSE level), not ConversationEvent.
// All tests COMPILE and fail behaviorally (never on import error).
//
// Edge table covered:
//   fragmentation at any byte boundary, partial UTF-8, empty event, [DONE], EOF

use std::sync::Arc;

use http_body::Body as HttpBody;
use vkdg_connections::{
    AuthKind, ConnectionCatalog, ConnectionConfig, CredentialManager, ProviderKind,
};
use vkdg_core::pipeline::PipelineCtx;
use vkdg_core::{ApiType, ClientId, ConnectionId, RequestEnvelope, RequestId, TenantId};
use vkdg_http::pipeline::run_conversation_pipeline;
use vkdg_http::sse::SseParser;
use vkdg_http::upstream::HttpClient;
use vkdg_http::{AdmissionGuard, PipelineState};
use vkdg_ingress_anthropic::encode_event;
use vkdg_observe::DecisionRecordExporter;
use vkdg_operations::{
    CapabilitySet, ConversationEvent, ConversationRequest, Message, MessageContent, Operation,
    Role, StopReason,
};
use vkdg_routing::{PluginHooks, RouteConfig, RouteId, Router, StrategyKind};

use crate::fake_upstream::{FakeUpstream, FakeUpstreamBehavior};
use vkdg_provider_anthropic::AnthropicAdapter;
use vkdg_provider_openai::OpenAIAdapter;
use vkdg_provider_sdk::ProviderRegistry;

// ── Shared helpers for pipeline tests ─────────────────────────────────────────

fn make_streaming_pipeline(base_url: String) -> Arc<PipelineState> {
    let conn_id = ConnectionId("stream-bp".into());
    let config = ConnectionConfig {
        id: conn_id.clone(),
        provider: ProviderKind::Custom { base_url },
        auth: AuthKind::ApiKey {
            env_var: "VKDG_SMOKE_KEY".into(),
        },
        models: vec!["claude-*".into()],
        max_concurrent: 10,
        weight: 1,
        tags: vec![],
        capabilities: CapabilitySet::default(),
    };
    let route = RouteConfig {
        id: RouteId("stream-bp".into()),
        match_models: vec!["claude-*".into()],
        strategy: StrategyKind::RoundRobin,
        targets: vec![conn_id],
        plugin_hooks: PluginHooks::default(),
    };
    Arc::new(PipelineState {
        admission: Arc::new(AdmissionGuard::new(10)),
        router: Arc::new(Router::new(vec![route])),
        catalog: Arc::new(ConnectionCatalog::new(vec![config])),
        credentials: Arc::new(CredentialManager::new()),
        http_client: Arc::new(HttpClient::new()),
        exporter: Arc::new(DecisionRecordExporter::new()),
        provider_registry: {
            let mut r = ProviderRegistry::empty();
            r.register(Arc::new(AnthropicAdapter));
            r.register(Arc::new(OpenAIAdapter));
            Arc::new(r)
        },
        cache: None,
        combo_resolver: None,
        compressor: None,
        dedup_table: None,
        session_registry: None,
        quota_tracker: None,
        global_system_prompt: None,
        ip_policy: None,
        latency_tracker: None,
        memory_store: None,
        eval_enabled: false,
        relay_enabled: false,
    })
}

// ── encode_event (already implemented, GREEN) ──────────────────────────────────

/// Plausible wrong impl: encode_event omits the leading "data: " prefix,
/// breaking SSE client parsers that require it.
/// PASSES.
#[test]
fn encode_event_output_delta_produces_sse_data_line() {
    let event = ConversationEvent::OutputDelta {
        delta: "hello".to_string(),
        index: 0,
    };
    let encoded = encode_event(&event);
    assert!(
        encoded.starts_with("data: "),
        "SSE event must start with 'data: '; got: {encoded:?}"
    );
    assert!(
        encoded.ends_with("\n\n"),
        "SSE event must end with double newline; got: {encoded:?}"
    );
}

/// Plausible wrong impl: Completed event doesn't append the [DONE] sentinel,
/// so clients never know the stream has ended cleanly.
/// PASSES.
#[test]
fn encode_event_completed_emits_done_frame() {
    let event = ConversationEvent::Completed {
        stop_reason: StopReason::EndTurn,
    };
    let encoded = encode_event(&event);
    assert!(
        encoded.contains("data: [DONE]"),
        "Completed must include 'data: [DONE]' frame; got: {encoded:?}"
    );
}

// ── SseParser: basic round-trip ────────────────────────────────────────────────

/// Plausible wrong impl: push() treats a partial buffer as a complete event
/// and emits it early, producing a garbled payload.
/// RED: fails if parser emits an event before the \n\n terminator arrives.
#[test]
fn parser_does_not_emit_partial_event() {
    let mut parser = SseParser::new();
    // Only the first half of a complete frame — no \n\n yet.
    let events = parser.push(b"data: {\"hello\":");
    assert!(
        events.is_empty(),
        "Must not emit event from partial chunk (no \\n\\n yet); got {:?}",
        events
    );
}

/// Plausible wrong impl: parser requires chunk alignment with event boundaries
/// and drops any bytes that don't start a new event.
/// RED: fails if fragmentation at an arbitrary byte causes data loss.
#[test]
fn parser_reassembles_event_split_at_arbitrary_byte() {
    let full_frame = b"data: hello world\n\n";
    // Try every possible split point.
    for split in 1..full_frame.len() {
        let mut parser = SseParser::new();
        let (a, b) = full_frame.split_at(split);

        let ea = parser.push(a);
        assert!(
            ea.is_empty(),
            "split_at={split}: partial chunk must not emit event; got {:?}",
            ea
        );

        let eb = parser.push(b);
        assert_eq!(
            eb.len(),
            1,
            "split_at={split}: completing the frame must emit exactly 1 event; got {:?}",
            eb
        );
        assert_eq!(
            eb[0].data, "hello world",
            "split_at={split}: data field must be intact after reassembly"
        );
    }
}

/// Plausible wrong impl: parser flushes its buffer between calls, so the
/// second push sees no prior data and never finds the delimiter.
/// RED: fails if state isn't preserved across push() calls.
#[test]
fn parser_preserves_buffer_across_push_calls() {
    let mut parser = SseParser::new();
    // First push: data line without terminator.
    let _ = parser.push(b"data: payload\n");
    // Second push: just the terminator.
    let events = parser.push(b"\n");
    assert_eq!(
        events.len(),
        1,
        "Delimiter arriving in a separate push must complete the buffered event; got {:?}",
        events
    );
    assert_eq!(events[0].data, "payload");
}

// ── SseParser: [DONE] sentinel ────────────────────────────────────────────────

/// Plausible wrong impl: parser treats "data: [DONE]" as a normal data event
/// and emits a ConversationEvent, confusing the downstream consumer.
/// For the SSE layer: [DONE] IS emitted as an SseEvent with data="[DONE]".
/// The CALLER (pipeline) is responsible for recognising and stopping.
/// RED: fails if parser silently drops [DONE] instead of surfacing it.
#[test]
fn parser_emits_done_sentinel_as_sse_event() {
    let mut parser = SseParser::new();
    let events = parser.push(b"data: [DONE]\n\n");
    assert_eq!(
        events.len(),
        1,
        "[DONE] frame must produce exactly one SseEvent so the caller can detect end-of-stream"
    );
    assert_eq!(
        events[0].data, "[DONE]",
        "SseEvent data must be the literal string '[DONE]'"
    );
}

// ── SseParser: empty event ─────────────────────────────────────────────────────

/// Plausible wrong impl: empty event (just \n\n, no data line) panics or
/// crashes instead of being silently dropped.
/// RED: fails if parser errors on a bare blank line.
#[test]
fn parser_drops_empty_event_without_panic() {
    let mut parser = SseParser::new();
    // A bare double-newline is a heartbeat / keep-alive, not a data event.
    let events = parser.push(b"\n\n");
    assert!(
        events.is_empty(),
        "Empty event (bare \\n\\n) must be silently dropped; got {:?}",
        events
    );
}

/// Plausible wrong impl: comment-only events (lines starting with ':') are
/// treated as data, polluting the event stream with keep-alive noise.
/// RED: fails if comments leak through as SseEvents.
#[test]
fn parser_drops_comment_only_event() {
    let mut parser = SseParser::new();
    let events = parser.push(b": keep-alive\n\n");
    assert!(
        events.is_empty(),
        "Comment-only event must be dropped; got {:?}",
        events
    );
}

// ── SseParser: multiple events in one push ────────────────────────────────────

/// Plausible wrong impl: parser stops after the first event in a chunk,
/// dropping subsequent events that arrived in the same TCP read.
/// RED: fails if only the first of two events in a single push is returned.
#[test]
fn parser_extracts_multiple_events_from_one_push() {
    let mut parser = SseParser::new();
    let chunk = b"data: first\n\ndata: second\n\n";
    let events = parser.push(chunk);
    assert_eq!(
        events.len(),
        2,
        "Two complete events in one push must both be returned; got {:?}",
        events
    );
    assert_eq!(events[0].data, "first");
    assert_eq!(events[1].data, "second");
}

// ── SseParser: event type field ───────────────────────────────────────────────

/// Plausible wrong impl: the `event:` field is ignored, so typed events
/// (e.g. "event: ping") lose their type tag downstream.
/// RED: fails if event_type is None when an `event:` line was present.
#[test]
fn parser_captures_event_type_field() {
    let mut parser = SseParser::new();
    let events = parser.push(b"event: ping\ndata: {}\n\n");
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].event_type.as_deref(),
        Some("ping"),
        "event_type must be captured from the 'event:' field"
    );
}

// ── SseParser: EOF / partial frame at stream end ──────────────────────────────

/// Plausible wrong impl: a partial frame left in the buffer at EOF is silently
/// emitted as a complete event, producing corrupted data.
/// RED: fails if push() of non-terminated bytes eventually leaks an event.
#[test]
fn parser_does_not_emit_unterminated_frame_at_eof() {
    let mut parser = SseParser::new();
    // This chunk never gets a \n\n — simulates a stream cut short.
    let events = parser.push(b"data: incomplete");
    assert!(
        events.is_empty(),
        "Unterminated frame must not be emitted; got {:?}",
        events
    );
    // A second push with more data (but still no terminator) also must not emit.
    let events2 = parser.push(b" continues but never ends");
    assert!(
        events2.is_empty(),
        "Continued unterminated frame must not emit; got {:?}",
        events2
    );
}

// ── Slow client ───────────────────────────────────────────────────────────────

/// Plausible wrong impl: pipeline collects the entire upstream body into Bytes before
/// returning the Response (e.g. resp.bytes().await instead of resp.bytes_stream()),
/// causing OOM with long responses and breaking backpressure to the upstream reader.
///
/// Proof: Body::from_stream(stream) has size_hint().upper() == None (unknown length,
/// truly lazy). Body::from(bytes) has size_hint().upper() == Some(n) (pre-buffered).
/// A regression to eager buffering would produce Some(n) and this test would fail.
#[tokio::test]
async fn slow_client_backpressure_no_unbounded_buffer() {
    std::env::set_var("VKDG_SMOKE_KEY", "test-token");

    let fake = FakeUpstream::spawn(FakeUpstreamBehavior::AnthropicStreamOk {
        content: "backpressure-probe".into(),
    })
    .await;

    let pipeline = make_streaming_pipeline(fake.base_url.clone());
    let envelope = RequestEnvelope {
        request_id: RequestId::new(),
        client_id: ClientId("bp-test".into()),
        tenant_id: TenantId("default".into()),
        session_key: None,
        api_type: ApiType::AnthropicMessages,
        model_requested: "claude-3-5-haiku-20241022".into(),
        deadline: None,
        mode_pack_override: None,
        compression_override: None,
        cache_bypass: false,
        include_think_tags: false,
        client_ip: None,
    };
    let op = Operation::Conversation(ConversationRequest {
        messages: vec![Message {
            role: Role::User,
            content: MessageContent::Text("ping".into()),
        }],
        tools: vec![],
        max_tokens: Some(10),
        temperature: None,
        stream: true,
        system: None,
        required_capabilities: CapabilitySet::default(),
    });
    let ctx = PipelineCtx::new(envelope);

    let response = run_conversation_pipeline(pipeline, ctx, op).await;

    // 1. Content-type proves the streaming code path was taken.
    let ct = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        ct.contains("text/event-stream"),
        "streaming response must have content-type: text/event-stream, got: {ct}"
    );

    // 2. size_hint().upper() == None proves the body is a lazy stream, not a
    //    pre-collected Bytes buffer.  Body::from_stream → (0, None).
    //    Body::from(bytes) → (n, Some(n)).  A regression to eager buffering fails here.
    let upper = response.body().size_hint().upper();
    assert!(
        upper.is_none(),
        "streaming body must not have a fixed upper size hint (got Some({upper:?})): \
         this indicates eager buffering instead of lazy Body::from_stream"
    );

    // 3. Drain and verify the body is well-formed SSE — functional sanity check.
    let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("body must be readable");
    let body_str = std::str::from_utf8(&body).expect("body must be valid UTF-8");
    assert!(
        body_str.contains("data: "),
        "body must contain SSE data frames, got: {body_str}"
    );
    assert!(
        body_str.contains("[DONE]"),
        "body must contain [DONE] sentinel, got: {body_str}"
    );
    assert_eq!(fake.call_count(), 1, "upstream must be called exactly once");
}

// ── think-tag stripping (scenario: think_tags_stripped_by_default) ────────────

/// Plausible wrong impl: think tags not stripped when strip_think_tags=true.
/// Scenario: spec/scenarios/think_tags_stripped_by_default.yaml
/// PASSES — strip_think_tags is already implemented.
#[test]
fn sse_parser_strips_think_tags_by_default() {
    use vkdg_http::sse::strip_think_tags;
    let input = "<think>internal reasoning</think>The answer is 42";
    let output = strip_think_tags(input);
    assert_eq!(
        output, "The answer is 42",
        "think tags must be stripped from output: got {output:?}"
    );
    assert!(
        !output.contains("reasoning"),
        "reasoning content must not appear in output after stripping: got {output:?}"
    );
}

/// Plausible wrong impl: stripping modifies content outside think blocks,
/// corrupting the factual part of the response.
/// Scenario: spec/scenarios/think_tags_stripped_by_default.yaml (invariant: text outside not modified)
/// PASSES.
#[test]
fn sse_parser_think_stripping_preserves_content() {
    use vkdg_http::sse::strip_think_tags;
    let input = "Before<think>remove this</think>after";
    assert_eq!(
        strip_think_tags(input),
        "Beforeafter",
        "content outside think blocks must be preserved verbatim"
    );
}

/// Plausible wrong impl: SseParser.new() has strip_think_tags=false by default,
/// leaking reasoning tokens to clients that don't opt in.
/// PASSES — default is true.
#[test]
fn sse_parser_defaults_to_stripping_think_tags() {
    let p = vkdg_http::sse::SseParser::new();
    assert!(
        p.strip_think_tags,
        "SseParser must default to strip_think_tags=true per spec/scenarios/think_tags_stripped_by_default.yaml"
    );
}

/// Plausible wrong impl: with_think_tags() does not disable stripping,
/// so clients that opt in still have reasoning stripped.
/// PASSES.
#[test]
fn sse_parser_with_think_tags_disables_stripping() {
    let p = vkdg_http::sse::SseParser::new().with_think_tags();
    assert!(
        !p.strip_think_tags,
        "SseParser::with_think_tags() must set strip_think_tags=false (X-VKDG-Think-Tags: include)"
    );
}
