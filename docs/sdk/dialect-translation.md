# Dialect translation: fidelity matrix

What the gateway translates between client and provider, how faithfully, and what it drops.
Design and decision: [ADR-004](../adr/ADR-004-dialect-translation.md).

## Supported pairs

Client dialect is the one the request arrived in (`/v1/messages`, `/v1/chat/completions`);
upstream dialect is `ProviderAdapter::wire_format`.

| Client | Upstream | Stream | Non-stream | Guarantee |
|---|---|---|---|---|
| Anthropic | Anthropic | passthrough | passthrough | bytes untouched, never parsed |
| OpenAI Chat | OpenAI Chat | passthrough | passthrough | bytes untouched, never parsed |
| OpenAI Chat | Anthropic | translated | translated | text, reasoning, tool calls, usage, stop reason, errors |
| Anthropic | OpenAI Chat | translated | translated | same set |
| any | Kiro (AWS event stream) | translated by Kiro's decoder | n/a (Kiro always streams) | unchanged |
| any | Codex (Responses API) | **pending** | **pending** | `wire_format` is `None`: raw Responses events reach the client |

Tested against: Anthropic Messages API `2023-06-01`, OpenAI Chat Completions (2025-2026 shape,
including `reasoning_content`), live Claude Code subscription (`claude-sonnet-4-5`,
`claude-haiku-4-5`, `claude-opus-4-5`).

## Response mapping

| Concept | Anthropic | OpenAI Chat | Notes |
|---|---|---|---|
| answer text | `text_delta` | `delta.content` | |
| reasoning | `thinking_delta` | `delta.reasoning_content` (also reads `reasoning`) | emitted only when the upstream sent it |
| tool call open | `content_block_start` `tool_use` (id, name) | `delta.tool_calls[i]` with `id`, `type`, `function.name` | id synthesized when empty, sanitized to `[A-Za-z0-9_-]` |
| tool arguments | `input_json_delta` | `function.arguments` fragments | non-stream Anthropic needs an object: empty means `{}`, a non-object is a 502 |
| tool call end | `content_block_stop` | next call opens or finish arrives | |
| stop reason | `end_turn` / `max_tokens` / `tool_use` / `stop_sequence` | `stop` / `length` / `tool_calls` / `stop` | unknown values end the turn; a stream cut after a tool call reports tool use |
| usage | `usage` in `message_start` + cumulative `message_delta` | trailing chunk with `choices: []` | see below |
| error before commit | HTTP status + `{"type":"error",...}` | HTTP status + `{"error":{...}}` | same status either way, one table |
| error after commit | `event: error` | error object + `[DONE]` | no retry, no further content |

### Usage

The event model counts the Anthropic way: `input` excludes cache tokens; `cache_read` and
`cache_creation` are separate. Each report is one of `Reported`, `Estimated`, `Unknown`; a later
event overwrites only the fields it reports.

| | to OpenAI client | from OpenAI upstream |
|---|---|---|
| input | `prompt_tokens = input + cache_read` | `input = prompt_tokens - cached_tokens` |
| cache read | `prompt_tokens_details.cached_tokens` | read from the same field |
| cache creation | `cache_creation_input_tokens` (never in `prompt_tokens`) | read from the same field |

Cache creation stays out of `prompt_tokens` because Anthropic pads short prompts up to a cache
minimum; counting it turned a 2-token prompt into ~2000 billed tokens.

## Deliberate losses

| What | Why | Where |
|---|---|---|
| thinking `signature` and `redacted_thinking`, between dialects | an OpenAI client has no field to hold them, and a signature is only valid for the provider session that made it | an Anthropic client over an Anthropic upstream keeps them (bytes untouched; checked live: a signed thinking block replayed with its tool result is accepted). Across dialects the Anthropic encoder never invents a signature, and a continuation turn without a signed thinking block is sent without `thinking` (checked live: an OpenAI client's tool loop with `reasoning_effort` works) |
| provider-run tool results (`server_tool_use`, `web_search_tool_result`) and citations, in the translated direction | an OpenAI client cannot declare server tools, so they cannot occur there | An Anthropic client over an Anthropic upstream gets them untouched (checked live: `web_search_20250305`, stream and not). Replayed as history they are dropped and the answer text stays; an OpenAI-dialect upstream is never sent a server tool |
| `n > 1` / choices other than the first | one answer per request | rejected at ingress; upstream extras ignored |
| `stop_sequence` value | not carried by the event model | `stop_sequence` is always `null` |
| unknown fields and events | forward compatibility | ignored, never fail a stream |
| images, Kiro and Codex | their request builders do not carry images at all (user or tool result) | pending: Kiro needs an account to verify its `images` field, Codex goes with the Responses work |
| images in a tool result that no call owns | the orphan result is rendered as text | text kept, images dropped |

## Failure handling (both directions)

- A stream or body that is empty, truncated (no stop reason), malformed JSON, over the size
  limit, or a body of the wrong dialect becomes a gateway error with a fixed message; the
  upstream payload is never echoed.
- Limits: SSE event 1 MiB, tool-call arguments 8 MiB, non-streaming response 32 MiB.
- A decoder failure after bytes were sent ends the client stream with its dialect's error frame
  and drops the upstream (which cancels the request). It is never retried on another provider.

## Request side

| Field | OpenAI client to Anthropic upstream | Anthropic client to OpenAI upstream | Notes |
|---|---|---|---|
| `tool_choice` | `auto` / `none` / `required` / named to `auto` / `none` / `any` / `tool` | reverse | dropped when there are no tools; a forced or unknown tool is a 400 at ingress |
| parallel tool calls | `parallel_tool_calls: false` to `disable_parallel_tool_use` | reverse | only with tools |
| `stop` / `stop_sequences` | to `stop_sequences` | to `stop` | max 16 entries of 256 bytes; empties dropped; over the limit is a 400 |
| `top_p` | sent; dropped when `temperature` is also set | sent | range 0 to 1 |
| `temperature` with thinking | kept only when equal to 1 | n/a | |
| `max_tokens` / `max_completion_tokens` | `max_completion_tokens` preferred; default 8192 | one field only (`max_completion_tokens` on api.openai.com, `max_tokens` elsewhere) | |
| thinking budget vs `max_tokens` | `max_tokens` raised by the budget when it is not larger | n/a | |
| thinking with a forced tool | `thinking` not sent | n/a | |
| system / developer messages | all joined with a blank line | joined into one leading system message | |
| assistant text beside tool calls | kept as a leading text block | kept as `content` beside `tool_calls` | |
| parallel tool results | one user turn of `tool_result` blocks in call order | one `tool` message each, behind its call | a call without a result gets an error result; an orphan result becomes text |
| images | `image_url` to Anthropic `source` (base64 or URL) | to `image_url` (data URL when inline) | in a tool result: kept inside the Anthropic `tool_result`; to an OpenAI upstream, the `tool` message carries the text and the images follow in a labelled `user` message (Chat Completions tool messages are text only) |
| Codex / Kiro | n/a | n/a | `tool_choice`, `stop`, `top_p` and the parallel limit are not forwarded |

### Kiro history limit

Kiro receives at most 100 turns, counting `currentMessage`. When history exceeds
that limit, the adapter keeps the system-bearing first user turn and the newest
suffix beginning with an assistant turn. It removes complete older exchanges,
not a tool call without its result or a result without its call. An assistant-ended
conversation reserves a slot for the current `Continue.` user message.

The cap does not fabricate tool results or turn results orphaned by the gateway's
own truncation into text. Existing history-content aging still applies; the current
tool result remains unabridged.

The loopback smoke uses the real OpenAI ingress, pipeline and Kiro request builder.
Its fake upstream independently checks immediate tool adjacency, system preservation
and the 100-turn limit:

```sh
cargo test -p vkdg-provider-kiro --test prepare_tools history_cap
cargo test -p conformance smoke_kiro_history_cap_real_http_ingress -- --nocapture
```

These checks need no upstream credentials. They do not establish which adapter
or binary version a remote deployment currently runs.

## Cost

Measured with a 2005-event Anthropic stream in 1 KiB reads, release build, one core
(`tests/conformance`, throwaway benchmark, not committed):

| Path | Cost per event |
|---|---|
| passthrough | 0 (bytes forwarded, never parsed) |
| framing only (`SseFramer`) | ~190 ns |
| Anthropic to OpenAI translation (frame + decode + encode) | ~760 ns (~1.3 M events/s) |

Translation is about 4x a bare framing pass: the extra time is the JSON parse of each event and
building the encoded chunk. A real stream emits tens to hundreds of events per second, so the
cost is well below the network and the model. Peak buffered input is one SSE event plus one
upstream chunk's events (9 events for a 1 KiB read).

## Pending

- **Codex / OpenAI Responses API.** A third decoder/encoder pair on the same canonical events:
  `WireFormat::OpenAiResponses`, a Responses client dialect for `/v1/responses` (today its stream
  encoder falls back to Anthropic's), and the Responses error shape. Until then Codex responses reach
  the client as raw Responses events, and its request body ignores `tool_choice`, `stop` and `top_p`.
