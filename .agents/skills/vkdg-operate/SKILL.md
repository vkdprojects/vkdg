---
name: vkdg-operate
description: "Modify or review the VKDG critical path and operation: concurrency, backpressure, OAuth, quota, routing, storage, jobs, diagnostics, security, benchmarks, and failures. Use for performance, reliability, or observability changes; combine with vkdg-change and vkdg-protocol as needed."
---

# Operating the core with explicit limits

## Before implementing

Map the request/job states and identify the owner of each resource: bytes in buffer, concurrency slot, upstream connection, credential lease, persisted job, and observability event. Define deadline and policy when each is exhausted. Write a failure test that verifies release and diagnostics; only then optimize.

## Data path invariants

- Queue, body, frame, event, channel, pool, and task all need limits. Admission happens before expensive work; lease is released exactly once on success, error, cancellation, timeout, and isolated panic.
- Stream propagates cancellation and backpressure in both directions. Do not collect the entire SSE body; no log exporter accidentally blocks sending. Error after commit does not restart on another provider.
- Connection selection/reservation is atomic; filter by operation/capability, tenant, quota, cooldown, model, health, and session. Provider circuit, account cooldown, and model lockout are independent states. Decision explains candidate exclusions.
- OAuth refresh singleflight per connection, conditional write per generation, and encrypted vault. Client authenticates with VKDG key, does not receive upstream token. In multiple replicas, reserving and rotating a token requires a fencing experiment before promising distributed safety.
- Configuration and plugins enter via validated immutable snapshot; existing streams retain their version until they finish or expire under explicit policy.

## Persistence and debug

- Separate configuration/vault, transient state, operational history, and jobs/artifacts. SQLite WAL serves a single instance with a controlled writer; do not write per token. An accepted job requires persistence and reconciliation after restart. Artifact has ACL, limits, TTL, and reference; no blob in logs.
- `DecisionRecord` records phase, config version, route, excluded candidates, chosen account, attempts, costs/usage with provenance, and safe error; content and secrets absent by default. Metrics avoid labels per request ID/user; trace uses stable request ID.
- Payload capture is opt-in, bounded, redacted, and expires. Reproducible fixtures use a fake upstream and sanitized data.

## Measure before asserting

Establish workloads of idle streams, active streams, slow clients, passthrough, translation, and plugin; report hardware, version, concurrency, peak RAM, CPU, p50/p95/p99, and additional TTFT. Profile identifies bottleneck and comparison uses an equivalent scenario. Start with Hyper/Tower and Axum routes where useful; only spike Pingora if the transport appears as a material cost in the profile. Do not present synthetic benchmarks as guaranteed production capacity.

## Minimum failure cases

Test 429 on one account while another serves, concurrent and rotating refresh, failing provider, stale quota, cancellation at each transition, slow downstream, full storage, job restart, configuration swapped under stream, and stalled plugin. Do not multiply equivalent tests; protect invariants with focal tests and deterministic integration. Report chosen degradation and what was not validated on a real provider.
