---
name: vkdg-plugin
description: Design or implement VKDG extensions, provider plugins, ingress plugins, policy hooks, observers, ABI/WIT, SDK, and installation lifecycle. Use when a task asks for a new plugin type, hook, template, sandbox, compatibility, or conformance test; combine with vkdg-protocol for converters.
---

# Evolving VKDG plugins

## Contract before runtime

1. Declare category (`ingress`, `provider`, `policy`, `observer`, or `artifact-store`), operation, hook, and typed input/output. Show a minimal author example and a case that must fail.
2. Define a versioned manifest: name, version, ABI/WIT, capabilities, operations, config schema, granted resources, failure policy, and compatibility. Design upgrade/rollback before enabling installation.
3. Specify CPU/time, memory, instance, payload, concurrency, network, file, and secret limits. Core retains authority over identity, admission, HTTP, credentials, state, and decision.

## Ergonomics and quality

- A provider plugin declares models/capabilities and implements conversion; it does not receive a raw bearer token when an opaque credential reference or HTTP host resolves. The SDK offers fixtures for full response, error, fragmented streaming, cancellation, and capability mismatch.
- Offer `init`, local test without network, manifest verification, packaging, permissions inspection, enable/disable, and rollback with messages that point to the fixture and the hook. Generate schema documentation. Do not announce language support without compiling and running a real template in it.
- WIT/Component Model is a candidate for isolated ABI, with negotiated versions and small host functions. Rust built-ins can implement the same logical contract. An external process is an option for slow operations or languages that do not support the ABI adequately. Never load third-party Rust `.so` as a stable ABI.
- In streaming, incrementally safe hooks work with events and backpressure; a hook that needs the entire response is incompatible with that streaming route. Do not call an external process per token. For media, pass a handle/stream with limits; do not copy the entire video between process/host.
- Register active plugins by snapshot; an in-flight request retains its used version. Deactivation/upgrade does not swap the implementation under an in-progress stream. Durable plugin state requires explicit migration or declared absence.

## Isolation and DX testing

Test invalid manifest, incompatible ABI, incomplete configuration, trap, timeout, malformed output, forbidden resource access, duplicate plugin, wrong tenant, slow stream, and rollback during traffic. Fail-closed for auth, mandatory policy, and essential transformation; fail-open only for explicitly configured optional enrichment, with a visible event. Use shared conformance with official adapters, ensuring that "ease" does not create a second class of correctness.

## Design review

Ask whether the hook needs to exist or whether declarative configuration suffices; whether it should operate before selection, before dispatch, on each event, or after finalization; whether the cost has been measured; whether failure preserves client semantics. The plugin proposal must allow implementing a new provider without editing core files. Hook count and marketplace do not demonstrate quality by themselves.
