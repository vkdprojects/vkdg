# vkdg-operations

Operation contracts by family: Conversation, Embedding, Image, Audio, Video. Does not implement routing logic, HTTP, or credentials.

## Public API

- `Operation`: sum enum of all supported operation types
- `ConversationRequest`: payload of a conversation request (messages, model, tools, parameters)
- `Message`, `MessageContent`, `ContentBlock`, `Role`: normalized message types
- `ConversationEvent`: stream events (`Started`, `OutputDelta`, `ToolCallDelta`, `Usage`, `Completed`, `Failed`)
- `ConversationResponse`: complete (non-streaming) response
- `Tool`, `OperationFamily`, `SupportLevel`: capability metadata per operation
- `Capability`, `CapabilitySet`: re-exported from `vkdg-core`; callers that import from here do not break after the migration

## Invariants

- `ConversationEvent::Completed` always terminates a stream; no events must follow it
- `CapabilitySet` imported from here is the same type as in `vkdg-core`: there are no two distinct types in the workspace

## Focal test

```bash
cargo test -p vkdg-operations
```

## Used by

`vkdg-ingress-anthropic` (decode/encode), `vkdg-http` (pipeline), `bin/vkdg`.
