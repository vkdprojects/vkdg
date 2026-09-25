# vkdg-provider-anthropic

Provider adapter for the Anthropic Messages API. Implements `ProviderAdapter` to translate a typed `Operation` into a ready-to-send HTTP request targeting `api.anthropic.com`.

## Public API

- `AnthropicAdapter`: implements `ProviderAdapter`; sets `x-api-key`, `anthropic-version`, and `content-type` headers; serializes `ConversationRequest` to Anthropic Messages wire format

## Invariants

- `prepare()` returns `Err(VkdgError::CapabilityUnsupported)` for operations other than `Conversation`
- Auth header uses the raw bearer token passed by `CredentialManager`: never reads env vars directly
- `anthropic-version` header is pinned to a constant; changes require an explicit version bump

## Focal test

```bash
cargo test -p vkdg-provider-anthropic
```

## Used by

`bin/vkdg` (wired into `PipelineState`), `tests/conformance`.
