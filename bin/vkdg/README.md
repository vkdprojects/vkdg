# bin/vkdg

Entrypoint for the VKDG binary. Wires all crates into a working server. Contains no domain logic — only configuration, composition, and initialization.

## Subcommands

```bash
vkdg serve [--listen <addr>]   # Start the HTTP gateway
vkdg doctor                    # Environment diagnostics
vkdg config check <file>       # Validate YAML configuration
```

## What `serve` does

1. Initializes tracing via `vkdg_observe::init_tracing`
2. Builds `PipelineState` from environment variables (`ANTHROPIC_API_KEY`, etc.) — optional; without a credential, `/v1/messages` returns 501
3. Creates `AppState` with `ServerConfig` and `PipelineState`
4. Calls `vkdg_http::build_router(state)` and `vkdg_http::serve(config, router)`

## Environment variables

| Variable | Effect |
|---|---|
| `ANTHROPIC_API_KEY` | Enables the pipeline with the Anthropic provider |
| `RUST_LOG` | Log level (e.g. `info`, `debug,vkdg_http=trace`) |
| `VKDG_LISTEN` | Listen address (default: `0.0.0.0:8080`) |

## Focal test

```bash
cargo test -p conformance smoke -- --nocapture
```

## Depends on

All workspace crates. This is the single composition point.
