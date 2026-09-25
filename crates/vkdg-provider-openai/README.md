# vkdg-provider-openai

Provider adapter for the OpenAI API. Implements `ProviderAdapter` to translate a typed `Operation` into a ready-to-send HTTP request targeting `api.openai.com`.

## Public API

- `OpenAIAdapter`: implements `ProviderAdapter`; sets `Authorization: Bearer` header; serializes `ConversationRequest` to OpenAI Chat Completions wire format including parallel tool calls

## Invariants

- `prepare()` returns `Err(VkdgError::CapabilityUnsupported)` for operations the adapter does not support
- Upstream SSE parsing handles parallel tool call deltas (`index`-keyed accumulation)
- Auth header uses the bearer token from `CredentialManager`: never reads env vars directly

## Focal test

```bash
cargo test -p vkdg-provider-openai
```

## Used by

`bin/vkdg` (wired into `PipelineState`), `tests/conformance`.
