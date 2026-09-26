# VKDG Plugin Registry

This repo is the community index for [VKDG](https://github.com/vkdprojects/vkdg) plugins. It contains YAML manifests that `vkdg plugin` uses to discover, install, and validate plugins.

VKDG itself lives at [vkdprojects/vkdg](https://github.com/vkdprojects/vkdg). This repo is just the index.

---

## Installing plugins

```sh
# Search the registry
vkdg plugin search openrouter
vkdg plugin search rust

# Install by name
vkdg plugin install openrouter
vkdg plugin install rust-build

# List installed plugins
vkdg plugin list

# Remove a plugin
vkdg plugin remove rust-build
```

Config-only providers (like `openrouter`) are activated by adding their snippet to your `vkdg.yaml`. WASM plugins are downloaded and stored in `~/.vkdg/plugins/`.

---

## Plugin kinds

| Kind | What it does |
|---|---|
| `provider` | Adds a new AI provider (config-only, no binary) |
| `oauth-provider` | Provider that uses OAuth flow for auth |
| `filter-pack` | Compresses a specific content class (WASM) |
| `compressor` | General-purpose compressor (WASM) |
| `router` | Custom routing logic (WASM) |

---

## Manifest format

Each plugin is a single YAML file under `plugins/<kind>/<name>.yaml`. Full field reference: [docs/plugins/registry.md](https://github.com/vkdprojects/vkdg/blob/main/docs/plugins/registry.md).

The schema is in [`schema.yaml`](./schema.yaml) in this repo. PRs are validated automatically by a GitHub Actions bot — if validation fails, the PR won't merge.

---

## Contributing a plugin

See [CONTRIBUTING.md](./CONTRIBUTING.md) for the full walkthrough. The short version:

1. Fork this repo.
2. Add your manifest at `plugins/<kind>/<name>.yaml`.
3. Open a PR — the bot validates the manifest automatically.
4. A maintainer reviews and merges.

Plugin names must be unique, kebab-case, and descriptive. If you're adding a provider for an existing service, check that it doesn't already exist.
