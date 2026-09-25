# ADR-001: Hyper/Tower over Pingora

**Status:** Accepted  
**Date:** 2026-09-25

## Context

We needed an HTTP server for the VKDG gateway. The two serious candidates were:

- **Hyper/Tower + Axum** — the most widely adopted Rust HTTP stack; Tower provides composable middleware; Axum adds ergonomic routing on top of Hyper.
- **Pingora** — Cloudflare's HTTP framework, built on `io_uring`/`epoll`; high performance in Linux production.

## Decision

Adopt **Hyper 1.x + Tower + Axum** as the gateway's HTTP stack.

## Reasons

1. **Cross-platform development**: Pingora uses `io_uring`/`epoll`, available only on Linux. The team develops on macOS; a stack that requires Linux from day 0 biases local tests and CI.

2. **Testing ecosystem**: `tower-test`, `axum::test`, and `hyper::server::conn` allow in-process unit and integration tests without spinning up real sockets. The `tests/conformance` suite depends on this.

3. **Shared Tokio runtime**: Axum and Hyper run on the same Tokio runtime as the rest of the workspace. Pingora has its own runtime — integrating the two would require executor bridges.

4. **Maintenance curve**: Axum and Tower have extensive documentation, numerous examples, and an active community. Pingora requires familiarity with its phase model and C bindings.

## Rejected alternative

**Pingora** remains open as a future spike. When real production profiles indicate transport as the bottleneck, the spike should compare both stacks in the same load scenario (concurrent streams, cancellation, translation). The current decision does not hinder that experiment — the protocol adapters and pipeline logic are HTTP-transport-agnostic.

## Consequences

- Build times: acceptable (Tokio + Axum in the workspace Cargo.toml).
- Behavior in Linux production: to be measured when real load is available.
- Pingora spike: record in Phase E backlog before any performance target.
