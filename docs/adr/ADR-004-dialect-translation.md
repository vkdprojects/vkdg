# ADR-004: Dialect translation through canonical events

**Status:** Accepted
**Date:** 2026-10-03

## Context

A client speaks one conversation dialect (Anthropic Messages, OpenAI Chat Completions) and
the provider behind the chosen connection speaks one too, not always the same. Before this
change only Kiro (AWS event stream) had a decoder; every other provider's response reached the
client untouched. An OpenAI client routed to Claude Code received an Anthropic message
(`{"type":"message","content":[...]}`), and an Anthropic client routed to an OpenAI-compatible
provider received `chat.completion` chunks. Both break the client's SDK.

The reference implementation we studied (OmniRoute, `open-sse/translator`) translates through
a hub: every dialect is converted to OpenAI chunks and back, per chunk, with a mutable state
object, separate code for streaming and non-streaming, and `JSON.parse` + `JSON.stringify` on
every chunk, including when both sides already speak the same dialect.

## Options

1. **Hub-and-spoke through OpenAI JSON** (the reference). Simple to add a pair, but every
   response is parsed and re-serialized, Anthropic-only information (cache creation, block
   structure) is squeezed through OpenAI's shape and lost, and stream and non-stream mappings
   drift apart.
2. **Pairwise translators** (Anthropic to OpenAI, OpenAI to Anthropic, ...). Fast, but N(N-1)
   mappings and each new dialect touches all of them.
3. **Canonical events** (chosen). The gateway already had `ConversationEvent`
   (`Started`, `OutputDelta`, `ReasoningDelta`, `ToolCallDelta`, `ToolCallEnd`, `Usage`,
   `Completed`, `Failed`) and per-client-dialect encoders for Kiro. A decoder per upstream
   dialect and transport produces events; one encoder per client dialect consumes them.

## Decision

- A provider declares the dialect it speaks upstream: `ProviderAdapter::wire_format(config)`
  returns `Option<WireFormat>` (`AnthropicMessages` or `OpenAiChat`). `None` (default) means
  the gateway never translates for it. An adapter's own `stream_decoder` (Kiro) still wins.
- Per response, the relay (`vkdg-http` `pipeline/relay.rs`) compares the upstream dialect with
  the client's (`ApiType`):
  - **same or unknown on either side: passthrough.** Bytes are forwarded untouched and never
    parsed (no framing, no JSON), which is the zero-cost path;
  - **different: translate.** Streams go through `SseFramer` + `AnthropicSseDecoder` /
    `OpenAiSseDecoder` + the client's `StreamEncoder`; complete bodies go through
    `decode_*_json` + the client's `JsonEncoder`. Streaming and non-streaming share the event
    vocabulary and the encoders' mapping tables (stop reasons, usage, tool ids).
- Decoders live in `vkdg-provider-sdk` (`dialect`), encoders and the single client-dialect
  error renderer (`client_error`, `stream_error_frame`) live in `vkdg-operations`. A client
  dialect is therefore written in exactly one place, and the pipeline's error responses use
  the same renderer as the stream encoders.
- Usage keeps the event model's Anthropic semantics (`input` excludes cache tokens, cache
  read and cache creation are separate). Converting to OpenAI's `prompt_tokens` happens only
  in the encoder (`prompt_tokens = input + cache_read`, cache creation never included); decoding
  OpenAI subtracts `cached_tokens`.
- Failures are values, not text in a body: a malformed, truncated, empty or oversized upstream
  stream becomes `Failed` with a fixed message (the payload is never echoed). A 2xx body that is
  not a valid message of its dialect is a 502 in the client's dialect, never a foreign-dialect body.
- Memory is bounded everywhere a hostile or runaway upstream could grow it: one SSE event
  (default 1 MiB), one tool call's streamed arguments (default 8 MiB), one non-streaming response
  (32 MiB).

## Consequences

- Adding a client dialect means one encoder; adding an upstream dialect means one decoder pair.
  Neither touches the router or another provider.
- Translation costs one framing pass, a JSON parse per event and an encode; same-dialect
  traffic costs nothing. Measured numbers are in `docs/sdk/dialect-translation.md`.
- Deliberate losses (signatures, redacted thinking, server-side tool blocks, choices beyond the
  first) are listed in the fidelity matrix. They are dropped, never partially forwarded.
- The Responses API (Codex) is not translated yet: `wire_format` is `None` for it.

## Reversible experiment

Set an adapter's `wire_format` back to `None` to restore passthrough for that provider, or keep
both dialects equal on a connection. The relay's mutation check in the conformance suite
(`dialect_translation`) fails when translation is disabled.
