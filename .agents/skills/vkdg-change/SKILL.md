---
name: vkdg-change
description: "Implement, fix, or review VKDG code in Rust with high codebase quality. Use for common changes, module/crate organization, clean code, tests, DX, and PR review; combine with the specific protocol, plugin, or operation skills when the task touches those areas."
---

# Changing VKDG code

## Work process

1. Read `AGENTS.md`, the owning module, and related contracts/ADRs. Write in one sentence what external behavior changes and which invariants remain.
2. Search for existing implementation and tests (`rg` before creating). Choose the smallest complete vertical change and review its effects on other APIs.
3. Write the first failing test/fixture for the expected reason for new behavior or bugs. For refactors, protect the existing contract before the change. Do not create tests for trivial files with no behavior.
4. Implement, run the focal test, fix, then run available relevant gates. Document contract/API if it changed. Review the entire diff for duplication, secret in log, semantic regression, and accidental resource changes.
5. Report behavior, executed proofs, and remaining risk. Never declare "tested" if the command did not run.

## Readable code with clear ownership

- Name modules by domain and action they own (`account_selection`, `sse_decoder`, `job_state`), not by generic groupings (`helpers`, `utils`, `common`, `misc`, `manager`). A helper function should live next to its owner; extract for sharing only when there are at least two real uses with identical semantics.
- Keep `lib.rs`/`mod.rs` as a map of modules and deliberate re-exports. Avoid giant provider switch statements, functions that authenticate + route + serialize + make HTTP + persist, universal traits, monolithic enums of all errors, and a central module depended on by everything.
- Split when you find independent reasons for change, a trust boundary, different invariants, or a test that requires spinning up half the system. Do not split just to reduce line count, nor multiply crates per imagined concept. If a file requires navigating many states to understand a single operation, extract the nameable responsibility and move its tests alongside.
- Maintain explicit flow: typed inputs, validation at the edge, named transformation, typed output/error. Use newtypes for IDs and states that must not be swapped; make invalid state hard to represent without adding unnecessary bureaucracy.
- Limit visibility (`pub(crate)`/private by default); small public API with rustdoc covering contracts, errors, cancellation, and usage examples where relevant. Forbid `unwrap`/`expect` on paths with external input; in tests or local invariants, explain if not obvious.
- Avoid globally mutable helpers, macros that hide control flow, cloning large payloads for convenience, and blocking calls inside async workers. Prefer clear composition over speculative abstractions. Do not use `Arc<Mutex<...>>` as an automatic solution: define the state scope and contention.

## DX for humans and agents

- Each existing crate should allow discovery within a few minutes: responsibility, public entry point, dependencies, invariants, focal test, and fixture. Update the crate's short README when the map changes, without duplicating the entire architecture.
- Error messages should point to resource/field, phase, safe reason, and next action. Correlation IDs and stable structured codes help reproduction; never display secrets or payload by default.
- Keep configuration declarative with validation before activation; minimal runnable examples; local commands without real keys using a fake upstream. Where a command does not yet exist, record the gap rather than inventing an instruction.
- When proposing a new dependency, evaluate maintenance, security, async compatibility, compile cost, and whether a library already present solves it. Record large decisions in a short ADR: context, options, choice, consequences, and reversible experiment.
- Deliver small, reviewable changes with related tests in the same unit; do not leave an API without a user/fixture. Review verifies clarity for someone arriving without context, beyond functional correctness.

## Exit criteria

Contract and owner clear; observable test failed before and passes after; error and cancellation cases covered where applicable; no pretend compatibility; reviewable diff; operational documentation updated; cost on the critical path measured if changed. Prefer simple code that explains the domain over an abstraction that merely shortens lines.
