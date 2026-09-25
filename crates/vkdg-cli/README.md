# vkdg-cli

Gateway operational commands: `doctor` and `config check`. Contains no routing or HTTP logic.

## Public API

- `commands::doctor::run() -> Result<()>`: prints the version and basic environment state (`RUST_LOG`, etc.)
- `commands::config_check::run(path: &str) -> Result<()>`: validates a YAML configuration file; reports invalid fields with location

## Invariants

- No command modifies runtime state: they are read-only / diagnostics only
- Commands are async (`async fn`) for compatibility with the main binary's Tokio runtime

## Focal test

```bash
cargo test -p vkdg-cli
```

## Used by

`bin/vkdg`: the CLI's `doctor` and `config check` subcommands delegate to this crate.
