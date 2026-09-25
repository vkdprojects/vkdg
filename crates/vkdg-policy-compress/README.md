# vkdg-policy-compress

Context compression policy: truncates conversation history to fit within a token budget while preserving system messages. Emits loss metrics for each truncation. Has no knowledge of HTTP, credentials, or routing.

## Public API

- `CompressPolicy` — applies context truncation to a `ConversationRequest`; returns the truncated request and a `CompressionMetrics` summary
- `CompressionMetrics` — records messages removed, estimated tokens saved, and whether system messages were preserved
- `CompressConfig` — configuration: `max_tokens`, `preserve_system`, `strategy` (`TailDrop` | `Summarize`)

## Invariants

- System messages (`role: system`) are never dropped when `preserve_system` is `true`
- `CompressPolicy` with `Summarize` strategy is a stub in Phase D — falls back to `TailDrop` until an internal metered call is available in Phase E
- Truncation never modifies the original request in place; returns a new value

## Focal test

```bash
cargo test -p vkdg-policy-compress
```

## Used by

`vkdg-http` (pipeline runner, optional policy stage), `bin/vkdg`.
