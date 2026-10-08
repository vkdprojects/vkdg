# VKDG Roadmap

VKDG is an AI gateway written in Rust. It started as a personal tool for routing requests across providers and is being built into a platform that anyone can self-host — from a single developer to a company running thousands of API keys.

This document is the honest state of where we are and where we're going.

---

## Phase 1 — Personal & Internal Team `active`

The goal of Phase 1 is a single binary that replaces every ad-hoc proxy script on a developer's machine. No Node.js, no Docker, no runtime dependencies. One `curl | sh`, and you have a working AI gateway.

### What works today

**Gateway**
- Single Rust binary — zero runtime dependencies
- OpenAI-compatible endpoint on port `8080` (`/v1/chat/completions`, `/v1/messages`)
- Anthropic-compatible endpoint on port `8080` (`/v1/messages`)
- Bootstrap token auth → HttpOnly session cookies (secure by default)
- Admin API bound to `localhost` by default — not exposed to the network unless you opt in

**Admin console**
- Embedded SPA served from the same binary on port `9090`
- No separate web server or static file hosting required
- Built with SvelteKit + adapter-static, compiled into the binary at build time

**Provider adapters (14 total)**

| Provider | Notes |
|---|---|
| Anthropic | Direct API |
| OpenAI | Direct API |
| Groq | OpenAI-compatible |
| Gemini | Native + OpenAI-compatible |
| DeepSeek | OpenAI-compatible |
| Mistral | OpenAI-compatible |
| Together AI | OpenAI-compatible |
| Fireworks AI | OpenAI-compatible |
| Claude Code | OAuth token refresh |
| Codex (OpenAI) | OAuth token refresh |
| Kiro / Amazon Q | OAuth token refresh |
| Kimi | OpenAI-compatible |
| Antigravity | OAuth token refresh |
| GitHub Copilot | OAuth token refresh |

OAuth-backed code agent providers handle token refresh automatically — you authenticate once and VKDG keeps the session alive.

**Routing strategies**
- `round-robin` — distribute across connections evenly
- `weighted` — send more traffic to preferred connections
- `fallback-chain` — try each connection in order until one succeeds
- `lowest-latency` — route to the connection with the best recent P50
- `P2C` — power-of-two-choices load balancing
- `last-known-good` — stick to the last connection that returned a clean response
- `fusion` — fan out to multiple connections and merge results
- `prompt-chain` — structured multi-step chaining
- `auto` — VKDG picks the strategy based on observed conditions

**Context compression**
- RTK compression: semantic reduction of prompt context, saves 15–95% tokens depending on content
- Caveman compression: aggressive token reduction for high-volume or cost-sensitive paths
- FilterPack: pluggable compression pipeline — swap or stack filters without changing gateway code

**Reliability**
- Full streaming support with mid-stream failure detection
- Streaming error surfaced to the client immediately; no silent truncation

**Extensibility**
- Provider plugin system via WIT/WASM — implement the `provider` interface in any language that compiles to WASM, drop the `.wasm` file in `plugins/`, done

### Target users

Individual developers, small engineering teams, and companies building internal AI infrastructure who want control over routing, costs, and provider dependencies without running a managed service.

---

## Phase 2 — Team & Organization `next`

Phase 2 makes VKDG safe to hand to a team. The admin interface needs real access control, and the gateway needs the operational primitives (TLS, audit logs, observability) that a company's security team will ask for.

**Access control**
- Multiple admin users with role-based access control: `admin`, `operator`, `viewer`
- Per-user API key management — revoke one user's access without rotating team credentials

**Security & networking**
- TLS-first: `vkdg serve --tls` with manual certs or ACME auto-cert (Let's Encrypt)
- `vkdg serve --expose-admin` flag to deliberately open the admin API to a network interface — explicit opt-in, not a config file gotcha

**Observability**
- Audit log: every admin action timestamped and attributed to a user — who changed what, when
- Per-API-key usage analytics: request counts, token totals, spend estimates
- Connection health monitoring with real circuit breaker state visible in the console

**Persistence**
- Config persistence: migrate from YAML file → SQLite database when you need state that survives restarts
- Per-key spend tracking requires persistence; Phase 1 is in-memory only

---

## Phase 3 — Platform `roadmap`

Phase 3 is the OpenRouter mode. VKDG stops being just a proxy you run for yourself and becomes a platform you can operate for others — your company, your customers, or the public.

The key architectural move here is splitting into two binaries:

```
vkdg          — the gateway; handles AI traffic; admin API stays on localhost forever
vkdg-console  — BFF + user management; the only process that talks to the admin API
```

The admin API never becomes internet-facing. `vkdg-console` sits in front of it, handles authentication, and exposes only the operations users are allowed to perform. This boundary is permanent by design.

**`vkdg-console` (second binary)**
- Full BFF pattern: browser → `vkdg-console` → `vkdg` admin API
- User registration, email invites, team management
- OAuth 2.0 login: GitHub, Google, corporate SSO
- Built in Rust — no Node.js BFF, no separate runtime

**Virtual key system**
- Free / pro / enterprise tier plans with quota enforcement
- Per-key rate limiting, spend caps, model allow-lists
- Programmatic key management via public API

**Usage dashboard**
- Tokens consumed, cost estimates, latency percentiles — per key, per team, per model
- Webhook delivery for billing events: quota reached, key expired, spend cap hit

---

## Phase 4 — Ecosystem `roadmap`

Phase 4 is what happens when the community builds on top of VKDG.

**Plugin marketplace**
- Discover, install, and pin WASM provider plugins from a registry
- Version-pinned installs: no surprise breakage when the registry updates
- One `plugins/providers/myprovider/` PR = your provider ships to every VKDG user

**Community contributions**
- Community provider catalog: first-party and community providers in the same registry
- Compression pack registry: share RTK filter configurations as reusable packs

**Hosted option**
- `vkdg.dev`: cloud-hosted console — the same `vkdg-console` binary running as a service for teams that don't want to self-host

**Kubernetes**
- `VkdgGateway` CRD: declare your gateway configuration as a Kubernetes resource
- Operator handles lifecycle, scaling, and cert rotation

---

## Architecture philosophy

These are the decisions we made and why. They're not going to change.

### Everything is a plugin

Providers, compressors, routers, and auth strategies are all interfaces with implementations. The core gateway knows almost nothing about Anthropic or OpenAI specifically — it knows about the `ProviderAdapter` trait. This means you can add a provider without touching gateway internals, and community providers are structurally identical to first-party ones.

### WIT/WASM for cross-language plugins

The provider plugin interface is defined in WIT (WebAssembly Interface Types). You can implement it in Rust, Go, C, Zig, or any language with a WASM target. Drop the `.wasm` file in `plugins/` and VKDG loads it at startup. No FFI, no native modules, no version pinning nightmares.

### Single binary by default

The default deployment is one `curl | sh`. No Docker, no Node.js, no systemd unit file with six environment variables. The admin console SPA is compiled into the binary with `adapter-static`. This is not a temporary shortcut — it's the intended experience for Phase 1 and Phase 2 deployments.

### Two-binary for platform mode

When you're running a platform for other people, the admin API must not be internet-facing. Ever. The two-binary split in Phase 3 makes this a structural guarantee, not a configuration discipline. `vkdg` never opens its admin port to the world. `vkdg-console` is the only process that can reach it, and it enforces its own auth layer on top.

### No database required to start

Phase 1 runs entirely from a YAML config file and in-memory state. You don't need to provision Postgres to try VKDG. When you need persistence — per-key spend tracking, audit logs, multi-user state — SQLite is the first step. Postgres is available for scale, but it's not required.

### Wire-compatible with OpenAI and Anthropic

VKDG is a drop-in replacement for both. Point your existing `OPENAI_BASE_URL` or `ANTHROPIC_BASE_URL` at VKDG and your clients work without changes. We maintain both wire formats and will not break them.

---

## Contributing

The fastest way to contribute today is a new provider. If it speaks OpenAI-compatible wire format, it's nine lines of Rust using `OpenAiCompatAdapter`. If it needs custom logic, implement `ProviderAdapter`. See `plugins/providers/` for examples and `docs/sdk/adding-a-provider.md` for the full guide.

Compression filter packs and routing strategies are the next best contributions. Open an issue before starting anything large — architecture decisions here have downstream consequences.
