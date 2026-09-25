# Designing Tests that Find Defects in VKDG

Use this guide when starting a behavioral change or reviewing tests. Write a small matrix **before** the code and adjust as you discover new risks. Start from the contract observed by the client or operator, including persistent effects, limits, and failures. Read the current tests, neighboring adapters, protocol fixtures, related issues/bugs, and the actual execution path. Do not derive truth from the code being changed.

## 1. Invariant inventory and failure hypotheses

For each decision crossed by the change, fill in mentally or in the PR:

| Item | Question | VKDG Example |
| --- | --- | --- |
| Invariant | What must remain true? | A 429 from one credential must not block another credential for the same provider. |
| State/boundary | What input and state trigger the decision? | Two Claude connections, one in cooldown and one eligible. |
| Plausible defect | What code error would violate the contract? | Breaker indexed by provider instead of connection. |
| Oracle | Where does the independent expected value come from? | Approved eligibility rule + fixed responses from both upstreams. |
| Observation | Which assert detects the defect? | Second connection selected, response from it received, first not reused. |
| Level | Where is the failure visible without simulating the logic? | Router test with injected store/clock and fake upstream. |

Investigate applicable dimensions: missing/empty/invalid/unknown value; `0`, `1`, max and max+1; order and duplication; state before/after commit; success/failure/timeout/cancellation/retry; concurrency between actors; restart and partial write; capability incompatibility; tenant permission boundaries; memory/queue/connection saturation. Pair dimensions when the interaction changes a decision: timeout **after** the first byte; concurrent refresh **with** a rotating token; retry **with** a repeated idempotency key. Avoid the Cartesian product without a failure hypothesis.

Track high-impact invariants (isolation, billing, secrets, stream commit, state integrity) to executable asserts, including negative paths and recovery. For lower risk, choose justified representatives. If a dimension is uncovered, note the impact and reason; do not claim complete coverage. One test per method or per line is a weak quality signal.

## 2. Oracles and test doubles

- The expected value comes from an approved spec/contract, a verifiable external example, a manually audited fixture, or an independent property. For conversions, compare fields and events that matter semantically, including the absence of a dangerous field/effect; do not generate the expected by round-trip through two paths that share the same bug.
- A fake represents the external boundary with fixed data and observable behavior. If the fake only returns what the test instructed and the only assert is that it was called, the test does not prove the transformation or the result to the client. Use a spy only to assert **absence** or destination of a call when that is part of the contract.
- Prefer a real temporary store for transactions, persistence, and restart; a fake store does not prove durability. Inject clock, transport, and controlled failures. A parser property must not use the same parser to validate its own output.
- Asserts must distinguish rival implementations. If removing the branch, swapping `>=` for `>`, changing the isolation key, discarding a delta, or omitting persistence leaves the test green, rewrite the assert or remove the redundant test.
- Broad snapshots are appropriate only when every fixture change will be audited and relevant failures are readable; select critical fields explicitly. Do not update snapshots automatically to make the suite pass.

## 3. Example scenarios

### Per-credential cooldown

Contract: a connection that receives a 429 becomes unavailable; another healthy connection on the same provider can serve a new call. Prepare two fictitious credentials with distinct identities and a fixed clock. Make the first receive a 429, then send another request. Expect the second to be selected, its response received, and the decision/cooldown persisted on the first. Defect detected: cooldown indexed by `provider_id`. A test that only verifies that `record_429` was called does not detect this defect.

### SSE after commit

Contract: before the first byte it is still possible to switch candidates according to policy; after sending content to the client, a stream from another upstream must not be mixed in. Have A send a delta, then fail; leave B ready to respond. Read the client's events and confirm there is only the delta from A, a termination/error allowed by the contract, and no call to B. Repeat with failure before the first delta and confirm the fallback when permitted. Defects detected: post-commit fallback, silent error, and concatenation of two providers.

### Concurrent OAuth refresh

Contract: multiple calls for the same expired credential share one refresh; an older completion must not overwrite a newer-generation token. Use barriers and a controlled refresh, without `sleep`. Release N calls simultaneously, check the number of refreshes **if** singleflight is the contract, and observe responses using the valid generation. Reverse the completion order of two generations and verify the stored token. Defects detected: duplicate refresh, lost update, and revived revoked token. Counting calls in isolation does not prove that requests received the correct credential.

### Isolation and storage

Contract: a media job created by tenant A is not visible to B and survives a restart in the documented state. Use a real temporary store, create a job, restart the instance, query as both A and B, verify allowed fields and that the artifact is inaccessible to B. Inject a persistence failure to confirm a typed error and the absence of a partially published job. An in-memory fake without restart does not prove this contract.

## 4. Techniques proportional to risk

| Technique | When it adds information | Pitfall |
| --- | --- | --- |
| Example table | Small boundaries and clear partitions | Hundreds of rows with the same weak assert |
| Property test | Invariant valid for many values, generator that reaches interesting cases | `parse(serialize(x)) == x` with a shared bug, or `no panic` only for semantics |
| State machine | Sequence of reserve, release, refresh, job, and replay matters | Model copied from the implementation |
| Fuzzing | Arbitrary bytes, UTF-8, SSE, JSON, and hostile frames | Corpus of only invalid input that never reaches relevant logic |
| Mutation | Stable suite exists and weak asserts are a risk | Raw score used as a goal; equivalent mutant is not a defect |
| Integration | Contract between modules or persistence is the risk | Mocking all involved modules |
| Benchmark | Latency, memory, or fairness regression | Treating an isolated benchmark as a deterministic correctness test |

Do RED focused on the chosen defect, GREEN with the smallest implementation, then expand only for other distinct defects in the matrix. After each bug found in fuzz/property/mutation, minimize the input and add a deterministic regression. Record seed/corpus when relevant. Coverage tools help find unexercised paths; mutation testing helps evaluate whether asserts catch changes, at a higher cost. Neither replaces contract judgment.

## 5. AI test review checklist

1. What specific plausible defect makes this test red? Did the RED fail on the expected behavior?
2. Is the expected value independent of the logic under test and of a mock that repeats that logic?
3. Does the assert see the effect required by the client/operator, including absent effect, state, and error?
4. What boundary of value, time, order, failure, retry, isolation, or concurrency matters here? Which interaction changes the decision?
5. Is the test deterministic, does it use synthetic data, and does it isolate resources? Is its result reproducible?
6. Does this test add detection power to the existing ones? Does a representative mutation defeat it?
7. Does the chosen level cover the real risk without duplicating the entire suite? Are material gaps documented?

If there is no convincing answer to 1–3, redesign or remove the cosmetic test. If 4–7 revealed new material risk, write a distinct scenario before implementing the fix. Do not add tests just to satisfy a count.

## References

- [Superpowers: test-driven-development](https://github.com/obra/superpowers/blob/main/skills/test-driven-development/SKILL.md) and [writing-good-tests](https://github.com/obra/superpowers/blob/main/skills/test-driven-development/writing-good-tests.md): RED/GREEN/REFACTOR cycle and the question of which bug the test finds.
- [The Practical Test Pyramid](https://martinfowler.com/articles/practical-test-pyramid.html): choosing the right level and avoiding fragile duplication.
- [cargo-mutants: mutation vs coverage](https://mutants.rs/vs-coverage.html) and [using results](https://mutants.rs/using-results.html): surviving mutants and the limits of line coverage.
- [Proptest: state machines](https://proptest-rs.github.io/proptest/proptest/state-machine.html) and [Rust Fuzz Book](https://rust-fuzz.github.io/book/cargo-fuzz.html): transition sequences, generation, and corpus.
