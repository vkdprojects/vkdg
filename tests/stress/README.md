# VKDG Stress Test

Quality and latency validation suite for the VKDG gateway.

## Quick start

```bash
# Against vkdg.codeatlas.com.br (default)
export VKDG_API_KEY=vkdg_xxx
just stress

# Custom URL + key
just stress --url https://vkdg.vixpi.host --key vkdg_xxx

# More turns, tighter latency threshold
just stress --turns 60 --latency-p95 15000

# Against an OpenAI-compatible endpoint
just stress --provider openai --url https://api.openai.com --key sk-xxx --model gpt-4o
```

## Environment variables

| Variable | Default | Description |
|---|---|---|
| `VKDG_BASE_URL` | `https://vkdg.codeatlas.com.br` | Gateway URL |
| `VKDG_API_KEY` | — | Client API key |
| `VKDG_MODEL` | `claude-sonnet-4.6` | Model ID |
| `VKDG_PROVIDER` | `anthropic` | Wire format (`anthropic` or `openai`) |

## What it tests

1. **Context persistence** — injects a secret word in the system prompt, verifies the model remembers it at turns 10, 20, 30, etc.
2. **Identity preservation** — the model must follow the system prompt throughout (not "forget" who it is mid-conversation).
3. **Simple arithmetic** — baseline sanity on every turn.
4. **Latency** — measures TTFB p50/p95/p99 across all turns; fails if above thresholds.
5. **Gateway errors** — any non-200 response is a failure.

## Exit codes

| Code | Meaning |
|---|---|
| `0` | All quality checks passed and latency within bounds |
| `1` | One or more quality/latency checks failed |
| `2` | Bad arguments or no API key |

## Requirements

- [`uv`](https://docs.astral.sh/uv/) — the script is self-contained via `uv run`
- No other install step needed — `uv` handles the venv and deps on first run
