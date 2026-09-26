# VKDG Plugin Registry

The registry is how the VKDG ecosystem grows without forking the core. You write a plugin, publish a manifest, and anyone can install it with one command.

```bash
vkdg plugin install groq-extended
vkdg plugin install filter-pack-rust-build
vkdg plugin search oauth
```

---

## What counts as a plugin

VKDG has five plugin kinds, each covering a different extension point:

| Kind | What it does | Interface |
|------|-------------|-----------|
| `provider` | Translates VKDG requests to a provider's wire format | `ProviderAdapter` trait / `provider.wit` |
| `oauth-provider` | Provider + handles OAuth login + token refresh | `OAuthProvider` trait |
| `filter-pack` | Compresses a specific content class (git output, build logs, etc.) | `FilterPack` trait |
| `compressor` | Full compression strategy (can combine multiple filter packs) | `Compressor` trait |
| `router` | Custom routing strategy beyond the built-ins | `router.wit` |

You can ship a plugin as:

- **WASM** — any language that compiles to `wasm32-wasip2`. Runs sandboxed. Doesn't require recompiling VKDG.
- **Rust crate** — compiled into the VKDG binary via `plugins/`. Faster, more capable, requires a rebuild.
- **Config-only** — for OpenAI or Anthropic-compatible endpoints. Zero code, just a YAML snippet.

---

## The registry repo

The central registry lives at **[github.com/vkdprojects/vkdg-registry](https://github.com/vkdprojects/vkdg-registry)**.

It's a plain GitHub repo with one YAML file per plugin:

```
vkdg-registry/
  plugins/
    providers/
      groq-extended.yaml
      deepinfra.yaml
      novita.yaml
    filter-packs/
      rust-build-errors.yaml
      kotlin-build.yaml
      swift-errors.yaml
    compressors/
      aggressive-trim.yaml
    oauth-providers/
      gitlab-duo.yaml
      codeium.yaml
```

Official plugins (maintained by the VKDG team) live in `plugins/official/`. Community plugins live alongside them — there's no separate tier once a plugin passes review.

---

## The manifest format

Every plugin is one YAML file. Here's a provider manifest:

```yaml
# plugins/providers/deepinfra.yaml
name: deepinfra
version: "1.0.0"
kind: provider
description: "DeepInfra inference API — OpenAI-compatible, 50+ models"
author: "Jane Doe <jane@example.com>"
repository: "https://github.com/jane/vkdg-plugin-deepinfra"
license: MIT
tags: [openai-compat, inference, cheap]
models: ["meta-llama/*", "mistralai/*", "deepinfra/*"]

install:
  wasm: "https://github.com/jane/vkdg-plugin-deepinfra/releases/download/v1.0.0/deepinfra.wasm"
  checksum: "sha256:abc123def456..."
```

For a config-only provider (zero code):

```yaml
# plugins/providers/openrouter.yaml
name: openrouter
version: "1.0.0"
kind: provider
description: "OpenRouter — 200+ models via one OpenAI-compatible endpoint"
tags: [openai-compat, aggregator, free-tier]
models: ["openrouter/*", "anthropic/*", "google/*"]

install:
  config_snippet: |
    provider: openai-compat
    base_url: https://openrouter.ai/api
    auth:
      type: api_key
      env_var: OPENROUTER_API_KEY
```

For a filter pack:

```yaml
# plugins/filter-packs/rust-build-errors.yaml
name: rust-build-errors
version: "1.0.0"
kind: filter-pack
description: "Compresses Rust compiler output — keeps errors and warnings, drops decorative lines. Saves ~70% on cargo check output."
tags: [rust, cargo, build]
content_class: RustBuild

install:
  wasm: "https://github.com/jane/vkdg-filterpack-rust/releases/download/v1.0.0/rust-build-errors.wasm"
  checksum: "sha256:..."
```

### Full manifest spec

```yaml
# Required
name: string                   # unique across registry, kebab-case
version: string                # semver
kind: provider | oauth-provider | filter-pack | compressor | router
description: string            # one sentence, what it does
license: string                # SPDX identifier

# Recommended
author: string                 # "Name <email>" or GitHub handle
repository: string             # source code URL
tags: [string]                 # searchable labels
models: [string]               # glob patterns (provider/oauth-provider only)
content_class: string          # ContentClass name (filter-pack only)

# Install — one of:
install:
  wasm: string                 # URL to .wasm file
  checksum: string             # sha256:<hex>

  # OR:
  config_snippet: string       # YAML config inline (config-only providers)

  # OR:
  crate: string                # crates.io package name (Rust compile-in)
  crate_version: string        # exact version

# Optional
min_vkdg_version: string       # minimum vkdg version required
homepage: string
changelog: string              # URL to CHANGELOG
```

---

## Installing plugins

```bash
# Install from the central registry
vkdg plugin install deepinfra

# Install a specific version
vkdg plugin install deepinfra@1.0.0

# Install from a URL directly
vkdg plugin install https://github.com/jane/vkdg-plugin-deepinfra/releases/download/v1.0.0/deepinfra.wasm

# Search
vkdg plugin search rust
vkdg plugin search --kind filter-pack

# List installed plugins
vkdg plugin list

# Remove
vkdg plugin remove deepinfra

# Update all
vkdg plugin update
```

Plugins are stored in `~/.config/vkdg/plugins/` by default. Override with `VKDG_PLUGINS_DIR`.

### Taps — third-party registries

Like Homebrew taps, you can add any GitHub repo as a plugin source:

```bash
# Add a tap
vkdg plugin tap myorg/vkdg-plugins

# Install from a tap
vkdg plugin install myorg/vkdg-plugins/custom-provider

# List taps
vkdg plugin tap --list
```

A tap is a GitHub repo with the same structure as `vkdg-registry`. No approval needed — you own it.

---

## Using installed plugins

After installing, register the plugin in your config:

```yaml
# vkdg.yaml
plugins:
  - deepinfra          # provider plugin
  - rust-build-errors  # filter-pack plugin

connections:
  - id: deepinfra-main
    provider: deepinfra    # references the installed plugin
    auth:
      type: api_key
      env_var: DEEPINFRA_API_KEY
    models: ["meta-llama/*"]
```

Or add from the console: Settings → Plugins → Install.

---

## Publishing a plugin

### Step 1 — Build your plugin

For a WASM provider, implement the WIT interface (see [adding-a-provider.md](../sdk/adding-a-provider.md)):

```rust
// src/lib.rs
wit_bindgen::generate!({ world: "provider", path: "wit/" });

struct MyProvider;
impl Guest for MyProvider {
    fn name() -> String { "my-provider".into() }
    fn model_patterns() -> Vec<String> { vec!["myprovider/*".into()] }
    fn translate_request(req: Request, config_json: String, token: String)
        -> Result<(String, Vec<u8>, bool), PluginError> {
        // build your upstream request
        todo!()
    }
    // ...
}
export!(MyProvider);
```

Build:
```bash
cargo build --target wasm32-wasip2 --release
# produces target/wasm32-wasip2/release/my_provider.wasm
```

### Step 2 — Create a release

Upload your `.wasm` file to a GitHub Release. Generate the checksum:

```bash
sha256sum my_provider.wasm
# sha256:abc123...
```

### Step 3 — Submit to the registry

Fork [vkdprojects/vkdg-registry](https://github.com/vkdprojects/vkdg-registry), add your manifest to `plugins/providers/my-provider.yaml`, and open a PR.

The bot checks:
- ✓ Manifest schema is valid
- ✓ WASM URL is reachable
- ✓ Checksum matches the downloaded file
- ✓ Plugin compiles and the WIT interface is satisfied
- ✓ `name` doesn't conflict with an existing plugin

Review is lightweight — we check that the plugin does what it says and doesn't do anything obviously malicious. We don't audit third-party provider APIs.

### Step 4 — Merged

Once merged, your plugin is available to everyone:

```bash
vkdg plugin install my-provider
```

No waiting for a VKDG release. The registry is live-updated.

---

## Plugin security

WASM plugins run in a Wasmtime sandbox. They:

- ✓ Can transform request/response bytes
- ✓ Can set HTTP headers
- ✓ Can return errors
- ✗ Cannot access the filesystem
- ✗ Cannot make outbound network calls (the gateway makes the HTTP request, not the plugin)
- ✗ Cannot read environment variables or credentials
- ✗ Cannot access other requests' data

The gateway strips credentials before passing config to the plugin. The plugin sees connection metadata (models, tags, base URL) but never the API key or OAuth token.

Checksums are verified on install. Plugins are pinned by version — `vkdg plugin update` requires confirmation for breaking-version bumps.

---

## The official providers

These ship with VKDG and don't need to be installed:

| Provider | Kind | Auth |
|----------|------|------|
| Anthropic | provider | API key |
| OpenAI | provider | API key |
| Google Gemini | provider | API key |
| Groq | provider | API key |
| DeepSeek | provider | API key |
| Mistral | provider | API key |
| Together AI | provider | API key |
| Fireworks AI | provider | API key |
| Claude Code | oauth-provider | OAuth PKCE |
| OpenAI Codex | oauth-provider | OAuth PKCE |
| Kiro / Amazon Q | oauth-provider | Device code (AWS SSO OIDC) |
| Kimi Coding | oauth-provider | Device code |
| GitHub Copilot | oauth-provider | Device code |
| Antigravity | oauth-provider | Google OAuth |

If you think a provider should be official (widely used, stable API, we maintain the adapter), open an issue or PR against the main repo.

If it's niche or experimental, the registry is the right place.
