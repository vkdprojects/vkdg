# Providers

VKDG is a gateway, not a provider directory. It doesn't maintain a catalog of every AI API on the internet — that's not the point. The point is that you can connect **any** upstream you have access to, and route traffic across them intelligently.

That said, VKDG ships with a set of official provider adapters for the most common APIs. These are compiled into the binary, maintained by the VKDG team, and get updates when provider APIs change.

Everything else — including thousands of OpenAI-compatible endpoints — works via config alone, no code needed. And if you need a custom integration, the [plugin system](./registry.md) covers that too.

---

## How providers work

When a request arrives at VKDG, the router selects a connection. The connection references a provider. The provider's job is exactly one thing: translate VKDG's internal request format into a real HTTP call and parse the response back.

The gateway owns the HTTP client, TLS, retries, auth injection, and backpressure. The provider only transforms bytes. This separation means you can add a new provider without touching the core at all.

```
Request → Router → Connection → Provider adapter → HTTP → Upstream API
                                     ↑
            translates: model name, messages, tools, streaming
```

---

## Official providers

These ship with every VKDG binary. We maintain the adapters — when an API changes, we update the adapter and release a new version.

**Why "official" matters:** these adapters handle edge cases that a simple OpenAI proxy misses — Anthropic's tool call format, streaming error injection, OAuth token refresh for code agents, per-provider capability detection.

### API key

| Provider | ID | Free tier | Notes |
|----------|----|-----------|-------|
| Anthropic | `anthropic` | — | Native Messages format. Claude family. |
| OpenAI | `openai` | — | Chat Completions + Images + o-series |
| Google Gemini | `gemini` | ✓ limited | OpenAI-compat endpoint |
| Groq | `groq` | ✓ generous | Very fast inference. Llama, Mixtral, Gemma. |
| DeepSeek | `deepseek` | ✓ limited | Very cheap. Strong at code. |
| Mistral | `mistral` | — | OpenAI-compat. Good European alternative. |
| Together AI | `together` | — | Large model catalog. OpenAI-compat. |
| Fireworks AI | `fireworks` | — | Fast inference. OpenAI-compat. |

```yaml
# Example: API key connection
connections:
  - id: groq-main
    provider: groq
    auth:
      type: api_key
      env_var: GROQ_API_KEY
    models: ["llama-3.3-70b-versatile", "mixtral-8x7b-32768"]
    max_concurrent: 30
```

### OAuth / device code (code agents)

These are AI coding tools that don't use API keys — they use OAuth. You authenticate once and VKDG stores and refreshes the token automatically.

| Provider | ID | Flow | What it is |
|----------|----|------|------------|
| Claude Code | `claude-code` | OAuth PKCE | Anthropic's Claude as a CLI agent |
| OpenAI Codex | `codex` | OAuth PKCE | OpenAI's Codex CLI |
| Kiro / Amazon Q | `kiro` | Device code (AWS SSO OIDC) | AWS's AI coding assistant |
| Kimi Coding | `kimi-coding` | Device code | Moonshot AI's coding agent |
| GitHub Copilot | `github-copilot` | Device code | GitHub's Copilot (short-lived tokens, auto-refreshed) |
| Antigravity | `antigravity` | Google OAuth | Google Cloud Code (requires GCP project) |

```bash
# Connect an OAuth provider
vkdg setup
# → picks the provider → walks through auth flow
# → token stored in ~/.config/vkdg/vault/ (encrypted)
# → auto-refreshed forever
```

After setup, the connection works with no `auth` block in your config — VKDG finds the stored token by provider ID.

---

## Free tier providers — zero API key required

Some providers offer a free tier generous enough for real usage. For most of these, VKDG works as a config-only connection (no plugin needed — they all speak OpenAI-compat).

```yaml
# SambaNova — free Llama inference
connections:
  - id: sambanova
    provider: openai-compat
    base_url: https://api.sambanova.ai
    auth: { type: api_key, env_var: SAMBANOVA_API_KEY }  # free at cloud.sambanova.ai
    models: ["Meta-Llama-3.1-405B-Instruct", "Meta-Llama-3.1-70B-Instruct"]

# Cerebras — fast free inference
connections:
  - id: cerebras
    provider: openai-compat
    base_url: https://api.cerebras.ai/v1
    auth: { type: api_key, env_var: CEREBRAS_API_KEY }   # free at inference.cerebras.ai
    models: ["llama3.1-70b", "llama3.1-8b"]

# Cloudflare AI — free inference via Workers AI
connections:
  - id: cloudflare
    provider: openai-compat
    base_url: https://api.cloudflare.com/client/v4/accounts/{ACCOUNT_ID}/ai/v1
    auth: { type: api_key, env_var: CLOUDFLARE_API_TOKEN }
    models: ["@cf/meta/llama-3.1-70b-instruct"]

# OpenRouter — 200+ models, many with free tiers
connections:
  - id: openrouter
    provider: openai-compat
    base_url: https://openrouter.ai/api
    auth: { type: api_key, env_var: OPENROUTER_API_KEY }  # free tier available
    models: ["*"]  # route any model name through OpenRouter
```

---

## Any OpenAI-compatible endpoint

If an API serves `/v1/chat/completions`, connect it as `openai-compat`:

```yaml
connections:
  - id: local-ollama
    provider: openai-compat
    base_url: http://localhost:11434
    auth: { type: api_key, env_var: OLLAMA_KEY }  # any value; Ollama ignores it
    models: ["llama3.3:70b", "qwen2.5-coder:32b"]
    max_concurrent: 4
```

Works for: Ollama, vLLM, LM Studio, LocalAI, Open WebUI, any self-hosted model.

For Anthropic-compatible endpoints (proxies that implement the Messages API), use `provider: anthropic-compat`.

---

## Adding a provider

VKDG is designed so that adding a provider never requires touching the core. Three paths:

**Config-only** — the fastest path. If the API speaks `/v1/chat/completions` or `/v1/messages`, write a YAML connection. Done. No code, no build, no restart needed.

**WASM plugin** — write an adapter in any language that compiles to `wasm32-wasip2`. Implements the `provider.wit` interface. Install with `vkdg plugin install`. Runs sandboxed (no filesystem access, no outbound calls — the gateway owns the HTTP client). The right choice for:
- Private/internal APIs
- Providers with proprietary protocols
- Community contributions without a VKDG rebuild

**Rust crate** — add a Rust crate to `plugins/providers/` in the VKDG repo. Gets compiled into the binary and ships with VKDG. The right choice for widely-used providers that warrant first-party support and maintenance.

→ Full guide: [adding-a-provider.md](../sdk/adding-a-provider.md)

---

## Community providers

The [plugin registry](./registry.md) lists community-maintained provider plugins. If a provider isn't in the official list and isn't OpenAI-compat, it might already exist as a community plugin:

```bash
vkdg plugin search <provider-name>
vkdg plugin install <name>
```

If you've built a provider adapter, submit it to the registry — it takes one YAML manifest file and a PR.

---

## The bigger picture

VKDG ships adapters for the providers we actually use and can maintain. The list will grow, but it will always be curated — we'd rather have 20 adapters that work correctly than 200 that might be stale.

The design is explicitly modular: the provider list is not hard-coded into the core. Every official provider is a separate Rust crate in `plugins/providers/`. Adding one doesn't change any core routing, admission, or streaming logic.

If you need a provider that isn't here:
- OpenAI-compat endpoint? Config-only, works now.
- Custom protocol? WASM plugin, no VKDG rebuild.
- Widely used and want to contribute? PR to `plugins/providers/`.

The registry tracks what the community builds. The core stays minimal.
