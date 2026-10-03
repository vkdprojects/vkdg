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
- Decode carries `tool_choice`, `parallel_tool_calls`, `stop`, `top_p`, `max_completion_tokens` (over `max_tokens`), the text beside `tool_calls`, every `system`/`developer` message, and `image_url` parts as images; a value it cannot carry (unknown `tool_choice`, non-string `stop`, `top_p` outside 0..1, unknown role or content part) is a `400` naming the field, never a silent default
- A tool call whose `arguments` is not a JSON object replays as `{}`: clients echo model output verbatim, and rejecting it would wedge the session

## Focal test

```bash
cargo test -p vkdg-ingress-openai
```

## Used by

`bin/vkdg` (registers the handler in the router), `tests/conformance`.
