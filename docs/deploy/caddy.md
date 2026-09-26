# Caddy Deployment

Caddy is the recommended proxy for VKDG. It handles automatic HTTPS via Let's Encrypt with zero configuration, and its `flush_interval -1` directive ensures SSE streams reach clients immediately without buffering.

## Prerequisites

- A VPS with ports 80 and 443 open
- Two DNS A records pointing to your server:
  - `api.example.com` → your server IP
  - `console.example.com` → your server IP
- Docker and Docker Compose installed

## Caddyfile

```caddy
# api.example.com — AI gateway (data plane)
# Serves: POST /v1/messages, POST /v1/chat/completions
api.example.com {
    reverse_proxy vkdg:8080 {
        # Flush SSE tokens immediately — critical for streaming AI responses
        flush_interval -1

        transport http {
            # Large buffers prevent mid-stream stalls
            read_buffer_size  0
            write_buffer_size 0
            dial_timeout      30s
            keepalive         1h
        }
    }

    # CORS — required for browser clients using OpenAI-compat SDK
    @cors_preflight method OPTIONS
    handle @cors_preflight {
        header Access-Control-Allow-Origin  "*"
        header Access-Control-Allow-Methods "GET, POST, OPTIONS"
        header Access-Control-Allow-Headers "Authorization, Content-Type, anthropic-version, x-api-key, Accept, Origin"
        header Access-Control-Max-Age       "86400"
        respond "" 204
    }

    header Access-Control-Allow-Origin  "*"
    header Access-Control-Allow-Headers "Authorization, Content-Type, anthropic-version, x-api-key, Accept, Origin"

    # Coarse DoS guard — rate limit at VKDG for token-aware limiting
    rate_limit {
        zone api_zone {
            key    {remote_host}
            events 10
            window 1s
        }
    }

    encode {
        # gzip must be disabled for SSE endpoints — gzip accumulates input
        # before emitting a compressed block, breaking streaming
        # Caddy's encode block is request-type-aware; SSE responses
        # (text/event-stream) are excluded automatically when flush_interval -1
        # is set. Leaving encode here for non-streaming responses only.
        gzip
        match {
            not header Content-Type text/event-stream*
        }
    }

    log {
        output file /var/log/caddy/api.log
    }
}

# console.example.com — admin API + embedded web console
# Serves: GET / (SPA), /v1/admin/*, /login
console.example.com {
    reverse_proxy vkdg:9090 {
        # WebSocket support for future live-update features
        header_up Connection {>Connection}
        header_up Upgrade    {>Upgrade}
    }

    encode gzip

    header {
        # Security headers for the console
        Strict-Transport-Security "max-age=31536000; includeSubDomains"
        X-Content-Type-Options    "nosniff"
        X-Frame-Options           "DENY"
        Referrer-Policy           "strict-origin-when-cross-origin"
    }

    log {
        output file /var/log/caddy/console.log
    }
}
```

Save this as `Caddyfile` next to your `docker-compose.yml`.

## docker-compose.yml

```yaml
version: "3.9"

networks:
  proxy:
    external: true   # created once with: docker network create proxy

services:
  caddy:
    image: caddy:2.8-alpine
    restart: unless-stopped
    ports:
      - "80:80"
      - "443:443"
      - "443:443/udp"   # HTTP/3 (QUIC)
    volumes:
      - ./Caddyfile:/etc/caddy/Caddyfile:ro
      - caddy_data:/data           # TLS certificates (persist across restarts)
      - caddy_config:/config
      - caddy_logs:/var/log/caddy
    networks:
      - proxy
    depends_on:
      - vkdg

  vkdg:
    image: ghcr.io/codeatlasdev/vkdg:latest
    restart: unless-stopped
    environment:
      VKDG_CONFIG: /etc/vkdg/config.toml
      # Set a fixed bootstrap token to avoid it changing on each restart
      # VKDG_BOOTSTRAP_TOKEN: your-secure-token-here
    volumes:
      - ./config.toml:/etc/vkdg/config.toml:ro
      - vkdg_data:/var/lib/vkdg
    networks:
      - proxy
    # Ports are NOT exposed to the host — Caddy reaches vkdg via Docker network
    expose:
      - "8080"
      - "9090"

volumes:
  caddy_data:
  caddy_config:
  caddy_logs:
  vkdg_data:
```

## Start

```bash
# Create shared network (once per host)
docker network create proxy

# Start services
docker compose up -d

# Watch Caddy obtain certificates (takes ~10s on first run)
docker compose logs -f caddy
```

Caddy automatically obtains and renews TLS certificates from Let's Encrypt. No certbot, no cron jobs.

## Verification

### Test SSE streaming directly (no proxy)

```bash
curl -N -s http://localhost:8080/v1/messages \
  -H "Content-Type: application/json" \
  -H "x-api-key: your-key" \
  -d '{"model":"claude-3-5-haiku-20241022","max_tokens":64,"stream":true,"messages":[{"role":"user","content":"Count to 5 slowly"}]}'
```

You should see tokens arriving one by one. If they all arrive at once, the issue is upstream of the proxy.

### Test SSE streaming through Caddy

```bash
curl -N -s https://api.example.com/v1/messages \
  -H "Content-Type: application/json" \
  -H "x-api-key: your-key" \
  -d '{"model":"claude-3-5-haiku-20241022","max_tokens":64,"stream":true,"messages":[{"role":"user","content":"Count to 5 slowly"}]}'
```

The `-N` flag disables curl's own output buffering. Tokens should arrive at the same cadence as the direct test.

### Test the console

```bash
curl -s https://console.example.com/
# Should return HTML (the embedded SPA)
```

### Test CORS preflight

```bash
curl -v -X OPTIONS https://api.example.com/v1/messages \
  -H "Origin: https://myapp.example.com" \
  -H "Access-Control-Request-Method: POST" \
  -H "Access-Control-Request-Headers: Content-Type, x-api-key"
# Should return 204 with Access-Control-Allow-* headers
```

## Troubleshooting

**Certificate not issued / HTTPS not working**

Caddy requires ports 80 and 443 reachable from the internet for the ACME HTTP-01 challenge. Check:
```bash
docker compose logs caddy | grep -i "certificate\|acme\|error"
```

**SSE tokens arrive in bursts or all at once**

Verify `flush_interval -1` is in the Caddyfile and that Caddy was reloaded:
```bash
docker compose exec caddy caddy reload --config /etc/caddy/Caddyfile
```

**Connection drops at ~100s**

You are behind Cloudflare Free/Pro. See the [README warning](./README.md#️-cloudflare-warning). Switch the `api.*` subdomain to DNS-only (grey cloud) mode.

**`vkdg` container not reachable**

Verify both containers are on the same Docker network:
```bash
docker network inspect proxy | grep -A3 '"Name"'
```

Both `caddy` and `vkdg` should appear in the containers list.
