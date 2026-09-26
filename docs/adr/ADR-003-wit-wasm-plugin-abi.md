# ADR-003: WIT/WASM as the VKDG plugin ABI

**Status:** Accepted
**Date:** 2026-09-26

## Context

VKDG needs an extension point for routing strategies, compression engines,
provider adapters, cache backends, and auth/rate-limit logic. Three options
were evaluated:

1. Native Rust traits loaded as .so/.dylib
2. gRPC/HTTP sidecar processes
3. WASM Component Model via Wasmtime + WIT

## Decision

Adopt WASM Component Model (Wasmtime 27, WIT interfaces in wit/) as the
primary plugin ABI for all five plugin roles.

## Reasons

1. **Sandbox by construction.** A WASM component has no ambient file system,
   network, or process access. The March 2026 LiteLLM supply chain attack
   (CVE-2026-59822) stole SSH keys from a pip-installed dependency. That attack
   is structurally impossible in a WASM sandbox -- the plugin cannot make
   outbound connections without an explicit host function grant.

2. **Cross-language.** Any language that compiles to WASM (Rust, Go via TinyGo,
   Python via componentize-py, C/C++, Zig) can implement a plugin. Plugin
   authors are not locked into Rust.

3. **Stable ABI.** WIT interfaces are versioned (vkdg:plugin@0.1.0). A plugin
   compiled against @0.1.0 loads on any gateway version that supports @0.1.x.
   Dynamic linking breaks across rustc versions; HTTP sidecars add per-request
   network latency (unacceptable for cache lookup and routing decisions).

4. **In-process performance.** A WASM function call round-trip is 1-50us.
   A gRPC round-trip to a sidecar is 0.5-2ms. For a cache lookup that runs on
   every request, the difference is measurable.

## Rejected alternatives

**Dynamic linking (.so/.dylib):** No sandbox, ABI breaks across rustc versions,
cannot load from untrusted sources without compromising host process security.

**gRPC/HTTP sidecar:** Acceptable for slow operations (video generation,
OAuth flows). Not acceptable for the hot path (cache, routing, compression).
We use sidecar processes for external services (ExternalServiceClient), but
not for pipeline-critical plugins.

## Consequences

- Wasmtime 27 is a heavyweight dependency (~20MB compile-time, ~5MB binary).
  It is already in the workspace from Phase D. No additional dep cost.
- WIT bindgen codegen (`wasmtime::component::bindgen!`) is deferred to Phase E.
  Phase D validates the component binary at install time but stubs the call surface.
- The WIT interfaces in wit/ are the stable contract. Changes to them follow
  semver: wit/types.wit, wit/router.wit, wit/compressor.wit, wit/provider.wit,
  wit/cache.wit, wit/auth.wit must be versioned independently.
- Native Rust implementations (AnthropicAdapter, OpenAIAdapter, SqliteExactCache)
  implement the same logical interface as WASM plugins but bypass the WASM
  sandbox. This is acceptable for official first-party plugins shipped with the
  gateway binary.
