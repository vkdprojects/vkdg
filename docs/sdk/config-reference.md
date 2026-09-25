# VKDG Configuration Reference

All configuration is provided as a YAML file. The gateway validates the full document at startup via `ConfigSnapshot::build()` before accepting any traffic. Invalid config prevents startup; invalid config on reload is rejected and the running snapshot is kept.

## Root structure

```yaml
listen: "0.0.0.0:8080"   # required
connections: [...]        # required; at least one
routes: [...]             # required; at least one
limits:                   # optional
observe:                  # optional
```

| Field | Type | Required | Description |
|---|---|---|---|
| `listen` | string | yes | TCP bind address, e.g. `"0.0.0.0:8080"` or `"127.0.0.1:3000"` |
| `connections` | list | yes | Upstream provider connections |
| `routes` | list | yes | Model-to-connection routing rules |
| `limits` | object | no | Global request limits; defaults applied if absent |
| `observe` | object | no | Telemetry settings; defaults applied if absent |

---

## connections

Each entry defines one upstream provider connection.

```yaml
connections:
  - id: anthropic-prod
    provider: anthropic
    auth:
      type: api_key
      env_var: ANTHROPIC_API_KEY
    models:
      - "claude-*"
    max_concurrent: 50
    weight: 1
```

| Field | Type | Required | Default | Description |
|---|---|---|---|---|
| `id` | string | yes | — | Unique identifier; referenced by `routes[].targets` |
| `provider` | string | yes | — | Provider kind (see below) |
| `auth` | object | yes | — | Authentication method (see `auth`) |
| `models` | list of strings | yes | — | Model names or patterns this connection serves |
| `max_concurrent` | integer | no | `u32::MAX` | Maximum simultaneous in-flight requests |
| `weight` | integer | no | `1` | Relative weight for `weighted` strategy |

### provider values

| Value | Description |
|---|---|
| `anthropic` | Anthropic Messages API (`https://api.anthropic.com`) |
| `openai` | OpenAI API (`https://api.openai.com`) |
| `google` | Google Generative Language API |
| `custom:<url>` | Any URL, e.g. `custom:https://my-proxy.internal` |

### models patterns

- Exact match: `"gpt-4o"` matches only that model name.
- Prefix match: `"claude-*"` matches any model name starting with `claude-`.
- Multiple patterns are OR-ed: a request matches the connection if any pattern matches.

---

## auth

Nested inside each connection entry.

### api_key

Reads the token from an environment variable at request time.

```yaml
auth:
  type: api_key
  env_var: ANTHROPIC_API_KEY
```

| Field | Type | Description |
|---|---|---|
| `type` | `"api_key"` | Selects this variant |
| `env_var` | string | Environment variable name that holds the API key |

The env var is resolved on every token fetch; rotating the variable value takes effect without restart.

### oauth2

Client-credentials OAuth2 flow. Token is fetched from `token_url` and cached until expiry.

```yaml
auth:
  type: oauth2
  token_url: https://auth.example.com/oauth2/token
  client_id: my-client
  client_secret_env: OAUTH_CLIENT_SECRET
  scopes:
    - completions.write
```

| Field | Type | Description |
|---|---|---|
| `type` | `"oauth2"` | Selects this variant |
| `token_url` | string | Full URL of the token endpoint |
| `client_id` | string | OAuth2 client identifier |
| `client_secret_env` | string | Environment variable holding the client secret |
| `scopes` | list of strings | Requested OAuth2 scopes |

---

## routes

Each route maps a set of model name patterns to one or more connection targets.

```yaml
routes:
  - id: claude-route
    match_models:
      - "claude-*"
    strategy: fallback_chain
    targets:
      - anthropic-prod
      - anthropic-backup
```

| Field | Type | Required | Description |
|---|---|---|---|
| `id` | string | yes | Unique identifier; appears in decision records and logs |
| `match_models` | list of strings | yes | Patterns matched against the requested model name (same rules as `connections[].models`) |
| `strategy` | string | yes | Selection strategy (see `strategies`) |
| `targets` | list of strings | yes | Connection ids to include; must all exist in `connections` |

Routes are evaluated in declaration order; the first matching route wins.

---

## strategies

| Value | Description |
|---|---|
| `round_robin` | Cycles through eligible targets using an atomic counter |
| `weighted` | Selects targets proportionally to their `weight` field |
| `fallback_chain` | Tries targets in declaration order; moves to next on failure |
| `lowest_latency` | Selects the target with the lowest observed p50 latency |
| `power_of_two_choices` | Picks two candidates at random, routes to the less-loaded one |
| `last_known_good` | Prefers the last target that returned a successful response |

A target is skipped if it is over its `max_concurrent` limit, its circuit is open, or it does not support the required capability set.

---

## limits

Global request limits applied before routing. All fields are optional; absence means no limit.

```yaml
limits:
  max_concurrent_requests: 500
  max_body_bytes: 10485760      # 10 MiB
  request_timeout_secs: 120
```

| Field | Type | Default | Description |
|---|---|---|---|
| `max_concurrent_requests` | integer | unlimited | Total concurrent requests gateway-wide; excess requests receive `429` |
| `max_body_bytes` | integer | unlimited | Maximum request body size in bytes; larger bodies receive `413` |
| `request_timeout_secs` | integer | unlimited | Per-request wall-clock timeout in seconds; expired requests receive `504` |

---

## observe

Telemetry and logging configuration. All fields are optional.

```yaml
observe:
  otlp_endpoint: "http://otel-collector:4317"
  log_level: info
  log_format: json
```

| Field | Type | Default | Description |
|---|---|---|---|
| `otlp_endpoint` | string | none | gRPC OTLP endpoint for traces and metrics |
| `log_level` | string | `"info"` | One of `trace`, `debug`, `info`, `warn`, `error` |
| `log_format` | string | `"pretty"` | `"pretty"` (human-readable) or `"json"` (structured) |

---

## Complete example: Anthropic + OpenAI fallback

```yaml
listen: "0.0.0.0:8080"

connections:
  - id: anthropic-prod
    provider: anthropic
    auth:
      type: api_key
      env_var: ANTHROPIC_API_KEY
    models:
      - "claude-*"
    max_concurrent: 40
    weight: 1

  - id: openai-prod
    provider: openai
    auth:
      type: api_key
      env_var: OPENAI_API_KEY
    models:
      - "gpt-*"
      - "o1-*"
      - "o3-*"
    max_concurrent: 40
    weight: 1

  - id: openai-fallback
    provider: openai
    auth:
      type: api_key
      env_var: OPENAI_FALLBACK_API_KEY
    models:
      - "gpt-*"
    max_concurrent: 10
    weight: 1

routes:
  - id: claude-primary
    match_models:
      - "claude-*"
    strategy: round_robin
    targets:
      - anthropic-prod

  - id: openai-with-fallback
    match_models:
      - "gpt-*"
      - "o1-*"
      - "o3-*"
    strategy: fallback_chain
    targets:
      - openai-prod
      - openai-fallback

limits:
  max_concurrent_requests: 200
  max_body_bytes: 5242880     # 5 MiB
  request_timeout_secs: 90

observe:
  otlp_endpoint: "http://localhost:4317"
  log_level: info
  log_format: json
```

---

## Hot-reload

The gateway watches the config file for changes using `vkdg_config::watch`. On change:

1. The new file is parsed and passed to `ConfigSnapshot::build()`.
2. If validation fails, the error is logged and the running snapshot is kept — **no traffic is disrupted**.
3. If validation succeeds, the new `Arc<ConfigSnapshot>` is published via a `tokio::sync::watch` channel.
4. In-flight requests finish against the old snapshot (receivers hold a strong `Arc` reference). New requests immediately use the new snapshot.

What triggers a reload:
- Any change to the config file on disk.

What does **not** trigger a reload:
- Environment variable changes (picked up at next token fetch without reload).
- Plugin binary changes (requires `uninstall` + `install_wasm`).

---

## Validation errors

`ConfigSnapshot::build()` validates cross-references and rejects:

| Error | Cause | Fix |
|---|---|---|
| `"duplicate connection id: <id>"` | Two connections share the same `id` | Give each connection a unique `id` |
| `"route '<id>' targets unknown connection '<cid>'"` | A route's `targets` entry has no matching connection `id` | Add the connection or fix the target name |
| `"route '<id>' has unknown strategy '<s>'"` | `strategy` value is not one of the accepted strings | Use one of the values listed in `strategies` above |
| `"provider parse error"` | `provider` is not `anthropic`/`openai`/`google` or `custom:<url>` | Check spelling; custom providers need the `custom:` prefix |

Run `vkdg config check --file gateway.yaml` to validate without starting the server.
