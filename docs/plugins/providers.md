# VKDG Providers

A provider adapter translates VKDG's internal request format into whatever wire format the upstream API expects. The gateway core knows nothing about HTTP URLs, auth headers, or JSON schemas — that's entirely the provider's job.

---

## What's a provider?

When you send a request through VKDG, the gateway routes it to a connection. That connection references a provider. The provider takes the normalized internal request and turns it into a real HTTP call — the right URL, the right headers, the right body shape — then translates the response back.

Every provider is an adapter. The core doesn't care whether the upstream speaks Anthropic Messages format, OpenAI Chat Completions, or something proprietary. It just hands the provider a request and expects a response.

---

## Two kinds of adapters

**Compiled-in** — Rust crate in `plugins/providers/`. Bundled with the binary, zero latency, full async Rust. These are the official providers that ship with every VKDG build.

**WASM plugin** — a `.wasm` file installed separately via `vkdg plugin install`. Any language that compiles to `wasm32-wasip2`. Sandboxed (no filesystem, no outbound calls), hot-loadable without a rebuild. Community providers live here.

**Config-only** — for any OpenAI-compatible or Anthropic-compatible endpoint. No code at all. Set `provider: openai-compat` and point it at a base URL. Works for Ollama, vLLM, LM Studio, OpenRouter, and hundreds of others.

If the API you want to connect speaks `/v1/chat/completions`, you almost certainly don't need to write a plugin.

---

## Official providers

These ship with every VKDG binary. No install needed — just configure a connection.

### API key providers

| Name | ID | Base URL | Notes |
|------|----|----------|-------|
| Anthropic | `anthropic` | `https://api.anthropic.com` | Native Anthropic Messages format |
| OpenAI | `openai` | `https://api.openai.com` | Chat Completions + Images |
| Google Gemini | `gemini` | `https://generativelanguage.googleapis.com/v1beta/openai` | OpenAI-compat endpoint |
| Groq | `groq` | `https://api.groq.com/openai` | OpenAI-compat, very fast inference |
| DeepSeek | `deepseek` | `https://api.deepseek.com` | OpenAI-compat |
| Mistral | `mistral` | `https://api.mistral.ai` | OpenAI-compat |
| Together AI | `together` | `https://api.together.xyz` | OpenAI-compat |
| Fireworks AI | `fireworks` | `https://api.fireworks.ai/inference` | OpenAI-compat |

Auth for all of these is an API key in an environment variable:

```yaml
connections:
  - id: anthropic-main
    provider: anthropic
    auth:
      type: api_key
      env_var: ANTHROPIC_API_KEY
    models: ["claude-*"]
```

### OAuth providers (code agents)

These are AI coding agents that use OAuth instead of API keys. You authenticate once; VKDG stores the token in `~/.config/vkdg/vault/` and refreshes it automatically.

| Name | ID | Auth flow | Notes |
|------|----|-----------|-------|
| Claude Code | `claude-code` | OAuth PKCE | Requires `CLAUDE_OAUTH_CLIENT_ID` |
| OpenAI Codex | `codex` | OAuth PKCE | Requires `CODEX_OAUTH_CLIENT_ID` |
| Kiro / Amazon Q | `kiro` | Device code (AWS SSO OIDC) | Per-connection client registration |
| Kimi Coding | `kimi-coding` | Device code | Requires stable device ID |
| GitHub Copilot | `github-copilot` | Device code | Short-lived copilot tokens, auto-refreshed |
| Antigravity | `antigravity` | Google OAuth | Requires GCP project onboarding |

To set up an OAuth provider, use `vkdg setup` — it walks you through the full flow interactively (see [Connecting an OAuth provider](#connecting-an-oauth-provider) below).

---

## Custom OpenAI-compatible endpoint (zero code)

Any API that speaks `/v1/chat/completions` works as a config-only connection:

```yaml
connections:
  - id: my-ollama
    provider: openai-compat
    base_url: http://localhost:11434
    auth:
      type: api_key
      env_var: OLLAMA_KEY   # set to any non-empty value for keyless endpoints
    models: ["llama3.3:70b", "qwen2.5-coder:32b"]
    max_concurrent: 4
```

Works for: Ollama, vLLM, LM Studio, LocalAI, Open WebUI, OpenRouter, DeepInfra, and anything else that implements the OpenAI spec.

For Anthropic-compatible endpoints (e.g., a proxy that wraps the Messages API), use `provider: anthropic-compat` instead.

---

## Connecting an OAuth provider

```bash
vkdg setup
```

Pick your provider from the list. Then:

1. VKDG prints an auth URL (PKCE) or a device code
2. Complete auth in your browser, or enter the code on the device
3. Token lands in `~/.config/vkdg/vault/` — encrypted at rest
4. VKDG refreshes it automatically; you never touch it again

After setup, the connection is ready to use in your config:

```yaml
connections:
  - id: copilot
    provider: github-copilot
    models: ["gpt-4o", "claude-*"]
```

No `auth` block needed — VKDG finds the stored token by provider ID.

---

## Adding a provider yourself

Three paths, roughly in order of effort:

**Config-only** — If the API is OpenAI or Anthropic compatible, just write a YAML connection. No code, no build. Works today.

**WASM plugin** — Implement `provider.wit`, compile to `.wasm`, install with `vkdg plugin install`. Any language. No VKDG rebuild. The right choice for community providers and private integrations.

**Contribute to `plugins/`** — Submit a Rust crate to `plugins/providers/` in the VKDG repo. Gets compiled into the binary and shipped with VKDG. The right choice for widely-used providers that warrant first-party support.

Full details on all three paths: [adding-a-provider.md](../sdk/adding-a-provider.md).

---

## Community providers

The [plugin registry](./registry.md) lists community-maintained provider plugins. Install any of them with:

```bash
vkdg plugin install <name>
```

If you've built a provider plugin, submit it to the registry — see [registry.md](./registry.md) for the manifest format and PR process.

---

## Provider config reference

All fields for a connection entry in `vkdg.yaml`:

```yaml
connections:
  - id: string            # unique; referenced in routes.targets[]
    provider: string      # official provider ID, or installed plugin name
    base_url: string      # override the provider's default URL (optional)
    auth:
      type: api_key | oauth2
      env_var: string     # api_key only: name of the env var holding the key
    models: [string]      # glob patterns this connection serves, e.g. "claude-*"
    max_concurrent: int   # max parallel in-flight requests (default: 100)
    weight: int           # for weighted load balancing across connections (default: 1)
    tags: [string]        # for tag-based routing rules
    capabilities:         # override auto-detected capabilities
      vision: bool
      tools: bool
      streaming: bool
```

`models` is used by the router to match incoming model names to connections. Globs are supported (`claude-*`, `gpt-4*`, `*`). A connection with `models: ["*"]` is a catch-all fallback.

`weight` only applies when multiple connections match the same request — higher weight means more traffic. A connection with `weight: 2` gets twice the requests of one with `weight: 1`.

`capabilities` is optional. VKDG auto-detects capabilities for official providers. Override it if you're using a custom endpoint that doesn't support everything the provider usually does (or supports more than VKDG knows about).
