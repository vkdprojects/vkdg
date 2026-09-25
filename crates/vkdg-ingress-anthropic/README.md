# vkdg-ingress-anthropic

Input adapter for the Anthropic Messages protocol. Decodes wire requests → typed `Operation`; encodes `ConversationEvent` → SSE; maps `VkdgError` → Anthropic HTTP response. Has no knowledge of routing, credentials, or storage.

## Public API

- `decode_request(body: Bytes) -> Result<(String, Operation), VkdgError>`: validates and converts Anthropic bytes into `(model_name, Operation::Conversation(...))`
- `encode_event(event: &ConversationEvent) -> String`: serializes an event as an SSE line; `Completed` emits the event JSON followed by `data: [DONE]`
- `events_to_sse_stream(stream) -> Response`: wraps a `ConversationEvent` stream in an Axum SSE response with correct headers
- `vkdg_error_to_anthropic_response(err: VkdgError) -> Response`: maps VKDG errors to HTTP status and JSON body in Anthropic error format
- `handle_messages(State<AppState>, Request) -> Response`: Axum handler for `POST /v1/messages`; orchestrates decode, pipeline, and encode

## Invariants

- A decode error returns an HTTP response with a valid Anthropic body, never a generic 500 without a body
- `encode_event` for `Completed` always emits `[DONE]` as the final frame
- `handle_messages` returns 501 when `AppState` has no `PipelineState` configured

## Focal test

```bash
cargo test -p vkdg-ingress-anthropic
```

## Used by

`bin/vkdg` (registers the handler in the router), `tests/conformance` (ingress and smoke tests).
