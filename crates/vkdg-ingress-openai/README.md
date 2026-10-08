# vkdg-ingress-openai

Input adapter for the OpenAI Chat Completions protocol. Decodes wire requests → typed `Operation`; encodes `ConversationEvent` → OpenAI SSE wire format; maps `VkdgError` → OpenAI-compatible HTTP response. Has no knowledge of routing, credentials, or storage.

## Public API

- `decode_request(body: Bytes) -> Result<(String, Operation), VkdgError>`: validates and converts OpenAI Chat Completions bytes into `(model_name, Operation::Conversation(...))`
- `encode_event(event: &ConversationEvent) -> String`: serializes an event as an OpenAI SSE line; `Completed` emits `data: [DONE]`
- `events_to_sse_stream(stream) -> Response`: wraps a `ConversationEvent` stream in an Axum SSE response with correct OpenAI headers
- `vkdg_error_to_openai_response(err: VkdgError) -> Response`: maps VKDG errors to HTTP status and JSON body in OpenAI error format
- `handle_chat_completions(State<AppState>, Request) -> Response`: Axum handler for `POST /v1/chat/completions`

## Invariants

- A decode error returns an HTTP response with a valid OpenAI error body, never a generic 500
- `encode_event` for `Completed` always emits `data: [DONE]` as the final frame
- Parallel tool call deltas are encoded preserving the `index` field

## Focal test

```bash
cargo test -p vkdg-ingress-openai
```

## Used by

`bin/vkdg` (registers the handler in the router), `tests/conformance`.
