# Changelog

All notable changes to VKDG are documented here.
Format: [keepachangelog.com](https://keepachangelog.com/en/1.1.0/).
Versioning: [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added
- **Anthropic prompt caching:** `cache_control` from Anthropic clients (system blocks, text,
  image, `tool_use`, `tool_result` blocks and custom tools) now reaches Anthropic-format
  upstreams with its `ttl` intact. When a request carries no marker at all, the gateway marks
  the last custom tool, the last system block and the last block of the last message; this applies
  only to Anthropic-format providers and is never done on top of client markers. At most 4
  breakpoints are sent (the last 4 are kept). Markers do not change the response-cache key or
  the session-affinity digest. `ConversationRequest::cache_key` carries that digest for providers
  with their own cache key. `cache_read_tokens` / `cache_write_tokens` appear in request logs and
  the console.
- **Codex and Kiro cache affinity:** requests with a `cache_key` now send it to Codex as the
  body field `prompt_cache_key` and the headers `session_id` / `session-id` (OAuth and API-key
  accounts; omitted when there is no key), and Kiro derives a deterministic UUID `conversationId`
  from it (an explicit client `session_id` still wins; otherwise a random UUID as before). Field
  and header names follow the openai/codex source; upstream cache hits are verified only by
  `cache_read` tokens in the request log, and Kiro reuse by `conversationId` is unverified.
- **OpenAI Codex provider:** full OAuth PKCE login flow via `auth.openai.com`, token refresh,
  and connection test. The provider appears in the console under "OpenAI Codex" alongside Claude
  Code and Kiro.
- **Dynamic model catalog:** `provider_catalog` table in `gateway.db` stores models per provider
  without requiring a rebuild. Admin API: `GET/PUT /admin/v1/catalog/{provider}` and
  `POST /admin/v1/catalog/{provider}/import-url` (imports from any JSON URL). Codex uses this
  catalog; any future provider can too.
- **Session persistence across restarts:** admin console sessions are now stored in `gateway.db`
  (`admin_sessions` table) and reloaded on startup. A deploy or restart no longer forces a new
  login.
- **Rate-limit windows for Claude Code and Codex accounts:** `GET /admin/v1/accounts` now returns
  `usage_windows` (`kind` = `five_hour|weekly|weekly_sonnet|weekly_opus`, `used_percent`,
  `resets_at` as Unix seconds). Only windows the upstream reports are listed, so plans without a
  general weekly limit show none. `credits_source`, `credits_checked_at` and `credits_plan` are the
  generic usage-source metadata for both credit and window providers; `credits_used` stays absent
  for percent-only providers. A failed read is `credits_source: "unavailable"`, never 0%. Usage
  reads use the refreshed OAuth token, and a revoked account makes no upstream call. Sources are
  undocumented endpoints (`api.anthropic.com/api/oauth/usage`, `chatgpt.com/backend-api/wham/usage`).
- **Usage overview in the console:** the Accounts page shows every metered account side by side
  (one bar per window, most-consumed first) above the per-account cards, which now render a bar per
  window with remaining percent and reset time.

### Fixed
- **Account "Test" failed for Claude Code and Codex with "no eligible connection":** account
  connections list only patterns (`claude-*`, `gpt-*`), so the test request named a model no route
  served and died in routing before reaching the account. The test now pins the request to the
  connection under test (`PipelineCtx::pinned_connection`), skipping routing and sibling failover.
- **`power_of_two_choices` routed by latency, starving the slower target:** it compared the p50
  latency of its two samples, so with two targets it behaved like `lowest_latency`. Latency is only
  measured on targets that get traffic, so the faster Claude Code account took every request while
  the other sat idle for hours (observed in prod: 0 requests for 2 h, the other account at 68% of
  its 5 h window). It now picks the target with fewer requests in flight (`RoutingHints.in_flight`,
  from `ConnectionCatalog::in_flight`); ties go to a random sample. Use `lowest_latency` to
  prefer the fastest target.
- Codex multi-turn history uses `output_text` for assistant content and `input_text`
  for user/developer content. Mixed text, function calls and results retain their
  order and call IDs instead of discarding text beside tool blocks.
- Codex Responses streams decode into client-native Chat Completions or Messages
  events, including reasoning, parallel tools, usage and terminal errors.
  `response.output_text.done` does not complete the response. Regression coverage
  includes an 87/89-message HTTP tool round trip and arbitrarily fragmented SSE.
- **Kiro tool history cap:** long conversations no longer split an assistant's tool calls
  from their results at the 100-turn boundary. The cap removes complete historical
  exchanges, preserves the system-bearing first user turn and current message, and
  reserves a turn for assistant-ended continuations. Regression tests cover 50–52 tool
  round trips and anonymized 111-, 113-, 103- and 161-message incident shapes through
  real HTTP ingress and a strict loopback upstream.
- **Kiro context overflow loop:** conversations were growing unboundedly (observed: 2 428 messages
  in one session). The provider now caps history at 100 turns before the aging pipeline runs.
  Beyond that, Kiro was returning `out=0` responses, causing the omp agent to retry silently and
  burn through the account's rate limit.
- **401/403 treated as retryable:** upstream auth failures (`bearer token invalid`, expired OAuth
  token) were marked `retryable: true`, causing the client to retry indefinitely. They are now
  non-retryable and put the connection into cooldown immediately.
- **Admin router path syntax:** catalog routes used `:provider` (Axum v4 syntax) instead of
  `{provider}` (Axum v0.8+), causing a startup panic and HTTP 502 on the console.
- **`import-url` without timeout:** `reqwest::Client` in the catalog import handler had no
  timeout; a slow server could block a Tokio worker indefinitely. Fixed to 30 s.

### Changed
- **PKCE helpers moved to `vkdg-provider-sdk`:** `random_b64url`, `validated_loopback_redirect`,
  `parse_pkce_callback`, and `build_pkce_authorization` are now shared utilities in the SDK.
  Duplicated copies in `claude-code` and `codex` have been removed.
- **Admin response helpers:** `require_store`, `ok`, and `err` extracted to
  `vkdg-admin/handlers/response.rs`; repeated boilerplate removed from all handlers.
- **`create_connection` / `update_connection`:** shared logic extracted to `persist_connection`;
  the two handlers now call a single upsert path.
- **Routing by conversation, not by request:** the first request of a conversation takes the
  route's strategy (P2C in production); later turns stay on that connection for up to one hour so
  the provider's per-account prompt cache keeps hitting. The conversation key is the client
  session id, or a SHA-256 of tenant, client, system prompt and first user message (never stored
  as content). The pin is also set after streaming responses and is ignored when the connection is
  in cooldown, at capacity, unhealthy, does not serve the model, or already returned a 429. Expired
  pins are purged every 5 minutes.


### Security
- Console login: after a password is set, the bootstrap token is refused (not just after the first
  sign-in). Failed sign-ins are throttled per source IP (resolved through `VKDG_TRUSTED_PROXIES`)
  and globally, with 429 and `retry-after`.

### Added
- **Account routing in the console.** The account card (Accounts and the provider page) now shows how
  that account is routed and lets you edit it: models, weight, max concurrent, plus Test. An account with
  no connection (connected before they were automatic) gets an "Enable routing" button. The connection
  stays a separate object in the gateway (an account can have several, e.g. both Kiro endpoints) but the
  console no longer asks you to think about it. API: `connections[]` list items carry `models` and
  `weight`; `PATCH /admin/v1/connections/{id}` changes only `models`/`max_concurrent`/`weight` (auth,
  endpoint, base URL and tags are untouched, 422 on empty models or zero values);
  `POST /admin/v1/accounts/{id}/connection` creates the connection (201) or returns the existing one (200).
- **Request history:** streaming rows now record `input_tokens`, `output_tokens` and `cost_microdollars`
  (from the response body, once the stream ends). Status changes from `pending` to `completed`
  (body ran to the end) or `cancelled` (client disconnected mid-stream). Before, every streaming row
  finished as `completed` with no tokens, even when the client disconnected.
- **Per-key `no_log`:** a key with `no_log: true` still has its usage counted but generates no request
  history row. Toggle in the console, set via `POST /admin/v1/keys` or `vkdg keys create --no-log`.
- **USD cost per request:** every row in the request history carries `cost_microdollars` from the
  provider plugin's declared list prices. Kiro and Claude Code (subscription) show unknown rather than
  $0. Cache reads bill at 0.1× and cache writes at 1.25× the input price. `Custom` and
  `anthropic-compat` endpoints are never charged at another provider's prices.
- **Combos that actually route:** combos were stored but never applied in production (`combo_resolver`
  was always `None`). Now they route with their own strategy, carry an explicit upstream model, and can
  be created, edited and deleted from the console and `GET/POST/PUT/DELETE /admin/v1/combos`, taking
  effect on the next request. A combo named by its id takes priority over pattern-based combos.
- **Live plugin reload:** installing or removing a plugin through the console or the admin API reloads
  provider and auth registries on the next request, without a restart. Works for both `serve --config`
  and env-only gateways. Removing an auth plugin that a route still names is refused (409).
- **`openai-compat` / `anthropic-compat`:** config-only connections to any
  OpenAI Chat Completions or Anthropic Messages endpoint. Before, `provider: openai-compat` was
  silently mis-parsed and the `base_url` was dropped. Config check, serve and reload now refuse
  unknown provider ids (listing the known ones), dropped/typo'd fields, and `base_url` on providers
  that don't use it.
- **Kiro improvements:** prompt-cache tokens (`cache_read`, `cache_write`) reported in usage and priced
  correctly; `usageEvent` decoded; prompt-caching headers sent; `<thinking>` / reasoning support
  (adaptive models `claude-opus-5` / `claude-sonnet-5` on Kiro, `gpt-5.6-*` native reasoning);
  inline `<thinking>…</thinking>` stream blocks split into `ReasoningDelta` events; `api_key` auth
  uses the API-key endpoint, not Builder ID.
- **Codex:** request body now uses the Responses API shape (`input`, `instructions`, `store: false`,
  no `max_tokens`). The old Chat Completions body was rejected by Codex `/v1/responses`.
- All provider adapters (anthropic, claude-code, codex, github-copilot, kimi-coding, openai-compat)
  now send the model the client requested (`req.model`), not `config.models.first()`. That sent the
  route glob (`claude-*`, `gpt-*`) upstream or pinned every call to one entry of a multi-model
  connection.
- Client reasoning requests (`thinking` block on Anthropic, `reasoning_effort` on OpenAI) are now
  carried through the operation type to provider plugins.

### Fixed
- Kiro history tool results no longer panic when the 2,000-byte cutoff lands inside a UTF-8
  character. A Kiro `CONTENT_LENGTH_EXCEEDS_THRESHOLD` 400 now reaches OpenAI Chat clients
  with `error.code: context_length_exceeded`, including pre-commit stream failures, so clients
  can compact their own context. The gateway does not silently shorten the current request.
- **A rate-limited connection rejoined routing after at most five minutes, whatever the upstream said.**
  A 429/5xx carrying `retry-after` now keeps that connection out of routing for at least that long
  (capped at 6 hours); the exponential backoff (1 s to 300 s) stays the floor. A Claude subscription
  limit that resets in hours is no longer re-probed every five minutes, and the sibling connection
  serving the same model takes over meanwhile. The cooldown is also recorded when the connection's lock
  is briefly contended, instead of being skipped.
- **Model ids differing only by `.`/`-` between version digits no longer split connections.** Kiro's
  `claude-sonnet-4.6` and Anthropic's `claude-sonnet-4-6` are the same model for connection eligibility,
  so a client using either spelling reaches both connections of a route.
- **A Kiro account that was throttled or out of credits kept receiving traffic.** Kiro answers HTTP 200 and
  reports `ThrottlingException`, `ServiceQuotaExceededException`, `RATE_LIMIT_EXCEEDED` (429) or
  `MONTHLY_REQUEST_COUNT` (out of credits, 402) as the first frame of the stream, after the response was
  committed: no failover and no cooldown, every request failed. The gateway now reads the stream up to its
  first substantive event before committing. A throttling or quota first frame cools that connection (a
  402 holds it for 6 hours: credits do not return in minutes) and the request is retried once on a sibling
  connection, before any byte reaches the client; if no sibling can serve it the client gets the upstream's
  error with a `retry-after`. A normal first event streams exactly as before; a failure after the first
  event, or one that is not an account failure (400), is still passed to the client as the dialect's error
  event. A 402, 429 or 529 anywhere in the HTTP response path now fails over the same way.
- **An OpenAI client over an Anthropic provider (and the reverse) got the other dialect's body.** A client
  calling `/v1/chat/completions` that was routed to Claude Code received `{"type":"message","content":[...]}`
  instead of a `chat.completion` (and an Anthropic client over an OpenAI-compatible provider received
  `chat.completion` chunks), in streaming and not. Providers now declare the dialect they speak
  (`ProviderAdapter::wire_format`: Anthropic, OpenAI Chat) and the gateway translates the response to the
  client's dialect through canonical events: text, reasoning (`reasoning_content`), tool calls, usage,
  stop reasons and errors. Same-dialect traffic passes through untouched and unparsed. Cached tokens are
  converted both ways (`prompt_tokens` includes them, Anthropic's `input_tokens` does not) and cache
  creation never inflates `prompt_tokens`. An empty, truncated, malformed, oversize (event 1 MiB, tool
  arguments 8 MiB, body 32 MiB) or foreign-dialect upstream answer is a `502` with a fixed message, never the
  upstream payload and never a clean empty completion. See `docs/sdk/dialect-translation.md` and ADR-004.
- **Errors now match the client's dialect.** Pipeline errors were always Anthropic-shaped, even for OpenAI
  clients (SDKs showed an opaque parse failure). They use one renderer shared with the stream encoders
  (`{"error":{"message","type","param","code"}}` for OpenAI Chat, `{"type":"error",...}` for Anthropic,
  same statuses and `retry-after`), and a stream cut mid-way ends with the dialect's own error frame.
- **Stream and cache fixes found on the way:** a stream whose decoder flush produced events skipped the
  encoder's closing frame and the Kiro context-usage event; the OpenAI stream encoder reported `stop`
  after tool calls when no stop event arrived; the response cache stored the raw upstream body and
  ignored the client dialect, so a hit could serve a body in the wrong dialect (the key now includes it).
  The usage meter counted OpenAI cached tokens twice.
- **Request fidelity between OpenAI and Anthropic clients and upstreams.** Requests were decoded
  lossily and re-encoded the same way, so tool-using clients broke across dialects:
  - OpenAI ingress now reads `tool_choice`, `parallel_tool_calls`, `stop`, `top_p` and
    `max_completion_tokens` (preferred over `max_tokens`), keeps the assistant text that accompanies
    `tool_calls`, joins every `system`/`developer` message instead of keeping the first, and decodes
    `image_url` parts as images instead of flattening the URL into the prompt text. Invalid values answer
    `400` naming the field (`tool_choice`, `stop`, `top_p`, `messages[i].role`, `messages[i].content`);
    more than 16 stop strings, one over 256 bytes, or a forced tool the request never declared are refused.
  - Anthropic ingress now reads `tool_choice` (with `disable_parallel_tool_use`), `stop_sequences` and
    `top_p`. A `tool_result` whose content is an array reaches the model as its text, not as escaped JSON.
  - Anthropic upstreams (`anthropic`, `claude-code`) get Anthropic's own block shapes (images were sent in
    gateway-internal form), one user turn of `tool_result` blocks in call order for parallel results,
    an error result for a tool call that has none, and no orphaned `tool_result`. `max_tokens`
    defaults to 8192 when the client sent none (Anthropic requires it). Extended-thinking limits are
    applied instead of forwarded into a 400: `temperature` other than 1 and `top_p` below 0.95 are dropped,
    `thinking` is not sent with a forced tool or when continuing a tool turn whose assistant message has no
    signed thinking block, and `max_tokens` grows by the budget when it would not exceed it.
  - OpenAI upstreams (`openai`, `github-copilot`, `kimi-coding`, every OpenAI-compatible provider) share one
    body builder: `tool_choice`, `parallel_tool_calls: false`, `stop`, `top_p`, assistant text beside tool
    calls, one `tool` message per result placed behind its call, and images. api.openai.com gets
    `max_completion_tokens`; compatible endpoints keep `max_tokens`; never both.
  - Not forwarded on purpose: Codex (Responses API) and Kiro ignore `tool_choice`, `stop`, `top_p` and the
    parallel-call limit.
- **Images and provider-run tools were dropped.** A tool result's images (a screenshot returned by a
  `computer`-style tool) never reached the model: the operation type carried tool results as text only.
  `ToolResult` now carries its images. Anthropic upstreams get them inside the `tool_result`; OpenAI upstreams
  get the text in the `tool` message and the images in a labelled `user` message right behind it (Chat
  Completions tool messages are text only); OpenAI ingress reads `image_url` parts of a `tool` message. Anthropic
  clients can now declare provider-run tools such as `web_search_20250305` (the gateway answered 400 "missing
  field `input_schema`"): the declaration is forwarded untouched to Anthropic upstreams and never to OpenAI ones.
  Checked live on Claude Code: a screenshot-style tool result read back correctly from an OpenAI client and from an
  Anthropic client, user images, web search (stream and not), signed-thinking replay and an OpenAI client's
  tool loop with `reasoning_effort`. Kiro and Codex still do not forward any image (pending).
- **Claude Code requests always failed upstream.** Anthropic answers a subscription OAuth token with
  `429 rate_limit_error` unless the first system block is the Claude Code identity; the gateway then put
  the connection in cooldown and the client only saw "no eligible connection". The plugin now sends that
  block first and the client's own system prompt after it. Checked live: `claude-sonnet-4-5`,
  `claude-haiku-4-5`, `claude-opus-4-5`, streaming and non-streaming through `/v1/messages` all return 200.
- **Claude Code ignored `thinking`.** The plugin never sent the client's extended-thinking request, so
  replies had no thinking block. It now sends `thinking: { type: enabled, budget_tokens }` (1024, the
  minimum, when the client gave only an effort). Checked live, streaming and not.
- **A rate-limited sole connection was reported as "no eligible connection" (502).** After a 429 with no
  sibling to fall back to, the retry failed and replaced the real error. The client now gets the
  upstream 429 with its `retry-after`, and while every connection serving the model is cooling down,
  requests get `429` + `retry-after` (seconds until the first one frees up) instead of 502, without
  touching the upstream.
- **Connecting an account never created the connection that serves it.** The account was saved and the
  card showed "active", but nothing routed to it until a connection was written by hand (YAML snippet).
  Now every console login (device code, PKCE, token or API-key import) creates one: id = account id,
  `auth: { type: account }`, models from the new `ProviderAdapter::default_models()` (set for Claude
  Code, Codex, Kiro, Kimi, Copilot; Antigravity declares none because its `prepare` is not implemented).
  An account that already has a connection (reconnect, YAML, hand-made) keeps it as is. Reconnect an
  account that predates this to give it one. Deleting an account removes its connections and their route
  targets. The login response carries `connection_id` or `connection_error`. `vkdg login` still only prints
  the snippet.
- **Console connections did not reach the running gateway, and a config file fought them.** With no
  config file and no `ANTHROPIC_API_KEY` there was no pipeline at all, so connections made in the
  console only took effect after a restart; with `ANTHROPIC_API_KEY` the pipeline never received console
  edits. Saving the YAML replaced the live connections with the file's, dropping the console's. And a
  store that was written to but never seeded counted as empty, so a later `--config` boot overwrote it.
  Now the pipeline always applies config changes live (an unconfigured gateway answers 502, not 501),
  `ANTHROPIC_API_KEY` is seeded into the store on the first boot like a config file, a store with rows is
  never re-seeded, and saving the config file merges with the store: the file's connections and routes
  are added or replaced by id and persisted, ones it used to define and dropped are removed, and what
  the console made is kept. A reload that does not validate changes nothing.
- **Claude Code sign-in failed on claude.ai with "Invalid request format" after Authorize.** The OAuth
  `state` was 16 bytes; claude.ai requires the 32 bytes the official CLI sends. The authorize URL also
  no longer carries `prompt=login`, which bounced an already signed-in browser to the login page.
  When the console runs on `localhost`/`127.0.0.1` it now asks for `http://<origin>/callback` as the
  redirect, so the code is captured automatically; elsewhere the user pastes the code shown by
  claude.ai. `start` accepts a `redirect_uri` param, restricted to loopback `/callback`, and the token
  exchange repeats it verbatim.
- **Connect-account dialog reset itself when the OAuth popup opened,** cancelling the login (the popup
  handle was reactive state read by the open/reset effect). The popup-closed watcher is gone as well:
  claude.ai's Cross-Origin-Opener-Policy makes `popup.closed` read true while the popup is open. The
  code now arrives over `BroadcastChannel` only, with an "Enter the code manually" button.
- **`endpoint:` per connection, so one account can use both Kiro planes.** Kiro answers on
  `runtime.{region}.kiro.dev` and on `codewhisperer.us-east-1.amazonaws.com`, and the two keep
  SEPARATE rate-limit buckets: driving `runtime` to 75% HTTP 429 (271 of 360 at 120 concurrent) left
  `codewhisperer` answering 80/80 in the same window. Under a concurrency ramp `runtime` starts
  refusing at 10 concurrent and denies ~50-66% at 100, while `codewhisperer` served 2055 requests
  with zero errors up to 200. A connection may now name its plane, so the same account can run one
  connection per plane and the router adds both buckets instead of sharing one. `runtime` is refused
  for an API key (that host answers "profileArn is required"), and `profileArn` now follows the
  credential rather than the plane, so an OAuth account keeps sending it on either host.
- **Routing skips targets that cannot take the request.** Besides the model catalogue, a candidate
  already at `max_concurrent` or in cooldown is now excluded before the strategy runs. It used to be
  picked anyway and then fail to reserve, killing the request with "no eligible connection" without
  ever trying a free sibling — 15 of 36 requests at 12 concurrent, in a three-connection route.
- **Kiro API keys reach the whole catalogue.** A `ksk_` connection was sent to the legacy Amazon Q
  protocol (service root + `x-amz-target` + `origin: CLI`), which only serves `claude-sonnet-4`,
  `claude-sonnet-4.5` and `claude-haiku-4.5` and answers `INVALID_MODEL_ID` for everything else —
  including `auto`. The same key on the editor plane (`codewhisperer.us-east-1.amazonaws.com`,
  operation in the path, `origin: AI_EDITOR`) serves all 20 models, confirmed against
  `GET /ListAvailableModels?origin=AI_EDITOR`. `EndpointKind` now has one variant per credential
  type; the `Cli` / `SendMessage` buckets are gone, since nothing selected them once `api_key`
  stopped doing so.
- **Routing honours each connection's `models`.** A route names its targets by id, so its
  `match_models` said nothing about what each target actually serves: a round-robin route over two
  connections with different catalogues sent every other request to a connection that could not
  serve the model, and the client got an opaque upstream 400. Candidates outside their catalogue are
  now excluded before the strategy runs, with `model_not_served` in the decision record. The model
  compared is the one that goes upstream, so a combo (whose id no connection lists) still routes.
- `vkdg setup` and the console connection snippet generated YAML with broken indentation (`auth:`,
  `models:` at column 0). The gateway refused to load them.
- Reconnecting an OAuth account now replaces the existing account in place instead of creating a new
  one with a different id (the config kept pointing at the old, revoked account).

### Docs
- `docs/sdk/adding-a-provider.md` rewritten from the code: Tier 1 uses `openai-compat`/`anthropic-compat`
  with `base_url`; the Tier 1 example is tested against the parser. Tier 2 lists the four exports the
  WASM host actually calls and what it does not call yet (decode, login, prices).
- `docs/sdk/writing-a-plugin.md` rewritten around the two WASM roles that run (provider and route auth).
  `prepare()` in a WASM plugin is no longer a stub.
- Tokens are stored in `accounts.db` (SQLite, 0600). The vault reference is removed.

---

### Security
- Console sign-in: after the first sign-in with the bootstrap token, the console asks for a password (at least 12 characters), stored as an argon2id hash in `admin.password` next to `accounts.db` (`0600`; override with `VKDG_ADMIN_PASSWORD_FILE`). From then on the token is refused, even after a restart, and the password signs in any number of times. Before, the token worked once, so signing out locked the admin out until a restart. Recover with `vkdg admin set-password` on the host. Failed sign-ins are throttled per client address (5 per 15 min, resolved through `VKDG_TRUSTED_PROXIES` like the data plane) and globally (100 per 15 min), with 429 and `retry-after`. Sessions expire after 12 h unused; a restart still signs everyone out
- **Breaking:** `/v1/messages`, `/v1/chat/completions` and `/v1/images/generations` now require a client API key (`x-api-key` or `Authorization: Bearer`), on every bind address including loopback: a loopback listener behind a reverse proxy is still public. Requests without a valid key get 401 in the client's wire format. `VKDG_DATA_AUTH=off` opts out explicitly and logs a warning at startup
- API keys are persisted in `keys.db` (next to `accounts.db`, `0600`, override with `VKDG_KEYS_DB`). Only a SHA-256 hash and a display prefix (`vkdg_1a2b3c4d`) are stored. A key revoked from another process stops working within 5 s
- The admin API's plaintext in-memory key store is gone: `/admin/v1/keys` manages the same keys `/v1/*` checks

- Client IP comes from the TCP socket. `X-Forwarded-For` is believed only from peers in `VKDG_TRUSTED_PROXIES` (walked right to left); before, any client could claim any address and pass an IP allowlist
- IP allowlist/blocklist entries match by bit mask for IPv4 and IPv6 (`10.16.0.0/12` no longer admits `10.32.0.1`). **Breaking:** the dotted-prefix form (`192.168.1.`) is gone; use CIDR. An unknown client IP now fails a non-empty allowlist instead of skipping it

- **Breaking:** `vkdg serve --config <file>` exits with the validation error when the file is invalid. It used to log a warning and serve an env-derived gateway (different routes, possibly an `ANTHROPIC_API_KEY` passthrough) instead of the one the operator asked for

### Added
- Browser end-to-end tests (`just e2e`, Playwright 1.62, `apps/console/e2e/`) run the real binary with the console embedded and a fresh data directory. They cover sign-in, the full API key lifecycle checked on `/v1/*` after each step, invalid limits, connecting two accounts on one provider, deleting one, and the provider list
- `GET /admin/v1/providers/oauth` lists the providers on this gateway that support interactive login, OAuth plugins included; the console's Connect-account dialog uses it instead of a hardcoded list
- Test-only `fake-oauth` provider for end-to-end tests of the login flows, registered only with `VKDG_E2E_FAKE_OAUTH=1` in debug builds (ignored with an error in release builds). Its device code approves on the first poll; a refresh token containing `revoked` behaves like a revoked login
- Console: Connections show live state as text (healthy, degraded, circuit open, cooling down), in-flight vs capacity and cooldown details, refreshed every 5 s while visible. Requests page refreshes every 3 s (pausable), filters by status and opens a detail drawer with the routing decision. Keys page can edit, regenerate (new secret shown once), disable and enable keys
- Request history and `DecisionRecord` carry the real routing decision: the route that matched (`auto` when none did), attempts, and the candidates left out with why. They used to record no decision and the connection id as the route
- Request history is persisted in `requests.db` next to `accounts.db` (newest 10 000 kept) and records the client key id. It used to live in memory, capped at 1 000, and vanished on every restart. Metadata only: no prompts, responses or credentials
- Key lifecycle: `PATCH /admin/v1/keys/{id}` edits name, scopes, expiry, allowed models and IPs, budget and rpm (absent field = unchanged, `null` clears a limit); `POST .../regenerate` issues a new secret with the same id, policy and usage and kills the old one at once; `POST .../disable` and `.../enable` switch a key off reversibly (status `disabled`), unlike revoke. Policy changes apply to the next request, cached lookups included
- `GET /v1/models` lists the concrete model ids this gateway routes (from routes and connections, globs skipped), filtered by the calling key's `allowed_models`, in OpenAI shape or Anthropic shape when `anthropic-version` is sent. Requires a key like the rest of `/v1/*`
- Route auth hooks: `hooks: { auth: [plugin-name] }` on a route runs installed WASM auth plugins after the client's vkdg key is accepted, before any dispatch path (fusion and prompt chains included). A plugin can only narrow access: a denial, an error, or a missing plugin returns 403. Plugins receive key id, tenant, client address, model and route, never headers or keys. A route naming a plugin that is not installed stops startup and makes a reload keep the current config
- **Breaking:** the unused `PluginHooks` fields `pre_auth`, `rate_limit`, `pre_dispatch`, `on_event` and `on_finish` are removed; they were never reachable from config nor executed
- Console: Accounts page (provider, label, expiry, `needs_login` with the upstream reason, delete, reconnect) and a Connect-account dialog for device-code and PKCE logins (code with copy and countdown, polling at the provider's interval, paste-back for PKCE, retry on expiry). Keys page: expiry, allowed models and IPs, monthly token budget and requests per minute on create; status, restrictions and this month's usage against the budget in the list
- `GET /admin/v1/connections` reports each connection's live state from the data plane (`healthy`, `degraded`, `circuit_open`, `cooldown` with `cooldown_until` and `failure_count`), in-flight requests and `max_concurrent`. It used to report `healthy` and 0 for everything
- `GET /admin/v1/keys` returns `monthly_token_limit`, `requests_per_minute` and `usage_this_month`
- Installed WASM provider plugins are loaded at startup and registered next to the built-ins. A plugin may only take a free provider id: one claiming `anthropic`, `kiro` or any other registered id is refused with a warning, because an adapter receives the credentials of every connection that names it. A plugin that fails to load, including one with an unreadable `manifest.yaml`, is logged by directory name and skipped; the other plugins and the gateway still start
- `GET /admin/v1/plugins` and `vkdg plugin list` show installed plugins even when another plugin's `manifest.yaml` is unreadable; the broken directory is listed under `broken` with its error instead of failing the whole listing
- The plugins directory defaults to `plugins/` next to `accounts.db` (the data volume), not `~/.config` or a path inside the container image
- Per-key usage, budgets and rate limits. Tokens are counted from the response the client receives (SSE or JSON, Anthropic or OpenAI dialect), at end of stream or on client disconnect, and stored per key per month in `keys.db` with an atomic upsert. `monthly_token_limit` refuses a key over budget before routing (429, `insufficient_quota`); `requests_per_minute` is an in-memory per-key window (429, `rate_limit_exceeded`). Both 429s carry `retry-after` (seconds until the window or the month resets) and a `rate_limit_error` type. The budget is a soft cap: it is checked before each request, so streams already in flight can go past it by what they consume. CLI `--monthly-tokens`/`--rpm`, admin `monthly_token_limit`/`requests_per_minute`
- Per-key limits: expiry (`expires_at`), allowed model patterns (`allowed_models`, same `*`/`?` syntax as routes) and allowed client ranges (`allowed_ips`, CIDR, v4/v6). Expired keys get 401; a model or address outside the key's list gets 403. Set them with `vkdg keys create --model 'claude-*' --ip 10.0.0.0/8 --expires-in-days 30` or `POST /admin/v1/keys`. Existing `keys.db` files gain the columns on open
- `vkdg keys create|list|revoke`. `create` prints only the raw key on stdout, so `$(vkdg keys create ci)` works
- `/admin/v1/keys` takes `scopes` (`data_inference`, `data_image`) instead of `role`, and returns `prefix`, `status`, `last_used_at`, `revoked_at`. The console Keys page shows them and keeps revoked keys listed

### Fixed
- Reconnect on an account (`account_id` on `/oauth/{provider}/start` and `/import`) replaces its tokens and clears `needs_login` in place, keeping the id, and evicts the cached credential. It used to create a second account, so connections referencing the revoked one stayed broken
- `limits.ip_allowlist` / `limits.ip_blocklist` were parsed and never enforced. They now apply to every `/v1/*` request, follow hot reload, and are validated at load: an invalid entry names its position (`limits.ip_allowlist[1]`) and a reload with it keeps the current config. `VKDG_TRUSTED_PROXIES` is validated at startup the same way
- Streams re-encoded for the client (Kiro and any decoded provider) now send the full final usage (`input_tokens`, cache fields, `output_tokens`) in `message_delta`. Before, only `output_tokens` reached the wire, so clients and per-key budgets saw 0 input tokens for every Kiro request
- IPv4 clients of a dual-stack listener (`::ffff:a.b.c.d`) are matched as IPv4 by IP lists, per-key lists and `VKDG_TRUSTED_PROXIES`; before, they slipped past blocklists and failed allowlists
- A corrupt policy column in `keys.db` (`expires_at`, `allowed_models`, `allowed_ips`) makes the key unusable instead of unrestricted
- **Behavior change:** route `match_models`, connection `models` and combo patterns share one matcher: `*` anywhere and `?` (one character) now work in routes and connections too, so a literal `?` in a model pattern now matches any single character. The combo matcher no longer backtracks exponentially on patterns like `*a*a*a*b`
- **Breaking:** route strategies `lowest_latency` and `power_of_two_choices` are implemented (p50 latency per connection, measured by the gateway). `weighted` and `last_known_good` were accepted and silently routed round-robin; config now rejects them
- A route that matched but whose targets were all excluded or cooling down fell through to "any connection serving the model", sending its traffic outside its `targets`. Only a model no route matches is auto-routed now; a matched route with no usable target returns 502
- Config hot reload reaches the data plane. Before, a valid edit to the config file bumped the admin revision but requests kept routing with the startup routes and connections. Routes and connection configs now swap on each valid reload; in-flight counters and health survive it, so capacity accounting never resets. One save triggers one reload (notify events are debounced). The watcher follows the config's directory, so atomic saves (editors, `sed -i`, ConfigMap symlink swaps) keep reloading on Linux; a watcher that cannot start logs an error instead of panicking
- A refresh token rejected by the upstream (401, `invalid_grant`, `InvalidGrantException`, `ExpiredTokenException`) now parks the account instead of being retried on every request. New typed error `VkdgError::CredentialRevoked` / `ProviderError::CredentialRevoked`; transient failures (5xx, throttling) still retry
- Refresh failures log the upstream status and message (`401: Bad credentials`) instead of a bare `authorization_failed`
- `/admin/v1/accounts` reports `status: active | needs_login` and `revoked_reason`; the state is stored in `accounts.db` (column added automatically on existing stores)

## [0.1.0-rc1]: 2026-09-26

### Added

**Provider accounts and OAuth login (generic, plugin-driven)**
- `ProviderAdapter::oauth()` hook; `OAuthProvider` gains `login_methods`, device-code (`start_device_login`/`poll_device_login`), PKCE (`start_pkce_login`/`finish_pkce_login`) and `import_token`, all opt-in
- `AccountStore` (SQLite, `0600`) and `auth: { type: account, account: <id> }`; `CredentialManager` refreshes 5 min ahead via the plugin, one refresh per account, and persists the result
- **Breaking (SDK):** `ProviderAdapter::prepare` takes `&Credential { token, extra }` instead of `token: &str`
- CLI: `vkdg login <provider> [--method] [--opt k=v] [--list-methods]`, `vkdg accounts list|remove`
- Admin API: `GET /admin/v1/providers/{id}/login-methods`, `POST /admin/v1/oauth/{provider}/{start,poll,import}`, `GET/DELETE /admin/v1/accounts`

**Plugin ABI made real (2026-09-27)**
- **Fixed:** `wit/` never parsed — `stream` is a WIT keyword, `float32` was renamed `f32`, and the world is `cache-backend`. No plugin could have been built against the published contract; a test now parses the package in CI
- `wit/provider.wit` grows from 5 to 15 functions so a community plugin can match the first-party Kiro adapter: interactive login, per-account credential data, plugin-chosen URL, raw-bytes stream decode, `finish-stream`, runtime model discovery. Verified by compiling Kiro as a `wit-bindgen` guest against the WIT alone
- WASM components now execute rather than only validate: per-call fuel, memory ceiling from the manifest, a fresh `Store` per call, and no host imports beyond WASI resolved against a context that grants nothing
- All 5 roles run as `.wasm`, each with the failure policy its job demands — provider fails the request, compressor is skipped, router falls back to gateway order, cache degrades to a miss, auth denies
- `RegistryManifest` and `RegistryIndex`: checksum mandatory for remote wasm, kebab-case names, exactly one install source; `PluginStore` verifies and compiles before writing
- `vkdg plugin install|list|remove|search` and `/admin/v1/plugins` now work, plus a console plugins screen

**Streaming (2026-09-27)**
- **Fixed:** `ConversationStreamDecoder::finish()` was never called, so a provider that sends no stop event produced a stream with no terminal events
- **Fixed:** the hand-rolled dialect encoders emitted an invalid Anthropic stream (`message_start` as `data: {}`, every block at index 0) and silently dropped tool calls. `StreamEncoder` in `vkdg-operations` is now the single encoding path
- `ConversationEvent` gains `ReasoningDelta`, `ToolCallEnd` and cache token counts

**Kiro (2026-09-27)**
- **Fixed:** the EventStream parser had been fitted to a synthetic fixture — `total_len` excluded the prelude, string headers ignored their `u16` length, and neither CRC was checked. A real Amazon Q stream would desync
- **Fixed:** `reasoningContentEvent` reads `text`, not `content`; `contextUsageEvent` is snake_case, so reading camelCase silently zeroed usage
- Endpoint follows the credential: OAuth accounts use `runtime.{region}.kiro.dev`, `ksk_` API keys use `q.{region}.amazonaws.com` and must omit `profileArn` (AWS answers 403). Kiro's own docs mark `q.*` legacy and slated for deprecation
- Five login methods (builder-id, idc, social, import, api-key) with three refresh paths; prompt caching via `cachePoint`; `x-amzn-codewhisperer-optout` on every request

**Session 2026-09-26 (plugin-first architecture and OmniRoute parity)**

Plugin system:
- WIT interfaces for all 5 plugin types (router, compressor, provider, cache, auth)
- PluginRole + PluginChain with fail-closed semantics for auth
- Cache backends: SQLite exact-match (zero-infra default), Redis/Valkey/DragonflyDB
- SHA-256 cache key derivation (model + messages, excludes temperature)

Routing:
- Auto-scorer with 5 mode packs (ship-fast, cost-saver, quality-first, offline-friendly, balanced)
- ScoredStrategy executing with live quota and latency signals via RoutingHints
- RoutingHints: quota_headroom from QuotaTracker, latency_p50_ms from LatencyTracker (EWMA)
- Session stickiness: SessionRegistry TTL-based pin, preferred connection on multi-turn

Compression:
- CavemanCompressor: 26 regex rules targeting preamble and filler (~30% savings)
- RtkCompressor: content-class detection (stack trace, JSON, file list, diff, command) with class-specific filters
- StackedCompressor: RTK->Caveman pipeline (78-95% savings on tool outputs)
- Compression threshold from combo CompressionPolicy; skip via X-VKDG-Compression: none

Combos:
- Named routing plans with own strategy, compression, cache, and budget policies
- ComboResolver: exact ID match -> glob pattern -> bare model routing
- Active in pipeline: combo targets override route result, combo compression threshold respected

Providers:
- Kiro / Amazon Q provider (`kiro`): API key auth via `KIRO_API_KEY` or device code OAuth (AWS SSO OIDC), fixed `q.us-east-1.amazonaws.com` endpoint
- Kiro exposed end to end: `vkdg setup` wizard, console connection and setup pages, registry manifest, `config.example.yaml`
- Kiro model families: `claude-*`, `gpt-5.6-*`, `minimax-*`, `deepseek-*`, `glm-*`, `qwen3-*`, `auto` (20 model ids)

Credentials and OAuth:
- OAuth2 client_credentials flow with per-connection singleflight and conditional generation write
- Cooldown: exponential backoff (2^(n-1)s, cap 300s, jitter) wired to 429/5xx upstream errors

Pipeline additions:
- IP allowlist/blocklist: extract_client_ip (X-Forwarded-For, X-Real-IP) + IpPolicy step 0
- Think-tag filtering: SseParser strips <think>...</think> by default (opt-in via X-VKDG-Think-Tags: include)
- Override headers: X-VKDG-Mode, X-VKDG-Compression, X-VKDG-Cache, X-VKDG-Think-Tags
- DedupTable RAII: first caller proceeds, subsequent callers register + complete (anti-thundering-herd)
- Global system prompt injection from config
- Memory injection: MemoryStore retrieval prepended to system context
- Eval scoring: EvalScorer records latency/quality score per response

New crates:
- vkdg-cache: CacheBackend trait, SqliteExactCache, RedisExactCache (feature-gated)
- vkdg-combos: Combo, ComboResolver, CompressionPolicy, CachePolicy, BudgetPolicy
- vkdg-memory: MemoryStore, extract_facts, inject_memories
- vkdg-eval: EvalScorer, EvalResult, LatencyMetrics

CLI additions:
- vkdg request explain <id>: fetch and display DecisionRecord from admin API
- vkdg config explain --model <name>: simulate routing without consuming quota
- vkdg replay <fixture>: send recorded request to gateway and optionally assert response
- vkdg doctor: expanded to 6 checks (rustc, RUST_LOG, data plane, admin API, API key, admin URL)

Jobs:
- Webhook dispatch on job state transitions (fire-and-forget, 10s timeout)
- webhook_url field on JobRecord and SQLite store

MCP:
- GET /mcp: minimal tool discovery (route_preview, gateway_health)

Tests:
- 19 BFF tests (msw mock server): getSystem, login, listConnections, previewRoute, etc.
- 6 new spec/scenarios for new contracts
- 250 total Rust tests, 0 failing, 0 clippy warnings

**Phase A: Executable specification**
- `vkdg-core`: fundamental types, attempt state machine (12 states), `DecisionRecord`, `VkdgError`, `Capability`/`CapabilitySet`
- `vkdg-operations`: contracts for `conversation.generate`, `image.generate/edit`, `video.generate/remix`, embedding, audio
- `spec/scenarios/`: 13 executable contract scenarios with RED/GREEN verified

**Phase B: Data vertical**
- `vkdg-http`: HTTP server Hyper/Tower/Axum, frontdoor, admission semaphore (503 before routing), incremental SSE parser (fragmentation at any byte, partial UTF-8, `[DONE]`)
- `vkdg-connections`: `ConnectionCatalog`, RAII `ConnectionGuard`, `CredentialManager`, `eligible_for_operation` fail-closed
- `vkdg-ingress-anthropic`: decode Anthropic Messages → Operation, encode response/stream
- `vkdg-provider-anthropic`: `AnthropicAdapter` implementing `ProviderAdapter`
- Complete pipeline: admission → routing → connection reserve → credential → upstream → passthrough
- Backpressure via `Body::from_stream` lazy; `DecisionRecord` emitted per attempt
- `vkdg-observe`: tracing + OTLP + `DecisionRecordExporter`

**Phase C: Interoperability**
- `vkdg-ingress-openai`: decode Chat Completions → Operation, encode SSE OpenAI wire
- `vkdg-provider-openai`: `OpenAIAdapter` with upstream SSE parsing (parallel tool calls included)
- Fallback 429: retry with second candidate before `committed`; `DecisionRecord` per attempt
- Routes: `/v1/messages` (Anthropic), `/v1/chat/completions` (OpenAI), `/v1/images/generations`

**Phase D: Modalities and extensions**
- `vkdg-config`: versioned `ConfigSnapshot`, hot-reload via `notify`, cross-reference validation; real `vkdg config check`
- `image.generate`: `ImageGenerateRequest/Response`, OpenAI Images API provider
- `vkdg-jobs`: `JobManager` + `SqliteJobStore` (SQLite WAL) + validated state transitions + reconciliation
- `video.generate`: `VideoGenerateRequest/RemixRequest` + async job 202 Accepted + simulated provider
- `vkdg-artifacts`: `InMemoryArtifactStore` with per-tenant ACL, TTL, purge GC
- `vkdg-plugin-host`: Wasmtime 27 + Component Model + `WIT` in `wit/provider.wit`; `install_wasm`/`uninstall` lifecycle
- `vkdg-policy-compress`: context truncation with system message preservation + loss metrics
- `ExternalServiceClient`: circuit breaker (threshold/cooldown) + deadline propagation
- `ProviderAdapter` trait: pipeline decoupled from Anthropic; adding a provider = new crate

### Architecture
- 21 crates with isolated responsibilities
- `CapabilitySet` in `vkdg-core` (no inverted coupling)
- `vkdg-http` split into domain modules: admission, frontdoor, server, app_state, pipeline, provider, sse, upstream, external_service
- `DecisionRecord` without secrets; `TokenState::Debug` omits token

### Tests
- 250 tests, 0 failing, 0 clippy warnings
- Each test documents a plausible defect it defeats
- SSE parser: fragmentation at any byte, partial UTF-8, multiple events
- Pipeline: admission before routing, RAII releases on all exit paths, fallback only before committed
- Config: cross-reference validation, hot-reload rejects invalid config

### Known gaps (Phase E targets)
- WASM `call_prepare` is a stub: full Component Model bindgen in Phase E
- `ModelSummarize` compression is a stub: requires metered internal call
- `video.generate` polling/webhook not implemented: returns 202 Accepted
- OAuth2 credential refresh is a stub: singleflight + encrypted vault in Phase E
- Admin API (`/admin/v1/`) and SvelteKit console pending: `VKDG-frontend-day0.md`

### Breaking changes
- None (first release)

[0.1.0-rc1]: https://github.com/vkdprojects/vkdg/releases/tag/v0.1.0-rc1
