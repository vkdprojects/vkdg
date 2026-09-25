---
name: vkdg-test-first
description: "Drive any VKDG behavior change with verifiable test-first/TDD. Use before implementing or fixing Rust code, API, protocol, provider, plugin, OAuth, storage, concurrency, or performance; define scenarios and edge cases, observe RED for the correct reason, produce GREEN, and refactor without weakening the contract."
---

# Test first as an engineering rule

Read `AGENTS.md`, the domain skill for the task, and [the investigation guide](references/test-design.md) before designing scenarios. This cycle is mandatory for behavior changes; it is not a stamp given to a test written after the code.

## Quality criterion: detecting real defects

Before writing each test, record in one sentence **which plausible wrong implementation it refutes**. The expectation must come from the contract, a reviewed fixture, or an independent calculation; never calculate the expected value by calling the converter itself. Assert the relevant observable effect (body, status, event, persisted state, released reservation, absence of improper send), not just `is_ok`, `is_some`, a mock call, or a line count.

Investigate risks before RED: normal flows, limits, hostile inputs, errors, cancellation, recovery, races, and tenant/credential isolation. For each material invariant touched, link at least one test that would fail if it were violated; cover combinations that change the decision. Record why a dimension does not apply. "Fully covered" means the known, relevant behaviors and risks for the change, with explicit gaps; not every mathematically possible combination.

A test that already passes without the logic, mirrors the implementation, only verifies a mock programmed to respond, or repeats the same risk without adding detection power is excluded. One test can protect multiple invariants; multiple tests may be needed for a dangerous boundary. Count is not a goal.

## RED → GREEN → REFACTOR

1. **Contract:** state input, output/error, initial state, transition, limit, and observable effect. Read current implementation, callers, external contracts, and nearby incidents/tests. Formulate the plausible defect and choose the smallest scenario that distinguishes it. Consult existing fixtures/protocols before adding a new one.
2. **RED:** write the test/fixture before the implementation and run the focal test. Confirm it fails on the assert of the intended behavior or due to explicitly absent functionality. A syntax error, accidentally missing import, disabled infrastructure, or flaky test does not prove RED. An API not yet created may initially cause a compile failure; after declaring the minimal interface, require behavioral failure before concluding the cycle.
3. **GREEN:** implement only the necessary slice in the owning module. Run the focal test until it passes. Do not relax the assert, omit a case, swap the test for a mock that replicates the implementation, or mark `ignored` to get green.
4. **REFACTOR:** improve names, modules, and duplication while keeping all tests green. Run tests at the reached boundary and available gates. Record the reason for any removed test; move the test alongside the contract, do not delete it because it makes implementation harder.
5. **Evidence:** in the change/PR, record risk → scenario → decisive assert, focal command, relevant RED failure (summary), GREEN command, and broader tests run. If the environment prevents execution, state the exact blocker and keep the test; do not invent a result.

Do not commit only a broken test on the main branch. The RED record can be in the change comment or a sanitized local log; a separate RED commit is not required. A new test that already passes on the old version can be useful for characterization, but does not demonstrate new behavior.

## Choosing tests by boundary

| Risk | Initial test | Cases that are often missing |
| --- | --- | --- |
| Pure rule or state machine | Unit/table in the module | invalid transition, limit, idempotency, typed error |
| API/adapter | External contract with fake upstream | headers, status, extra fields, required capability, upstream error |
| SSE/parser | Fixture + property test and regression corpus | byte cut at any position, partial UTF-8, empty/large event, EOF |
| Concurrency/OAuth | Deterministic integration with barrier/fake clock | N simultaneous refreshes, cancellation between selection and reservation, rotating token |
| Storage/job | Integration with real temporary store + injected failures | restart, partial write, repeated idempotency key, full volume |
| Security/isolation | Negative authorization test | cross-tenant, secret in log, private URL, plugin without permission |
| Performance | Benchmark with baseline and profile | slow client, idle streams, p95/p99, peak RAM, hook cost |

Do not turn the table into a mechanical test quota. For each change, select the main scenario, adversarial case, and material failure transition. Combine cases in tabular tests when the semantics are the same. Prefer the lowest level that captures the contract without mocking the tested logic; add integration where the boundary between modules is the risk.

## VKDG edge matrix

When touching an area, traverse the relevant dimensions and write at least one scenario that refutes a naive implementation:

- **Protocol:** required field absent, unexpected value, valid but incompatible format, tool/reasoning delta, unknown usage, vendor extension.
- **Streaming:** arbitrary fragmentation, disconnected client, slow upstream, backpressure, error before/after commit, end without expected marker.
- **Routing:** candidate excluded by capability, stale quota, session pinned to ineligible connection, one account 429 and another healthy, no candidate.
- **Credential:** concurrent refresh, rotating token, revoked secret, delayed update must not overwrite a newer generation.
- **Jobs/media:** cancelled upload, repeated/out-of-order callback, restart with pending job, expired artifact, access by another tenant.
- **Plugin:** trap, timeout, invalid output, incompatible ABI, install/rollback under traffic, hook unsafe for streaming.
- **Resources:** body/frame/queue exceeded, lease release on every exit, telemetry saturation, and full storage.

## Determinism and cost

- Common suite runs without external credentials: fake upstream in-process, injectable clock, barriers instead of `sleep`, ephemeral ports, synthetic data, reproducible seed. Tests with a real provider are separate, identified smoke tests.
- Property testing is useful when there is an independent invariant and a generator that reaches relevant inputs; save the minimized counterexample for regression. Fuzzing byte parsers and decoders requires an oracle (no panic, limits, no improper loss, or independent comparison), a corpus, and triage; long fuzz runs run outside the focal cycle.
- For critical logic, manually check a representative mutation (swap a limit, remove a check, invert a condition) and run the focal test to prove it fails. When the suite is stable, selectively use `cargo mutants` to find surviving mutations; review equivalents, do not chase percentages blindly. Line coverage guides paths, not assertion quality.
- Snapshot only with human review of the diff; do not accept automatic updates as a fix. Coverage measures exercised surface but does not replace assertions that protect semantics.
- Refactor without observable change can start from existing characterization tests; documentation and purely mechanical changes do not need invented tests. If a new file contains behavior, the contract it implements needs a test prior to implementation.

## Mandatory review

Reject behavior changes without invariant tracing and RED/GREEN evidence, tests without a plausible defect, tests that pass with the implementation removed, tests that reproduce the function itself via mock, post-commit fallback, fixtures with secrets, or `sleep` to mask a race. An urgent hotfix still starts with a focal reproduction; if execution is not possible, state the exception in the review and open the test as the first verifiable next action.
