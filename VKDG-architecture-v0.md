# VKDG: reference architecture v0.1

**Status:** architecture proposal v0.3; 25/09/2026. **Goal:** central AI API, open source and in Rust, for agents and applications, with a SvelteKit operations console as a separate process from day 0. Exposes the native VKDG API and facades compatible with well-known protocols; connects multiple accounts/providers for text, image, audio, and video through extensible converters, policies, route composition, and diagnostics. The console contract and deployment plan are in `VKDG-frontend-day0.md`.

## 1. Scope and principles

VKDG receives calls from agent CLIs and applications, authenticates each client, understands the requested operation, selects an eligible **account connection**, manages the provider credential, translates when necessary, and forwards a complete response, a stream, or an async job. A person can operate several simultaneous agents through a single OAuth account, up to the real limits of that account and provider rules. The gateway does not create quota nor guarantee concurrency that the upstream service does not allow. The initial product does not execute agent tools and workspaces.

The parallel with SIP is useful as an **interconnection analogy**: clients and providers speak distinct protocols, and VKDG negotiates capabilities, chooses destination, applies policy, and records the path. It does not imply implementing SIP, nor inventing a single message format capable of representing all modalities without loss.

**Non-negotiable principles**

1. Protocol and security correctness precedes optimization. Performance is measured on reference hardware and workloads.
2. The data path has explicit memory, CPU time, concurrency, and queue limits. Admission can reject work when the budget is exhausted.
3. Streaming works with propagated cancellation and end-to-end backpressure. No unbounded channel in the request path.
4. A plugin declares capabilities; the host limits access to secrets, network, storage, and time. Plugin failure has a behavior configured by policy type, never a universal `continue`.
5. Requests and streams use an immutable version of configuration until they finish. Changes are validated and activated atomically.
6. The system explains its decisions without requiring recording of the prompt or response tokens.
7. Each new behavior starts with an observable scenario and a failure test. We do not require an artificial test for every file; we require evidence for every altered contract and risk.
8. The repository is agent-navigable: short and canonical documentation, reproducible commands, stable contracts, and structured errors.
9. Client, session, provider account, and model are distinct identities. A VKDG key is never confused with the upstream OAuth token.
10. Agent compatibility means protocol testing and observable experience with the real CLI, including tool calls, thinking, compaction, cancellation, and subagents; an isolated HTTP 200 is not enough.
11. `Universal` means consistent input, capability discovery, and explicit incompatibility error; it does not mean any client can use every capability of every provider.
12. The core remains owner of identity, admission, routing, lifecycle, transport, and observability. Plugins only extend documented contracts and granted capabilities.

### 1.1 Public surface and translation map

| Input | Purpose | Fidelity rule |
| --- | --- | --- |
| Native VKDG API, versioned | Stable contract for new applications; explicit operations and capabilities | May expose own features without mimicking third-party API limitations |
| OpenAI compatible: Responses, Chat Completions, images as implemented | SDK and framework integration, including Mastra applications that accept a compatible endpoint | Each endpoint only announces tested features; events, tools, usage, and errors follow their declared contract |
| Anthropic compatible: Messages | Claude Code and other clients that use the protocol | Preserves block/event sequence and semantics observed in real testing |
| Other protocols, via ingress plugins | Specialized clients | Installation does not automatically make every client/provider pair compatible |

A call may be translated `Anthropic -> VKDG operation -> provider X -> Anthropic`. If the translation loses a required feature, selection excludes the provider or returns `unsupported_capability` before sending. A `passthrough` mode can preserve protocol bytes when input and output match and no policy requires inspection. `vendor_extensions` have a namespace, size limit, and transport rule; they never become a silent loss.

## 2. Component overview

```text
Client -> Ingress adapter -> Typed Operation -> Policies -> Router -> Connection -> Provider adapter -> Provider
              |                 |                 |             |                |
              +-----------------+---- DecisionRecord / trace -----+----------------+
                                          |
                                   Async exporter

Control: config file -> validation -> immutable snapshot -> atomic swap
Extensions: ingress / provider / policy / telemetry via versioned contract
```

### 2.1 Data plane

`frontdoor`: accepts HTTP, limits body and headers, associates `request_id`/`trace_id`, resolves identity and input protocol. TLS can be terminated here or by a trusted proxy; identity headers coming from untrusted clients are never accepted as authority.

`admission`: global and per-tenant semaphores for calls and streams, memory budget, small bounded queue, optional priority with fairness. Accounting belongs to core; each entry acquires a RAII lease released on success, error, cancellation, and isolated panic. Defines overload responses and `Retry-After` where appropriate.

`routing`: filters **account connections**, not just provider names, by modality/operation, capability, model, authorized identity, quota, cooldown, health, and concurrency limit. An authenticated session id can receive affinity to the connection used before. Connection selection and reservation are atomic. Provider circuit, connection cooldown, and model lock on a connection are different states; a 429 on one account does not take down the entire provider. An attempt never crosses incompatible protocols by mere fallback.

`ingress_adapter`: validates and decodes the client protocol, produces a typed operation or a passthrough plan; on return encodes response/events/error for the same client. Explicitly versions wire protocol differences. CLI configurators are auxiliary tools and do not replace this adapter.

`provider_adapter`: publishes a capability/model descriptor and describes `prepare` (method, allowed destination, authorized headers, body or stream), `decode_event`/`decode_complete`/`decode_job`, `classify_error`, and idempotency. Core owns HTTP, secret injection, DNS, limits, and cancellation. Official adapters are initially compiled Rust crates. The adapter semantics are separate from how it is loaded.

`transport`: connection pool, TLS, HTTP/1.1 and HTTP/2 according to provider requirement, per-phase deadlines, DNS and allowed destinations, disconnect propagation, and slow-write control. Do not `.collect()` the streaming body. Do not block Tokio workers with heavy compression, database, or synchronous plugins. Large upload/download uses streaming with a limit and artifact storage under explicit policy.

`stream`: passthrough mode when protocols and policies allow; transformation mode with incremental parser and typed events when necessary. HTTP read boundaries do not equal SSE, message, or token boundaries. Limit frame, event, and partial sequence size. A failure after bytes have been sent to the client terminates the stream with a protocol-compatible indication or closes the connection; never simulate a new response from another provider.

`telemetry`: typed decision events and fields with controlled cardinality. Export outside the response loop via bounded channel. If the exporter becomes slow, preserve the request and count telemetry losses, except in strict audit mode, which requires its own policy.

`connections/credentials`: catalog of upstream connections with `connection_id`, provider, account, capability, token state, quotas, limits, and availability. `CredentialManager` runs proactive and on-demand refresh with **singleflight per connection**, stores refresh tokens encrypted, and performs conditional writes per generation so that a delayed process does not overwrite a new token. Do not distribute a refresh token across multiple CLIs: the gateway is its owner; clients receive their own VKDG credentials. A refresh error on one account does not invalidate other accounts.

`session_affinity`: extracts a stable per-protocol identifier and binds it to an authenticated client and connection. Affinity avoids switching accounts on every turn, protects remote context, and favors cache; it is not synonymous with exclusivity. Pins have TTL, reason for repin, and rule when eligibility is lost. Active call reservations are released at end of stream, error, or cancellation. If an integration requires durable exclusive ownership, that is a separate contract with lease and generation, not a side effect of affinity.

`job_manager`: for async providers, persists public `job_id`, owner identity, connection/provider, encrypted `upstream_job_id` when sensitive, state, deadlines, idempotency key, and artifact reference. Creation, polling, webhook, and cancellation are separate operations with authentication and deduplication. A restart cannot cause an accepted job to disappear. Polling workers have concurrency and cadence limits; jobs do not hold a stream lease throughout the entire generation.

### 2.1.1 Operations, capabilities, and lifecycles

| Native operation family | Execution shape | Capabilities verified before routing |
| --- | --- | --- |
| `conversation.generate` | single response or streamed events | tools, reasoning, vision, JSON/schema, events, context, compaction |
| `embedding.create` | single response or batch with limits | dimension, batch size and cardinality |
| `image.generate` / `image.edit` | response or job depending on backend | input image, mask, format, resolution, number of outputs |
| `audio.transcribe` / `audio.synthesize` | upload/stream/job depending on real capability | format, duration, language, and output mode |
| `video.generate` / `video.remix` | job with polling/cancellation and artifact | duration, resolution, reference input, audio, callback |

These names are a domain proposal, not a promise of already-implemented endpoints. The native API may expose `/vkdg/v1/operations/<family>` and `/vkdg/v1/jobs/{id}`; schema and routes are frozen only after integration tests. Capability is a descriptor versioned by `provider + connection + model + operation` with input, output, streaming, idempotency, and limit characteristics. The router intersects what the client requires with what the provider delivers and what the translation can preserve; the result is `supported | supported_with_declared_loss | unsupported`. Default: require full support for required features. Optional losses depend on explicit consumer opt-in and appear in diagnostics.

An image can be an immediate result or a referenced artifact; video is modeled as a durable job even when some backend delivers a quick response. Binary bodies and URLs are not copied into a universal JSON envelope; the core uses tenant-authorized artifact handles, validates MIME/size, TTL, origin, and download limits. Different contracts may preserve provider-specific metadata in the provider namespace.

### 2.2 Control plane

Initial version: declarative configuration in a file, validated offline with `vkdg config check`, converted to `ConfigSnapshot { version, routes, providers, plugins, limits }`. Broken references, cycles, missing secrets, incompatible plugins, and ambiguous priorities fail validation. The loader prepares new resources before swapping the snapshot; the old one drains until its associated requests finish. Reload must have a maximum retention time for interminable streams. Both CLI and console propose changes to the same Rust control service, with expected version, validation, atomic write, and audit; the browser never modifies a file or database directly.

**Day 0:** versioned administrative API in Rust and SvelteKit console as an independent process/deploy. Authentication, sessions, access control, credentials, configuration versioning, and audit belong to Rust. SvelteKit serves the web experience and acts as BFF with limited endpoints; it does not store its own operational state. The data plane remains usable without the web process and does not consult the console per request. See `VKDG-frontend-day0.md` for modules, delivery sequence, contract, and tests.

### 2.3 Local clients, remote gateway, and OAuth accounts

**Main topology:** Claude Code, Codex, OMP, or another CLI remains installed on the developer's machine; its files, tools, MCP, and local sessions remain in the CLI. VKDG runs on the same machine or on the server. The CLI speaks its HTTP protocol with VKDG using a client token. VKDG holds one or more upstream OAuth connections and uses them for multiple concurrent streams, with affinity, quota, and limits per account. Configuration should be generated by a client adapter and offer `dry-run`, backup, and diagnostics; environment variables and CLI profiles are an integration, not an implementation of the provider protocol.

**Kiro as provider vs Kiro CLI as client:** a Kiro account connection can serve requests that arrived from a supported client and were translated to the Kiro protocol, subject to real conformance. This **does not prove** that the native Kiro CLI can be pointed as an HTTP client to another gateway; its official documentation exposes Kiro as an ACP agent via `kiro-cli acp`. If the requirement is to remotely control the Kiro CLI itself, we need an agent/ACP adapter or a documented custom endpoint capability. This mode runs a process and may involve tools/workspace, so it is a separate execution plan from the model proxy.

**Native compatibility as quality target:** test a real local CLI against VKDG, including long flows, tool calls, thinking, images, usage, and cancellation, comparing with the same CLI connected directly to the service. Do not declare universal equivalence: incompatible fields/features may fail explicitly. The same OAuth login does not multiply quota and may not be authorized for all CLI/provider combinations; each integration requires technical verification and applicable conditions.

**Two-dimensional matrix:** `ClientAdapter { Claude Code/Messages, Codex/Responses, OpenCode, ... }` is distinct from `ProviderAdapter { Anthropic API, Claude Code OAuth, Kiro account, Codex account, ... }`. For each supported pair, the project declares the preserved and blocked features. Do not automatically promise the Cartesian product of all pairs.

## 3. Fundamental contracts

### 3.1 Request envelope

`RequestEnvelope`: `request_id`, `client_id`, derived identity, authenticated `session_key`, tenant, API type, requested model, requested capabilities, relevant parameters, estimated size, deadline, and content reference. `session_key` does not come only from User-Agent and is not a security identity. Only fields necessary for the current phase are copied. Headers and secrets never enter `Debug`/logs through automatic derivation.

The internal model does **not** promise a universal `Message` that preserves all APIs without loss. It defines `Operation` as a sum type by family, with payloads and events versioned per operation. Input interfaces live in distinct crates: start with a compatible API and one native API case, include `/v1/responses` after a specific conformance proof before declaring support for agents that depend on its particularities.

### 3.2 Attempt state machine

`received -> authenticated -> admitted -> account_reserved -> credential_ready -> prepared -> upstream_open -> committed -> completed | partial | cancelled | failed`. Account reservation and release cover all exits; refresh occurs outside the selection lock, maintaining generation and avoiding duplication.

`committed` means a response has already been sent to the client in a way that cannot be transparently restarted. Retry rules distinguish unsent connection, request possibly accepted by upstream, response with headers but no content bytes, and started stream. An upstream may execute billable work before the first token; retry of a non-idempotent operation requires explicit policy. A single request has an aggregate attempt budget and shared deadline.

### 3.3 Event and error

For streaming conversation, minimal normalized events: `Started`, `OutputDelta`, `ToolCallDelta`, `Usage`, `Completed`, `Failed`; preserve provider-specific metadata with namespace and limits, without silently dropping them. Jobs use state events `queued | running | succeeded | failed | cancelled | expired` and artifact references; do not pretend that video frames are conversation tokens. Do not infer exact usage from chunks when the provider does not supply it; expose `reported | estimated | unknown`.

Structured error: `code`, `phase`, `provider`, `attempt`, `retryable`, `committed`, `request_id`, `cause_safe`. Internal sensitive causes stay in the protected trace. Error catalog is versioned, with documentation and compatibility tests.

### 3.4 Combos and compression

A **combo** is a public name for a versioned plan of candidates and policies, for example `coding-fast`, `visual-draft`, or `video-quality`. Its schema declares operation family, capability requirements, allowed connections, weights/priorities, cost and latency limits, attempt budget, affinity, and fallback. Resolution produces an eligible list with auditable reasons. `fallback` only occurs when permitted by commit/idempotency state; hedging and parallel dispatch are outside the first release due to risk of duplicating cost and effects. Chained composition between modalities (text produces prompt for image and then video) is an opt-in workflow with its own persistence, billing, and failure semantics; it does not appear implicitly in a routing combo.

**Compression has three distinct meanings:** HTTP content encoding in transport; conversation history/context compaction; and media transformation (image/video). The first is HTTP negotiation with limits against abusive expansion. The second is an optional `pre_dispatch` policy that applies a declared strategy (`truncate`, model-based summary, or other) with budget, provenance, and loss measurement, respecting system messages, tools, and cache requirements; it never modifies context without route consent. Media requires a specific plugin with quality and format limits, without passing through a text compactor. Model-based summaries are internal accounted calls, with deadline and recursion limit.

## 4. Plugins and SDKs for humans and AIs

**Taxonomy:** `ingress` implements a public protocol; `provider` implements conversion and descriptor for a backend; `policy` implements selection, compression, and filters; `observer` exports decisions; `artifact-store` implements external storage per the core port. A provider plugin can register text, image, and video separately; installing the package does not activate all those capabilities without testing and configuration. Identity, secret policy, admission, state transitions, transport, and decision records remain in the core.

**Logical contract:** manifest with identifier, semantic version, ABI/WIT version, plugin types, operations/capabilities, offered hooks, requested resources, configuration JSON schema, migrations, failure policy, and minimum/maximum compatibility. Hooks are per-phase with typed input/output: `on_decode`, `pre_route`, `route_policy`, `pre_dispatch`, `on_event` (only explicitly streaming-safe plugins), `on_complete`, `on_job_transition`, `on_finish`. Hooks cannot reorder fundamental phases nor bypass identity/admission. A plugin cannot declare `streaming-safe` merely by wish: a conformance suite and latency budget validate that.

**Execution:** official protocol adapters start as compiled Rust crates; third-party extensions can use WASM components with versioned WIT and permissions granted by the operator. A provider WASM generates a request plan and transforms typed events; HTTP calls, OAuth refresh, and stream control are executed by the host. For binary payload, use a reference/bounded stream, avoiding copying entire video across the ABI. Plugins in other languages can also run as external processes/services via a versioned protocol, with deadline, pool, and circuit breaker, especially for slow operations; do not do RPC per token. The language/SDK support matrix is proven by runnable fixtures before being announced. Native `.so` libraries are not a stable ABI candidate.

**Fast path:** the hook plan is compiled per route in each configuration snapshot. A route without transformation passes through transport in passthrough mode with bounded buffers; plugins that inspect the response opt into incremental events and pay a measured cost. A response hook that requires the full body makes the route incompatible with streaming and must fail validation. This is an architectural inference reinforced by existing gateways: response-phase extensions can activate buffering and prevent streaming.

**Failure by class:** authentication, authorization, and limits fail closed; optional enrichment may fail open with an explicit event; routing and transformation that define the response fail with an error when there is no validated alternative. Each plugin has memory, execution, simultaneous instance, input/output size, and host call limits. WASM resources do not cross tenants inadvertently. Reload retains the old version while requests use it; plugins with durable state require explicit migration.

**WordPress-level experience:** `vkdg plugin init <type>`, `test`, `pack`, `install`, `inspect`, `enable`, `disable`, `rollback`. Provider, ingress, and policy templates include manifest, minimal example, backend simulator, success/error/stream fixtures, conformance tests, and schema-generated docs. Packages installed from a verifiable artifact, with checksum/signature when distributed, permissions inventory, and snapshot-based activation; a catalog/marketplace may emerge later. `plugin doctor` explains incompatible ABI, missing capability, and fixture failure. Simple hooks like WordPress in the authoring experience; isolated execution and stricter limits for a multi-tenant gateway.

## 5. Storage: separating three needs

| Need | Data | First implementation | Availability rule |
| --- | --- | --- | --- |
| Ephemeral execution state | leases, circuits, pools, snapshot, local metrics | bounded memory | loss on restart is expected and documented |
| Configuration and secrets | routes, tenants, models, OAuth refresh token, access token, token versions | versioned file for configuration; encrypted vault for credentials, master key outside the database | refresh has atomic persistence and generation; lost key requires explicit re-login |
| Operational record | request metadata, attempt, usage, fallback reason, errors | async exporter + optional local SQLite | saturation has counter, signaling, and clear discard policy; never accidental stream blocking |
| Jobs and artifacts | job state, owner, upstream id, idempotency, file references | durable SQLite for jobs on single instance; object/file storage with TTL for media | creation only accepted after persistence; recovery reconciles pending jobs after restart |

For a single instance, SQLite WAL is a practical option for history and diagnostics, with retention, checkpoint, and a controlled writer. WAL allows concurrent readers with writes, but only one writer at a time; one event per token would be a design error. Write events per attempt/finalization, in batches when possible. For multiple replicas with shared administration, PostgreSQL can be the control plane and history backend. Each replica compiles a local snapshot; no SQL query per chunk or token. Redis/Valkey only if a measured need for distributed state justifies the semantics and its degradation.

**Conscious exception to the no-database-in-hot-path principle:** the first account lease acquisition and OAuth refresh may require durable coordination between replicas. In single-instance mode, per-connection lock and local vault resolve this. In distributed mode, we require token atomicity/fencing and distributed reservation with tested semantics; do not promise safe account sharing between pods before this experiment. History writing remains async.

**Content policy:** by default do not store prompt, response, Authorization, or arbitrary headers. Record hash/count/size and allowed metadata. A payload capture mode for diagnostics must be opt-in, with authorization, redaction, short TTL, access protection, and explicit warning; reproduction uses sanitized fixtures, never real credentials. Retention and event loss must be visible.

## 6. Debuggability as a product feature

Each decision produces a machine-readable `DecisionRecord`: config version, pseudonymized client and session, route, required capabilities, excluded candidate accounts with reason, chosen connection, credential age/state without secret values, quota/concurrency, circuit/cooldown, attempts, transitions, per-phase times, result, and plugin/versioning. The record omits content by default. `request_id` links response, log, trace, metrics, and CLI query.

Planned commands: `vkdg doctor` (environment and dependencies), `vkdg config explain --model ... --tenant ...` (simulated route), `vkdg request explain <id>` (observed decisions), `vkdg replay <fixture>` (deterministic fake transport), `vkdg plugins inspect`, `vkdg bench`. Explain commands accept `--json` with a versioned schema and concise human output. `replay` reproduces gateway behavior with recorded or generated responses; it cannot automatically reproduce real remote effects.

Structured logs with `tracing`; HTTP trace propagation when trusted; spans per ingress, admission, routing, attempt, preparation, upstream, streaming, and export. Metrics: active, rejected, estimated memory/buffer, TTFT, duration, bytes, errors per phase, plugin timeout/trap, queues, reported usage, and discarded diagnostic events. Avoid labels per request ID or user. Align HTTP and GenAI semantics with OpenTelemetry, versioning adopted conventions; sensitive content is opt-in.

## 7. Test first that actually tests contracts

**Before implementing a change:** write a case in `spec/scenarios/*.yaml` or a Rust test that reproduces external behavior, plus relevant invariants. The initial test must fail for the expected reason (red); implement the minimum; run the focal test; only then the regression suite and log/error review. For pure refactor, the existing contract must be protected before the change. A new documentation file, trivial type, or fixture without new behavior does not need its own test.

| Layer | Test type | Essential examples |
| --- | --- | --- |
| Parser/protocol | table, reviewed snapshots, property/fuzz | SSE fragmented at any byte, partial UTF-8, exceeded size, `[DONE]`, duplicate events |
| State machine | unit and property | retry before/after commit, cancellation at each state, aggregate budget |
| Concurrency | modeled schedules where feasible | config swap with streams, leases released once, half-open circuit |
| Adapter | shared contracts | tool calls, unknown usage, status/headers, malformed error, slow upstream |
| Multimodal matrix | ingress/provider/operation pairs | image with mask, incompatible format, video job, expired artifact, cancelled upload |
| Combos/compression | policy test with budget | candidate excluded by capability, fallback without commit, aggregate cost, summary recursion, tool preservation |
| Integration | two in-process fake servers | disconnect, backpressure, fallback, flush, credentials, HTTP/2 when needed |
| Plugin | ABI/capability conformance | incompatible version, trap, timeout, memory, secret and network denied |
| OAuth and accounts | state machine, race, and restart | N streams on same account, concurrent/rotating refresh, revoked token, revocation across pods, affinity, isolated 429 per connection |
| Real agent client | conformance and versioned smoke | Claude Code/Messages, Codex/Responses, tool calls, reasoning, compaction, cancellation; Kiro CLI/ACP only in supported mode |
| Storage | migrations and failures | checkpoint, crash, full volume, duplication, recovery, and retention |
| Jobs | integration under restart and race | repeated creation by idempotency key, duplicate callback, polling after restart, cross-tenant access denied |
| Performance | versioned scenarios | idle streams, slow clients, passthrough, translation, and plugin under load |

Candidate tools: `proptest` for properties and shrinking; `loom` only for own instrumentable synchronization structures; `tokio-test` for predictable I/O; `insta` for serialized contracts with human approval. Continuous parser fuzzing in dedicated CI; small regression corpus in each PR CI. Benchmarks report environment and variance, never a lone number. A critical path change requires profile and before/after comparison.

**Gates:** `fmt`, lint, build, unit tests, and contracts on every PR; integration and failure tests in CI; long fuzz/concurrency/load suite on schedule or before release. Flaky tests are bugs with an owner, not permanent exceptions.

## 8. AI-friendly repository

```text
README.md                      vision, quickstart, and map
AGENTS.md                      short operational instructions for agents
docs/architecture/             vision, invariants, contracts, failure flows
docs/adr/                      numbered decisions and alternatives
docs/contributing/             safe change, review, and commands
spec/protocols/                external fixtures and contracts
spec/scenarios/                deterministic failure cases
wit/                           versioned ABI for WASM extensions
crates/vkdg-core/              states and decisions, no transport/storage
crates/vkdg-operations/        contracts per family and versioned capabilities
crates/vkdg-http/              ingress and transport
crates/vkdg-routing/           eligibility and strategies
crates/vkdg-combos/            route plans, budget, and fallback
crates/vkdg-jobs/              async lifecycle and idempotency
crates/vkdg-artifacts/         handles, ACL, TTL, and media streaming
crates/vkdg-connections/       account catalog, reservation, affinity, quota, and OAuth
crates/vkdg-ingress-*/         public protocols, ingress, and response
crates/vkdg-client-*/          CLI configuration and diagnostics
crates/vkdg-provider-*/        official adapters
crates/vkdg-plugin-host/       sandbox and host capabilities
crates/vkdg-storage/           ports and optional implementations
crates/vkdg-observe/           diagnostics and OTEL
crates/vkdg-cli/               operational commands
tests/conformance/             shared suite with fake upstream
tests/faults/                  failure cases per phase
bench/                         reproducible workloads and results
xtask/                         documented development tasks
```

Crates are responsibility boundaries, not an obligation to create all of them before the first working flow. Dependencies point toward the domain; `vkdg-core` does not know about database, HTTP runtime, or Wasmtime. `AGENTS.md` defines: product goal, repo map, order for modifying contracts, test commands, sensitive areas, where to add fixtures, and evidence checklist. Do not duplicate documentation; each crate has a `README` of up to one page with owner, interfaces, invariants, and examples.

Every tool error reports cause and next action. Examples: invalid config points to file/field and expected value; contract test points to fixture and differences; plugin failure points to id/version/hook without leaking input. CI publishes small artifacts: filtered structured logs, property test seed, minimal fixture, and benchmark profile. Local development should not require a provider key: fake upstream covers nearly the entire suite; real tests are optional smokes with secrets outside logs.

## 9. Technical choices and hypotheses to prove

- **Rust/Tokio** for the data path. **Hyper/Tower** as the reference candidate; run an equivalent spike in **Pingora** and choose by protocol compatibility, observability ease, and behavior under load, not reputation.
- **Wasmtime + Component Model/WIT** as the isolated ABI candidate. Verify required support in two languages, instantiation cost, safe reuse, and hook times. Extension does not receive general HTTP by default.
- **SQLite** on single instance for the OAuth connection vault and its durable rotation; call history is optional and logically separate. PostgreSQL for shared control plane if there is distributed deployment. Operation without persistence is only valid for providers with static external keys and no managed OAuth promise.
- **OpenTelemetry/tracing** as the observability contract, with attribute versioning and opt-in content capture.
- **Administrative interface from day 0** in SvelteKit, deployed separately after stabilizing each corresponding control API slice. A UI cannot be a separate source of truth; changes follow the same validation, audit, and activation contract as the CLI.

## 10. Milestones and exit criteria

**Phase A: executable specification:** inventory of requests from an agent client and an application, `conversation` and `image` contracts, chosen protocol, states, resource budget, OAuth account lifecycle, capability matrix, and failure corpus; decisions recorded in ADRs. Criterion: tests describe failures before implementation and distinguish ingress, operation, client, and upstream account.

**Phase B: data vertical:** native VKDG conversation API, Anthropic or OpenAI compatible endpoint tested with a real client, one authorized upstream OAuth/API key account, two concurrent streams over the same connection, singleflight refresh when applicable, streaming/passthrough, cancellation, limits, telemetry, and fake upstream. Criterion: slow client does not grow memory without bound; cancellation releases reservations; concurrent refresh does not lose token; metrics and request explain point to each phase.

**Phase C: interoperability:** second provider with distinct format, second ingress protocol, per-event translation, capabilities, simple fallback combo, and errors. Criterion: tool call, usage, partial stream, and retry matrices pass on both; semantic loss is rejected or documented.

**Phase D: modalities and extensions:** native image and via chosen compatible protocol, video job on simulated provider, artifact with ACL/TTL, provider WASM plugin, optional compression policy plugin, example external service, atomic configuration, optional history, and replay. Criterion: restart recovers jobs; stalled plugin respects budget; DB-free mode remains operational for operations without jobs; fixture proves plugin installation and rollback without editing core.

**Phase E: scale/release:** load with tens, hundreds, and thousands of streams depending on available hardware; compare passthrough/translation/plugin; profiler identifies bottlenecks; SDK and operational documentation; release candidate. Publish numbers with environment, load, and margin, without universal claims.

**Release blockers:** credential leakage, unbounded memory growth, retry after commit, tenant mixing, silent loss of required events, unexplained TTFT regression, invalid config activated, and protocol error in a supported agent client.

## 11. Open decisions with experiments

1. Initial input: start with the VKDG conversation API and one real agent client (Claude Code/Messages or Codex/Responses), with a conformance corpus and the corresponding OAuth/API key account flow. Isolated Chat Completions does not validate the central goal.
2. Hyper/Tower versus Pingora: implement the same stream with cancellation, translation, and failures; compare profile and maintenance.
3. WIT contract: decide event granularity and host calls after measuring payload, latency, and instance reuse.
4. Persistence scope: define whether the first deployment needs durable local history or only exported traces.
5. SLO and capacity: declare concurrency target, RAM/CPU budget, additional TTFT, and availability before the benchmark; do not invent numeric goals without an environment.
6. Same OAuth provider on multiple CLIs: validate authorization and capabilities documented per provider; measure a single account under N sessions and N simultaneous calls. Separate local limit from real upstream. For Kiro, differentiate Kiro as upstream account from the native Kiro CLI as client.
7. Choose the first image/video pair and the durable job storage after a real/simulated provider fixture; do not block the API design by the contract of a single vendor.
8. Measure three plugin paths (`passthrough`, WASM pre-route, and event conversion) under load to define hook budgets; choose the second language SDK by real development experience and conformance.

## 12. Competitive review: OmniRoute and 9Router (25/09/2026)

**Correction of the initial analysis:** OmniRoute already covers many items that v0.1 treated as possible VKDG differentiators. The current README and API reference take precedence over the `ARCHITECTURE.md` page, whose header still identifies itself with v3.8.0 and old numbers. Provider/model counts vary by commit, catalog, and counting method; they are project claims, not independent benchmark metrics.

| Dimension | OmniRoute currently documented | 9Router currently documented | Consequence for VKDG |
| --- | --- | --- | --- |
| Input | `/v1/chat/completions`, `/v1/messages`, `/v1/responses`, Gemini, Ollama, WS, and integration aliases | OpenAI compatible with Claude/Gemini/Cursor/Kiro/Vertex translation | Multiple facades are a parity requirement, not a differentiator in themselves |
| Modalities | Embeddings, image generate/edit, STT/TTS, video, music, OCR, search, rerank, moderation, classification/segmentation, Files/Batch | Chat and documented support for STT/TTS/local embeddings, plus other providers | Each operation needs a fidelity matrix, lifecycle, and test; a catalog does not prove equivalence of all backends |
| Routing | `auto/*` without configuration; 19 announced strategies including fusion/pipeline; manual combos, quota, cost, latency, quality, and affinity | 3 tiers, combos, quota, multi-account fallback | VKDG should start with safe commit semantics, idempotency, eligibility, and explanation; advanced strategies require a separate specification |
| Accounts | OAuth/API key, refresh, quota-based selection, affinity, optional exclusive lease; separate provider circuit, connection cooldown, model lockout | OAuth/API key, refresh, multi-account, quota, round-robin | Client/account/connection/session separation remains essential; test real upstream limits |
| Compression | RTK, Caveman, aging, summarization, stacked pipelines, and profiles | RTK, Headroom, Caveman, and Ponytail | Measure output quality and semantics, not savings percentage as a guarantee |
| Extensions | npm CLI plugin; request/response/error hook SDK; child process and catalog marketplace. Provider manifest for sidecars is a distinct contract | Built-in adapters and providers; extensibility per repository | VKDG cannot claim exclusivity of WordPress-style plugins; focus on provider/protocol plugin with versioned conformance, isolation, and safe streaming |
| Expanded surface | Dashboard, analytics/costs, MCP/A2A/ACP, skills/memory, cloud agents, webhooks, playground, cache, and sidecars | Dashboard, logs, cloud sync, token savers, and local/remote deploy | Delimit gateway core; agent features are possible adjacent modules, not a first-version condition |

**Detail that matters:** the OmniRoute marketplace allows browsing the catalog and installing by local path, but the documentation records that one-click installation directly from a catalog entry is still pending. The request/response/error hook SDK is different from the `provider plugin manifest`: the latter exports safe JSON metadata from the TypeScript registry to Bifrost/CLIProxyAPI and does not automatically install new provider executors. The sidecar contract explicitly states that providers with OAuth and custom executors remain in the TypeScript path until observed parity exists. OmniRoute can also supervise 9Router as a local service and consume it as a subprovider, showing that the two are not mutually exclusive rivals.

**Observable risks that inform VKDG:** `auto/<category>:<tier>` documents a `fail-open` filter when no model meets the constraint; for client mandatory requirements, VKDG should prefer explicit error. The compression documentation calls certain modes safe, but also describes removing repeated instructions and transforming tool results; quality requires per-task evaluation and a reversible trail. Responses after commit, possibly stale quotas, and media jobs must be tested under failure, not inferred from a feature table. These are architectural inferences from published contracts, not claims of proven defects in the implementations.

**Strategic implication:** competing by provider count or number of strategies is a moving target. The strong hypothesis for VKDG is a Rust core with bounded concurrency and resources, native per-operation API, public interoperability matrix, independently testable provider and ingress plugins, stream/job contracts, and reproducible diagnostics. This hypothesis only becomes an advantage if a fair benchmark and a real compatibility suite demonstrate a gain. Review this section against specific commits/tags before turning the comparison into public material.

## Technical sources consulted

- Tokio, tasks and queues: https://tokio.rs/tokio/tutorial/spawning ; https://tokio.rs/tokio/tutorial/channels
- Hyper HTTP body and Tower: https://hyper.rs/guides/1/server/echo/ ; https://docs.rs/tower/latest/tower/
- Pingora: https://github.com/cloudflare/pingora ; https://github.com/cloudflare/pingora/blob/main/docs/user_guide/phase.md
- Wasmtime and WIT: https://docs.rs/wasmtime/latest/wasmtime/ ; https://component-model.bytecodealliance.org/design/wit.html
- SQLite WAL: https://www.sqlite.org/wal.html
- Rust tests: https://docs.rs/proptest/latest/proptest/ ; https://docs.rs/loom/latest/loom/ ; https://docs.rs/tokio-test/latest/tokio_test/ ; https://docs.rs/insta/latest/insta/
- tracing and OpenTelemetry: https://docs.rs/tracing/latest/tracing/ ; https://opentelemetry.io/docs/specs/semconv/
- OmniRoute: https://github.com/diegosouzapw/OmniRoute/wiki/CLI-Integrations ; https://github.com/diegosouzapw/OmniRoute/wiki/Kiro-Setup ; https://github.com/diegosouzapw/OmniRoute/blob/release/v3.8.51/docs/architecture/RESILIENCE_GUIDE.md
- OmniRoute current architecture: https://github.com/diegosouzapw/OmniRoute/blob/main/docs/architecture/ARCHITECTURE.md
- Hooks/extensibility: https://developer.wordpress.org/plugins/hooks/ ; https://docs.konghq.com/gateway/latest/plugin-development/custom-logic/ ; https://docs.konghq.com/hub/kong-inc/ai-proxy-advanced/how-to/streaming/
- Video jobs: https://platform.openai.com/docs/api-reference/videos
- OmniRoute API and features: https://github.com/diegosouzapw/OmniRoute/blob/main/docs/reference/API_REFERENCE.md ; https://github.com/diegosouzapw/OmniRoute/blob/main/docs/routing/AUTO-COMBO.md ; https://github.com/diegosouzapw/OmniRoute/blob/main/docs/compression/COMPRESSION_GUIDE.md
- OmniRoute plugins/sidecars: https://github.com/diegosouzapw/OmniRoute/blob/main/docs/frameworks/PLUGIN_MARKETPLACE.md ; https://github.com/diegosouzapw/OmniRoute/blob/main/docs/frameworks/PLUGIN_SDK.md ; https://github.com/diegosouzapw/OmniRoute/blob/main/docs/reference/PROVIDER_PLUGIN_MANIFEST.md ; https://github.com/diegosouzapw/OmniRoute/blob/main/docs/architecture/ROUTER_BACKENDS.md
- 9Router: https://github.com/decolua/9router/blob/master/README.md
- Client documentation: https://code.claude.com/docs/en/llm-gateway ; https://kiro.dev/docs/cli/acp/
