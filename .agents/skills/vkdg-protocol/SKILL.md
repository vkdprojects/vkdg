---
name: vkdg-protocol
description: "Work on VKDG native and compatible APIs, ingress/provider adapters, conversion, SSE, multimodality, and capability matrix. Use when adding a provider, endpoint, protocol field, stream, or media job; combine with vkdg-change and vkdg-operate as needed."
---

# Implementing a protocol contract

## First analysis

1. Identify the client/input protocol, the VKDG operation, the provider/connection/model pair, and the response shape: JSON, stream, or job. Read current fixtures and the official documentation for the protocol version.
2. Build a mapping table: source field/event, meaning, destination field/event, reversibility, losses, and error for required feature without support. Separate `required`, `optional`, `vendor-specific`, and `unknown`.
3. Define complete examples for success, error before commit, error after commit, cancellation, and partial usage. For the first client, exercise real Claude Code/Messages; then Codex/Responses. Do not call an endpoint compatible until it passes its corpus.

## Boundaries and semantics

- Ingress decodes the client protocol and encodes the response back. Provider adapter publishes an operation/capability descriptor, prepares upstream, decodes events, and classifies errors. Core owns HTTP, credentials, size limits, timeout, commit, and telemetry.
- Do not create a universal `Message` to hide Anthropic blocks, Responses items, and specific events. Use variants per operation; preserve extensions in a typed, size-limited namespace. An unknown field must not be silently dropped if it affects behavior.
- Capability is verifiable by operation + model + connection + protocol, including tools, reasoning, vision, schema, streaming, idempotency, and limits. `required` without correct translation means `unsupported_capability` before the attempt. Passthrough only when semantics and policy allow.
- Keep the input protocol outside the routing domain: the combo chooses only candidates capable of serving the operation. Model name does not prove support for media or tool format.
- For large video/audio/image, use a stream or handle with ACL/TTL; do not copy binary content to a JSON string at each layer. Async video is a durable job, with idempotent polling/callback/cancellation.

## Streaming and failures

- Treat HTTP bytes, SSE lines, events, and semantic deltas as different levels. Incremental parser supports splitting at any byte, partial UTF-8, large events up to configurable limit, unexpected EOF, and cancellation. Do not join the entire body for convenience.
- Mark `committed` when the client can no longer receive a transparent substitute response. Before retrying, assess whether upstream may have accepted a billable operation; after commit, do not fall back in a way that produces a second response.
- Translate usage as `reported | estimated | unknown`; never invent exact numbers. Forward `request_id` without leaking token. A compatible error response preserves the status and shape expected by the client whenever possible.
- Distinguish conversation, embedding, image, audio, and job codecs. Agent tools and reasoning deserve specific tests with real event sequences.

## Fixtures and proof

Start with a minimal failing fixture, with a fake upstream, variable fragmentation, headers/status, cancellation, and a slow client. Compare event by event, not just concatenated text. Add a matrix of supported pairs with guarantee level and tested version. Smoke with a real client measures tool calls, thinking, session, and compaction. If credentials are missing for a real test, complete the deterministic tests and honestly record the gap.

## Expected output

Deliver an updated fidelity table, fixture/contract, implementation at the correct boundary, and incompatible feature diagnostics. The goal is for another agent to add a provider without modifying the router or editing another provider's codec.
