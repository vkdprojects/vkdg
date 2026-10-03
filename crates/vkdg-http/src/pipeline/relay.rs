//! Relay of an upstream response to the client in the client's dialect.
//!
//! Three arms, chosen per response by [`plan`]:
//! - passthrough: same dialect (or a provider the gateway cannot translate):
//!   bytes go through untouched and unparsed;
//! - translate: upstream and client speak different dialects, so the response is
//!   decoded to [`ConversationEvent`]s and re-encoded for the client, stream and
//!   non-stream alike through the same event vocabulary;
//! - provider decoder: the adapter owns a decoder for a non-JSON protocol (Kiro).

use std::pin::Pin;

use bytes::Bytes;
use futures::{Stream, StreamExt};
use vkdg_connections::ConnectionConfig;
use vkdg_core::{ApiType, VkdgError};
use vkdg_operations::{ConversationEvent, StreamContext, StreamEncoder, WireFormat};
use vkdg_provider_sdk::{ConversationStreamDecoder, ProviderAdapter};

/// How one upstream response reaches the client.
pub(super) enum Relay {
    Passthrough,
    /// Upstream speaks `from`; the client speaks the other dialect.
    Translate {
        from: WireFormat,
    },
    /// The adapter decodes its own protocol (its decoder wins over `wire_format`).
    ProviderDecoder(Box<dyn ConversationStreamDecoder>),
}

/// Chooses the arm for a response from `adapter` over `config` to a `client`.
pub(super) fn plan(
    adapter: &dyn ProviderAdapter,
    config: &ConnectionConfig,
    client: &ApiType,
) -> Relay {
    if let Some(decoder) = adapter.stream_decoder() {
        return Relay::ProviderDecoder(decoder);
    }
    match (adapter.wire_format(config), WireFormat::of_client(client)) {
        (Some(upstream), Some(client)) if upstream != client => Relay::Translate { from: upstream },
        _ => Relay::Passthrough,
    }
}

/// Looks ahead in a provider-decoded stream for an account-level failure, before
/// any byte is committed to the client.
///
/// A provider such as Kiro answers HTTP 200 and reports "throttled" or "out of
/// credits" as the first frame of the stream. Seen after the response is
/// committed, that failure can neither cool the connection nor move the request
/// to a sibling. So the upstream is read until its first *substantive* decoded
/// event (`Started` carries nothing for the client and is skipped):
///
/// - that event is a `Failed` whose status [`fails_over`](super::helpers::fails_over) → `Err` with the
///   upstream error; the caller cools the connection and the request is retried
///   on a sibling. Nothing was sent to the client.
/// - any other event, a `Failed` with another status, an upstream I/O error or
///   the end of the stream → `Ok`, and the returned stream replays every chunk
///   read so far ahead of the rest of the upstream, byte for byte. The relay then
///   decodes it from the start exactly as if no look-ahead had happened.
///
/// Only the chunks up to the first event are held (usually one); the rest of the
/// stream is not buffered and is polled only as the client asks, so backpressure
/// and cancellation are unchanged. A `Failed` after a normal first event is not
/// examined: by then the client has the response and a retry is unsafe.
///
/// The look-ahead decodes with a separate decoder instance
/// ([`ProviderAdapter::stream_decoder`] builds a fresh one per call), so the
/// relay's own decoder state is untouched.
pub(super) async fn guard_first_event(
    adapter: &dyn ProviderAdapter,
    mut body: ByteStream,
) -> Result<ByteStream, VkdgError> {
    // The adapters that own a decoder are exactly those `plan` relays through
    // `Relay::ProviderDecoder`; everything else is passthrough or translated.
    let Some(mut probe) = adapter.stream_decoder() else {
        return Ok(body);
    };

    let mut seen: Vec<Bytes> = Vec::new();
    let tail = loop {
        match body.next().await {
            Some(Ok(chunk)) => {
                let events = probe.feed(chunk.clone());
                seen.push(chunk);
                if let Some(error) = failover_error(&events) {
                    return Err(error);
                }
                if first_substantive(&events).is_some() {
                    break Tail::Open;
                }
            }
            Some(Err(e)) => break Tail::Failed(e),
            None => {
                if let Some(error) = failover_error(&probe.finish()) {
                    return Err(error);
                }
                break Tail::Ended;
            }
        }
    };
    Ok(replay(seen, tail, body))
}

/// How the upstream stood when the look-ahead stopped.
enum Tail {
    /// More may follow: the rest of the upstream is chained after the replay.
    Open,
    /// Already ended; polling it again is not allowed.
    Ended,
    /// Failed while read; the client gets the same error after the replay.
    Failed(std::io::Error),
}

/// The first event that is not `Started`.
fn first_substantive(events: &[ConversationEvent]) -> Option<&ConversationEvent> {
    events
        .iter()
        .find(|e| !matches!(e, ConversationEvent::Started { .. }))
}

/// The upstream error when the first substantive event is a `Failed` that
/// [`fails_over`](super::helpers::fails_over).
fn failover_error(events: &[ConversationEvent]) -> Option<VkdgError> {
    match first_substantive(events)? {
        ConversationEvent::Failed {
            error:
                VkdgError::UpstreamError {
                    code,
                    message,
                    retry_after,
                },
        } if super::helpers::fails_over(*code) => Some(VkdgError::UpstreamError {
            code: *code,
            message: message.clone(),
            retry_after: *retry_after,
        }),
        _ => None,
    }
}

/// `seen` chunks, then whatever `tail` says follows.
fn replay(seen: Vec<Bytes>, tail: Tail, body: ByteStream) -> ByteStream {
    let seen = futures::stream::iter(seen.into_iter().map(Ok));
    match tail {
        Tail::Open => Box::pin(seen.chain(body)),
        Tail::Ended => Box::pin(seen),
        Tail::Failed(e) => Box::pin(seen.chain(futures::stream::once(async move { Err(e) }))),
    }
}

/// The streaming body the client receives for `body`.
pub(super) fn relay_stream(
    relay: Relay,
    body: ByteStream,
    client: &ApiType,
    ctx: &StreamContext<'_>,
) -> ByteStream {
    match relay {
        Relay::Passthrough => body,
        Relay::Translate { from } => {
            decode_stream_to_sse(body, vkdg_provider_sdk::sse_decoder_for(from), client, ctx)
        }
        Relay::ProviderDecoder(decoder) => decode_stream_to_sse(body, decoder, client, ctx),
    }
}

/// The streaming body the client receives: the relay arm for `relay`, then what
/// every streaming response gets.
///
/// - think tags are removed unless the client opted in;
/// - the termination guard ends a stream that closes without a terminal marker
///   with an error event in the client's dialect;
/// - a heartbeat comment keeps the socket alive while the model thinks.
pub(super) fn into_client_stream(
    relay: Relay,
    body: ByteStream,
    client: &ApiType,
    ctx: &StreamContext<'_>,
    include_think_tags: bool,
) -> ByteStream {
    let body = relay_stream(relay, body, client, ctx);
    let body = if include_think_tags {
        body
    } else {
        super::think_tags::strip_think_tags_stream(body)
    };
    let body = crate::with_termination_guard(body, client);
    crate::sse::with_heartbeat(
        body,
        crate::sse::HEARTBEAT_INTERVAL,
        crate::sse::COMMENT_PING,
    )
}

/// The complete (non-streaming) body the client receives for `body`.
///
/// Passthrough returns the very same `Bytes`. Translation fails with a
/// `VkdgError` (rendered as a 502 in the client's dialect) when the upstream body
/// is not a valid message of its dialect: a foreign-dialect body must never reach
/// the client.
pub(super) fn relay_complete(
    relay: &Relay,
    body: Bytes,
    client: &ApiType,
    ctx: &StreamContext<'_>,
) -> Result<Bytes, VkdgError> {
    let Relay::Translate { from } = relay else {
        return Ok(body);
    };
    let events = vkdg_provider_sdk::json_decoder_for(*from)(&body)?;
    let mut encoder = vkdg_operations::json_encoder_for(client, ctx);
    for event in &events {
        encoder.push(event);
    }
    encoder.finish().map(Bytes::from)
}

pub(super) type ByteStream = Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>>;

/// Decodes an upstream byte stream to events and re-encodes them in the dialect
/// the client spoke.
///
/// The encoder lives in `vkdg-operations` so a dialect is written in exactly one
/// place. Each upstream chunk yields at most one output chunk (all its events
/// encoded together, so the client sees fewer, larger writes). When upstream
/// ends, `decoder.finish()` and the encoder's own close both run: a provider that
/// sends no stop event still produces a well-formed terminal sequence. A decoder
/// `Failed` ends the stream right after the dialect's error event and drops the
/// upstream, which cancels the request.
///
/// Backpressure: upstream is polled only when the client asks for the next
/// chunk, so at most one upstream chunk's events are in flight.
pub(super) fn decode_stream_to_sse(
    body: ByteStream,
    decoder: Box<dyn ConversationStreamDecoder>,
    api_type: &vkdg_core::ApiType,
    ctx: &vkdg_operations::StreamContext<'_>,
) -> ByteStream {
    struct State {
        upstream: ByteStream,
        decoder: Box<dyn ConversationStreamDecoder>,
        encoder: Box<dyn StreamEncoder>,
    }

    /// Encodes `events` into `out`; true when one of them ended the response.
    fn encode_all(
        encoder: &mut dyn StreamEncoder,
        events: &[ConversationEvent],
        out: &mut Vec<u8>,
    ) -> bool {
        let mut failed = false;
        for event in events {
            out.extend(encoder.encode(event));
            if matches!(event, ConversationEvent::Failed { .. }) {
                failed = true;
                break;
            }
        }
        failed
    }

    let state = State {
        upstream: body,
        decoder,
        encoder: vkdg_operations::stream_encoder_for(api_type, ctx),
    };

    Box::pin(futures::stream::unfold(Some(state), |state| async move {
        let mut state = state?;
        loop {
            let mut out = Vec::new();
            match state.upstream.next().await {
                Some(Ok(chunk)) => {
                    let events = state.decoder.feed(chunk);
                    if !encode_all(state.encoder.as_mut(), &events, &mut out) {
                        if out.is_empty() {
                            continue;
                        }
                        return Some((Ok(Bytes::from(out)), Some(state)));
                    }
                    // Failed: the error event is the last thing the client gets.
                }
                Some(Err(e)) => return Some((Err(e), None)),
                None => {
                    // Upstream closed: flush the decoder, then close the dialect.
                    let events = state.decoder.finish();
                    encode_all(state.encoder.as_mut(), &events, &mut out);
                }
            }
            out.extend(state.encoder.finish());
            // Vendor-specific event with the Kiro context window usage percentage,
            // so the metering layer can store it in the request log.
            if let Some(pct) = state.decoder.context_usage_pct() {
                out.extend_from_slice(
                    format!("data: {{\"type\":\"vkdg_context_usage\",\"pct\":{pct}}}\n\n")
                        .as_bytes(),
                );
            }
            return (!out.is_empty()).then(|| (Ok(Bytes::from(out)), None));
        }
    }))
}

#[cfg(test)]
mod tests;
