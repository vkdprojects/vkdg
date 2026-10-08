# vkdg-operations

Operation contracts by family: Conversation, Embedding, Image, Audio, Video. Does not implement routing logic, HTTP, or credentials.

## Public API

- `Operation`: sum enum of all supported operation types
- `ConversationRequest`: payload of a conversation request (messages, model, tools, parameters)
- `Message`, `MessageContent`, `ContentBlock`, `Role`: normalized message types
- `ConversationEvent`: stream events (`Started`, `OutputDelta`, `ReasoningDelta`, `ToolCallDelta`, `ToolCallEnd`, `Usage`, `Completed`, `Failed`)
- `StreamEncoder` / `stream_encoder_for(api_type, ctx)` and `JsonEncoder` / `json_encoder_for(api_type, ctx)`: events to the client's dialect, streaming and complete bodies
- `client_error` / `stream_error_frame`: the one renderer of `VkdgError` in each client dialect (status, `retry-after`, body, mid-stream frame)
- `WireFormat`: the dialect a provider speaks upstream (`ProviderAdapter::wire_format`); `ToolChoice`: tool policy in neutral terms
- `Tool`, `OperationFamily`, `SupportLevel`: capability metadata per operation
- `Capability`, `CapabilitySet`: re-exported from `vkdg-core`; callers that import from here do not break after the migration

## Invariants

- `ConversationEvent::Completed` always terminates a stream; no events must follow it
- `CapabilitySet` imported from here is the same type as in `vkdg-core`: there are no two distinct types in the workspace
- Usage in the event model counts the Anthropic way (`input` excludes cache tokens); the OpenAI `prompt_tokens` conversion lives only in the encoders (`usage.rs`)
- Every encoder output chunk ends on a `\n\n` event boundary; a stream always ends with its dialect's terminal frame (`message_stop`, `[DONE]`, or an error frame)

## Focal test

```bash
cargo test -p vkdg-operations
```

## Used by

`vkdg-ingress-anthropic` (decode/encode), `vkdg-http` (pipeline), `bin/vkdg`.
