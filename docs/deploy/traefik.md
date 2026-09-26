# Traefik Deployment

Traefik is Docker-native: it reads labels directly from your `docker-compose.yml` and routes traffic automatically. It handles Let's Encrypt certificates via ACME.

> **Critical:** Traefik's `ServersTransport` configuration (required for SSE streaming) **cannot be set via labels**. You must provide a dynamic configuration file. See the [dynamic.yml section](#required-dynamicyml) below.

## Prerequisites

- A VPS with ports 80 and 443 open
- Two DNS A records pointing to your server:
  - `api.example.com` → your server IP
  - `console.example.com` → your server IP
- Docker and Docker Compose installed
- Shared Docker network: `docker network create proxy`

## File layout

```
.
├── docker-compose.yml
├── dynamic.yml          # REQUIRED — ServersTransport for SSE
└── acme.json            # Created by you; Traefik writes certs here
```

## docker-compose.yml

```yaml
version: "3.9"

networks:
  proxy:
    external: true   # docker network create proxy

services:
  traefik:
    image: traefik:v3.1
    restart: unless-stopped
    command:
      # API dashboard (disable in production or put behind auth)
      - "--api.dashboard=true"
      - "--api.insecure=false"

      # Docker provider — watches labels on containers
      - "--providers.docker=true"
      - "--providers.docker.exposedbydefault=false"
      - "--providers.docker.network=proxy"

      # Dynamic config file — required for ServersTransport
      - "--providers.file.filename=/etc/traefik/dynamic.yml"
      - "--providers.file.watch=true"

      # Entrypoints
      - "--entrypoints.web.address=:80"
      - "--entrypoints.websecure.address=:443"

      # Redirect HTTP → HTTPS
      - "--entrypoints.web.http.redirections.entrypoint.to=websecure"
      - "--entrypoints.web.http.redirections.entrypoint.scheme=https"
      - "--entrypoints.web.http.redirections.entrypoint.permanent=true"

      # Let's Encrypt ACME
      - "--certificatesresolvers.letsencrypt.acme.httpchallenge=true"
      - "--certificatesresolvers.letsencrypt.acme.httpchallenge.entrypoint=web"
      - "--certificatesresolvers.letsencrypt.acme.email=you@example.com"
      - "--certificatesresolvers.letsencrypt.acme.storage=/acme/acme.json"

      # Logging
      - "--log.level=INFO"
      - "--accesslog=true"
    ports:
      - "80:80"
      - "443:443"
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock:ro
      - ./acme.json:/acme/acme.json
      - ./dynamic.yml:/etc/traefik/dynamic.yml:ro
    networks:
      - proxy

  vkdg:
    image: ghcr.io/codeatlasdev/vkdg:latest
    restart: unless-stopped
    environment:
      VKDG_CONFIG: /etc/vkdg/config.toml
      # VKDG_BOOTSTRAP_TOKEN: your-secure-token-here
    volumes:
      - ./config.toml:/etc/vkdg/config.toml:ro
      - vkdg_data:/var/lib/vkdg
    expose:
      - "8080"
      - "9090"
    networks:
      - proxy
    labels:
      - "traefik.enable=true"

      # ── API gateway (port 8080) ──────────────────────────────────────────
      - "traefik.http.routers.vkdg-api.rule=Host(`api.example.com`)"
      - "traefik.http.routers.vkdg-api.entrypoints=websecure"
      - "traefik.http.routers.vkdg-api.tls.certresolver=letsencrypt"
      - "traefik.http.routers.vkdg-api.service=vkdg-api-svc"
      - "traefik.http.routers.vkdg-api.middlewares=cors@docker,ratelimit@docker"

      - "traefik.http.services.vkdg-api-svc.loadbalancer.server.port=8080"
      # Reference the ServersTransport defined in dynamic.yml (required for SSE)
      - "traefik.http.services.vkdg-api-svc.loadbalancer.serversTransport=llm-transport@file"
      # CRITICAL: flush immediately — default 100ms causes bursts in SSE streams
      - "traefik.http.services.vkdg-api-svc.loadbalancer.responseForwarding.flushInterval=-1"

      # ── Console (port 9090) ──────────────────────────────────────────────
      - "traefik.http.routers.vkdg-console.rule=Host(`console.example.com`)"
      - "traefik.http.routers.vkdg-console.entrypoints=websecure"
      - "traefik.http.routers.vkdg-console.tls.certresolver=letsencrypt"
      - "traefik.http.routers.vkdg-console.service=vkdg-console-svc"
      - "traefik.http.routers.vkdg-console.middlewares=security-headers@docker"

      - "traefik.http.services.vkdg-console-svc.loadbalancer.server.port=9090"

      # ── CORS middleware ──────────────────────────────────────────────────
      - "traefik.http.middlewares.cors.headers.accesscontrolalloworiginlist=*"
      - "traefik.http.middlewares.cors.headers.accesscontrolallowmethods=GET,POST,OPTIONS"
      - "traefik.http.middlewares.cors.headers.accesscontrolallowheaders=Authorization,Content-Type,anthropic-version,x-api-key,Accept,Origin"
      - "traefik.http.middlewares.cors.headers.accesscontrolmaxage=86400"

      # ── Rate limit middleware (coarse DoS guard) ─────────────────────────
      # Token-aware rate limiting is handled inside VKDG; this is a coarse guard only
      - "traefik.http.middlewares.ratelimit.ratelimit.average=10"
      - "traefik.http.middlewares.ratelimit.ratelimit.burst=20"
      - "traefik.http.middlewares.ratelimit.ratelimit.period=1s"

      # ── Security headers for console ─────────────────────────────────────
      - "traefik.http.middlewares.security-headers.headers.stsSeconds=31536000"
      - "traefik.http.middlewares.security-headers.headers.stsIncludeSubdomains=true"
      - "traefik.http.middlewares.security-headers.headers.contentTypeNosniff=true"
      - "traefik.http.middlewares.security-headers.headers.frameDeny=true"
      - "traefik.http.middlewares.security-headers.headers.referrerPolicy=strict-origin-when-cross-origin"

volumes:
  vkdg_data:
```

## Required: dynamic.yml

Labels alone cannot configure `ServersTransport`. This is a Traefik limitation — transport-level settings (connection timeouts, idle connection lifetime) must go in a dynamic configuration file.

Without this file, SSE connections will drop at **180 seconds** (Traefik's default `idleConnTimeout`).

```yaml
# dynamic.yml
http:
  serversTransports:
    llm-transport:
      # 0 = no read timeout — AI responses can take minutes for long completions
      forwardingTimeouts:
        readTimeout: 0
        # 30s dial timeout is fine; the long wait is the response, not the connection
        dialTimeout: 30s
        # Keep idle connections alive for 1h to avoid reconnect overhead
        idleConnTimeout: 3600s
      # Disable compression at transport level — gzip breaks SSE streaming
      disableHTTP2: false
      maxIdleConnsPerHost: 100
```

> **Why can't this be a label?**
> Traefik's Docker provider supports router/service/middleware configuration via labels, but `ServersTransport` is a separate object type in Traefik's dynamic config model. The label `serversTransport=name@file` references a transport defined in a file provider — `@file` is the suffix indicating the file provider. This two-step indirection is by design.

## acme.json setup

Traefik writes Let's Encrypt certificates to `acme.json`. The file must exist with mode `0600` before Traefik starts:

```bash
touch acme.json
chmod 600 acme.json
```

If Traefik starts without this, it may fail to write certificates or error on startup.

## Start

```bash
docker network create proxy
touch acme.json && chmod 600 acme.json
docker compose up -d

# Watch certificate issuance (~10-30s)
docker compose logs -f traefik | grep -i "certificate\|acme\|error"
```

## Verification

### Test SSE streaming

```bash
curl -N -s https://api.example.com/v1/messages \
  -H "Content-Type: application/json" \
  -H "x-api-key: your-key" \
  -d '{"model":"claude-3-5-haiku-20241022","max_tokens":64,"stream":true,"messages":[{"role":"user","content":"Count to 5 slowly"}]}'
```

Tokens should arrive one at a time. If they arrive in ~100ms bursts, `flushInterval=-1` is not applied — check the label spelling and reload.

### Confirm dynamic.yml is loaded

```bash
docker compose logs traefik | grep -i "dynamic\|file provider"
# Should show: "Starting provider" and "Loaded configuration from file"
```

### Test the console

```bash
curl -s https://console.example.com/
# Should return HTML
```

## Troubleshooting: the 5 most common Traefik SSE failures

### 1. Tokens arrive in 100ms bursts

**Cause:** `responseForwarding.flushInterval` defaults to 100ms.
**Fix:** Label `traefik.http.services.vkdg-api-svc.loadbalancer.responseForwarding.flushInterval=-1`
**Verify:** Watch output with `curl -N` — tokens should arrive individually, not in groups.

### 2. Connection drops at exactly 180s

**Cause:** `dynamic.yml` not loaded, so `idleConnTimeout` remains at the default (~180s).
**Fix:** Ensure `dynamic.yml` exists, is mounted read-only, and `--providers.file.filename` points to it.
```bash
docker compose exec traefik traefik version  # confirm it started
docker compose logs traefik | grep "file"
```

### 3. `ServersTransport` not found — `llm-transport@file` error in logs

**Cause:** The `dynamic.yml` file was not mounted or the `@file` suffix is missing from the label.
**Fix:**
- Confirm `./dynamic.yml:/etc/traefik/dynamic.yml:ro` is in volumes
- Confirm label ends with `@file`: `serversTransport=llm-transport@file`

### 4. Certificate not issued

**Cause:** Port 80 is not reachable from the internet (firewall), or Traefik started before `acme.json` existed.
```bash
# Check port 80 is open
curl -v http://api.example.com/.well-known/acme-challenge/test

# Re-create acme.json and restart
rm acme.json && touch acme.json && chmod 600 acme.json
docker compose restart traefik
```

### 5. Connection drops at exactly 100s

**Cause:** Cloudflare Free/Pro hard idle timeout — not a Traefik issue.
**Fix:** Switch `api.example.com` DNS to grey cloud (DNS-only) in Cloudflare, or upgrade to Enterprise.
