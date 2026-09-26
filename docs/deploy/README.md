# VKDG Deployment — Reverse Proxy Guide

VKDG is a single Rust binary. No Node.js, no separate BFF, no sidecar required.

## Why a reverse proxy?

- TLS termination with automatic certificate renewal
- Custom domain names (`api.yourdomain.com`, `console.yourdomain.com`)
- SSE/streaming must reach clients unmodified — each proxy requires specific config to avoid buffering
- Rate limiting and CORS at the edge

## Port map

| Port | Service | Routes |
|------|---------|--------|
| 8080 | AI gateway (data plane) | `POST /v1/messages`, `POST /v1/chat/completions` |
| 9090 | Admin API + embedded console | `GET /`, `/v1/admin/*`, `/login` |

Port 8080 is pure AI API — no HTML, no web UI. Port 9090 serves the management console as a static SPA embedded in the binary.

## The 4 buffering layers that kill SSE

Streaming AI responses use Server-Sent Events (SSE). Each layer in the stack can silently buffer and break them:

| Layer | Default behavior | Fix |
|-------|-----------------|-----|
| nginx | `proxy_buffering on` — accumulates full response | `proxy_buffering off` |
| Traefik | `flushInterval=100ms` — bursts every 100ms | `flushInterval=-1` (immediate) |
| gzip at proxy | Must accumulate input before compressing | `gzip off` on streaming locations |
| Cloudflare free/pro | Hard 100s idle timeout — no override | Use non-CF proxy for API endpoint |

## Choose your proxy

| Option | Best for | Complexity | HTTPS |
|--------|----------|------------|-------|
| **Caddy** | Most setups; recommended | Low | Automatic |
| **Traefik** | Docker-native, service discovery | Medium | Automatic (ACME) |
| **nginx** | Fine-grained control, existing infra | Medium | Certbot / manual |

**Recommended: [Caddy](./caddy.md).** Automatic HTTPS, one config file, handles SSE correctly by default.

## Docker network

All three guides assume a shared Docker network named `proxy` so the proxy container can reach the `vkdg` container by hostname:

```bash
docker network create proxy
```

## ⚠️ Cloudflare warning

Cloudflare Free and Pro plans enforce a **100-second hard idle timeout** on HTTP responses. This cannot be overridden via proxy config. If your streaming requests exceed ~100s idle time, use:

- Cloudflare Enterprise (configurable timeout), or
- DNS-only mode (orange cloud off) for your API subdomain, or
- A different CDN/proxy for the `api.*` subdomain

## Guides

- [Caddy](./caddy.md) — recommended, automatic HTTPS
- [Traefik](./traefik.md) — Docker-native, labels-driven
- [nginx](./nginx.md) — battle-tested, fine-grained control

## Common failure modes

| Symptom | Root cause | Fix |
|---------|-----------|-----|
| Stream arrives all at once at the end | `proxy_buffering on` | `proxy_buffering off` |
| Stream arrives in 100ms bursts | Traefik default `flushInterval=100ms` | `flushInterval=-1` |
| Connection drops at exactly 60s | `proxy_read_timeout` default | `proxy_read_timeout 3600s` |
| Connection drops at exactly 100s | Cloudflare free/pro hard cap | Use non-CF proxy for API |
| Connection drops at exactly 180s | Traefik default `idleConnTimeout` | `forwardingTimeouts.readTimeout=0` |
| gzip breaks SSE | gzip accumulates before emitting | `gzip off` on streaming locations |
| Double billing / duplicate tool calls | Client reconnects after proxy timeout | Fix timeouts + idempotency keys |
