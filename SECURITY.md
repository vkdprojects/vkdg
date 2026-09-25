# Security Policy

## Supported versions

| Version | Supported |
| ------- | --------- |
| latest (main) | yes |
| pre-1.0 releases | no |

## Reporting a vulnerability

Use GitHub's private vulnerability reporting:
[Report a vulnerability](https://github.com/vkdprojects/vkdg/security/advisories/new)

Do not file a public issue. Include the affected crate and version, steps to
reproduce, and your assessment of impact. You will get a response within 72 hours.
Critical issues get a patch within 7 days of confirmation.

## Attack surface

VKDG sits between clients and upstream AI providers, which shapes what matters most:

**Credential handling.** `CredentialManager` holds provider tokens in memory.
`TokenState::Debug` omits the token value from `DecisionRecord`, but anything that
causes a token to appear in logs or error responses is a serious finding.

**Tenant isolation.** `InMemoryArtifactStore` enforces per-tenant ACL on every
artifact read. A path that bypasses the ACL check, or lets one tenant infer another
tenant's artifact IDs, is in scope.

**Stream commit semantics.** Once a streaming response is marked committed, the
pipeline does not retry. A bug that allows retrying a committed stream could cause
duplicate charges or data corruption at the application layer.

**Plugin sandbox.** WASM plugins run in a Wasmtime sandbox. Sandbox escapes,
capability leaks through the host ABI, or denial-of-service via the plugin compute
budget are all in scope.

Out of scope: vulnerabilities in upstream dependencies (report to the dependency
maintainer directly), theoretical weaknesses without a demonstrated exploit path.

## Disclosure

We follow coordinated disclosure and will credit researchers in release notes unless
they prefer to stay anonymous.
