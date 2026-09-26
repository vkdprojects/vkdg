# Contributing a plugin

Thanks for adding to the registry. Here's how it works.

## 5 steps

**1. Check it doesn't already exist**

```sh
vkdg plugin search <your-plugin-name>
```

Also scan `plugins/` in this repo. Duplicate names aren't accepted.

**2. Fork and create your manifest**

Fork this repo, then add a single YAML file:

```
plugins/<kind>/<name>.yaml
```

Valid kinds: `provider`, `oauth-provider`, `filter-pack`, `compressor`, `router`.

Use an existing manifest as a template — `plugins/providers/openrouter.yaml` is a good starting point for config-only providers. Full field reference: [`schema.yaml`](./schema.yaml).

**3. Open a PR**

The validation bot runs automatically on any PR that touches `plugins/**/*.yaml`. It checks:

- All required fields are present and valid
- `name` matches the filename (without `.yaml`)
- `version` is valid semver
- `checksum` is a real `sha256:` hex string (for WASM plugins)
- `license` is a valid SPDX identifier

Fix any bot errors before requesting review.

**4. Wait for review**

A maintainer will check that the plugin does what it says, the author/repository fields are accurate, and the manifest isn't duplicating something that already exists. Most PRs are reviewed within a few days.

**5. Done**

Once merged, `vkdg plugin search` and `vkdg plugin install` will find your plugin immediately — the CLI fetches the registry index on demand.

---

## Notes

- **WASM plugins**: the `.wasm` URL must be a stable, versioned release URL (GitHub Releases works well). The `checksum` must be the real SHA-256 of the file — the bot verifies it.
- **Config-only providers**: just the `config_snippet` block. No binary needed.
- **Versioning**: bump the `version` field in a follow-up PR when you release a new version. The registry tracks the latest stable version per plugin name.
- **Yanking**: open a PR removing the manifest or setting a `yanked: true` field with a reason. Yanked plugins won't be installed by default.

Questions? Open an issue or ask in the VKDG discussions.
