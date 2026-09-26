# nginx Deployment

nginx is the most widely deployed reverse proxy and gives you the finest control over buffering, timeouts, and headers. SSE streaming requires explicit configuration — nginx buffers responses by default.

## Prerequisites

- A VPS with ports 80 and 443 open
- Two DNS A records pointing to your server:
  - `api.example.com` → your server IP
  - `console.example.com` → your server IP
- Docker and Docker Compose installed
- Shared Docker network: `docker network create proxy`

> **Prefer automatic HTTPS?** Use [Caddy](./caddy.md) instead. nginx requires certbot or manual certificate management.

## nginx.conf

```nginx
# /etc/nginx/nginx.conf (or nginx/conf.d/vkdg.conf)

# ── Upstreams ────────────────────────────────────────────────────────────────
upstream vkdg_gateway {
    server vkdg:8080;
    keepalive 64;
}

upstream vkdg_console {
    server vkdg:9090;
    keepalive 16;
}

# ── HTTP → HTTPS redirect ────────────────────────────────────────────────────
server {
    listen 80;
    server_name api.example.com console.example.com;
    return 301 https://$host$request_uri;
}

# ── AI gateway: api.example.com (port 8080) ──────────────────────────────────
server {
    listen 443 ssl;
    http2 on;
    server_name api.example.com;

    ssl_certificate     /etc/letsencrypt/live/api.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/api.example.com/privkey.pem;
    ssl_protocols       TLSv1.2 TLSv1.3;
    ssl_ciphers         HIGH:!aNULL:!MD5;
    ssl_session_cache   shared:SSL:10m;
    ssl_session_timeout 10m;

    # ── Streaming routes (/v1/*) ─────────────────────────────────────────────
    location ~ ^/v1/ {
        proxy_pass http://vkdg_gateway;

        # CRITICAL: disable response buffering — nginx default is on,
        # which collects the entire response before forwarding it.
        proxy_buffering             off;
        proxy_request_buffering     off;
        proxy_cache                 off;

        # CRITICAL: disable gzip on streaming locations.
        # gzip must accumulate input before emitting a compressed block,
        # which defeats SSE streaming entirely.
        gzip                        off;

        # CRITICAL: extend timeouts for long AI completions.
        # Default proxy_read_timeout is 60s — causes drops mid-generation.
        proxy_read_timeout          3600s;
        proxy_send_timeout          3600s;
        proxy_connect_timeout       30s;

        # Use HTTP/1.1 to the upstream (required for chunked transfer / SSE)
        proxy_http_version          1.1;
        proxy_set_header Connection "";   # clear Connection: close

        # Standard proxy headers
        proxy_set_header Host              $host;
        proxy_set_header X-Real-IP         $remote_addr;
        proxy_set_header X-Forwarded-For   $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;

        # CORS — required for browser clients (OpenAI-compat SDK, etc.)
        # OPTIONS preflight
        if ($request_method = OPTIONS) {
            add_header Access-Control-Allow-Origin  "*" always;
            add_header Access-Control-Allow-Methods "GET, POST, OPTIONS" always;
            add_header Access-Control-Allow-Headers "Authorization, Content-Type, anthropic-version, x-api-key, Accept, Origin" always;
            add_header Access-Control-Max-Age       "86400" always;
            add_header Content-Length 0;
            return 204;
        }

        add_header Access-Control-Allow-Origin  "*" always;
        add_header Access-Control-Allow-Headers "Authorization, Content-Type, anthropic-version, x-api-key, Accept, Origin" always;

        # Coarse DoS guard (token-aware rate limiting is inside VKDG)
        limit_req zone=api_zone burst=20 nodelay;
    }

    # Reject anything outside /v1/ on the API port
    location / {
        return 404;
    }
}

# ── Console: console.example.com (port 9090) ─────────────────────────────────
server {
    listen 443 ssl;
    http2 on;
    server_name console.example.com;

    ssl_certificate     /etc/letsencrypt/live/console.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/console.example.com/privkey.pem;
    ssl_protocols       TLSv1.2 TLSv1.3;
    ssl_ciphers         HIGH:!aNULL:!MD5;
    ssl_session_cache   shared:SSL:10m;
    ssl_session_timeout 10m;

    location / {
        proxy_pass http://vkdg_console;

        # WebSocket support (for future live-update features)
        proxy_http_version  1.1;
        proxy_set_header    Upgrade    $http_upgrade;
        proxy_set_header    Connection "upgrade";

        proxy_set_header Host              $host;
        proxy_set_header X-Real-IP         $remote_addr;
        proxy_set_header X-Forwarded-For   $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;

        proxy_read_timeout  3600s;

        # Security headers for the console SPA
        add_header Strict-Transport-Security "max-age=31536000; includeSubDomains" always;
        add_header X-Content-Type-Options    "nosniff" always;
        add_header X-Frame-Options           "DENY" always;
        add_header Referrer-Policy           "strict-origin-when-cross-origin" always;
    }
}

# ── Rate limiting zone (defined in http block) ───────────────────────────────
# Place this in the http {} block of your nginx.conf:
#
#   limit_req_zone $binary_remote_addr zone=api_zone:10m rate=10r/s;
```

> **Note:** The `limit_req_zone` directive must go in the `http {}` block of your top-level `nginx.conf`, not inside a `server {}` block.

## docker-compose.yml

```yaml
version: "3.9"

networks:
  proxy:
    external: true   # docker network create proxy

services:
  nginx:
    image: nginx:1.27-alpine
    restart: unless-stopped
    ports:
      - "80:80"
      - "443:443"
    volumes:
      - ./nginx.conf:/etc/nginx/conf.d/vkdg.conf:ro
      - ./nginx-main.conf:/etc/nginx/nginx.conf:ro   # includes limit_req_zone
      - /etc/letsencrypt:/etc/letsencrypt:ro          # certbot certs
      - nginx_logs:/var/log/nginx
    networks:
      - proxy
    depends_on:
      - vkdg

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

volumes:
  nginx_logs:
  vkdg_data:
```

## TLS certificates with certbot

nginx does not manage certificates automatically. Use certbot:

```bash
# Install certbot (Ubuntu/Debian)
apt install -y certbot

# Obtain certificates (stop nginx first if it's on port 80)
certbot certonly --standalone \
  -d api.example.com \
  -d console.example.com \
  --email you@example.com \
  --agree-tos

# Certificates are written to:
# /etc/letsencrypt/live/api.example.com/fullchain.pem
# /etc/letsencrypt/live/api.example.com/privkey.pem
```

Add a cron job or systemd timer to renew:

```bash
# /etc/cron.d/certbot
0 3 * * * root certbot renew --quiet --deploy-hook "docker compose -f /opt/vkdg/docker-compose.yml exec nginx nginx -s reload"
```

## Start

```bash
docker network create proxy
docker compose up -d

# Confirm nginx started without errors
docker compose logs nginx
```

## Verification

### Test SSE streaming

```bash
curl -N -s https://api.example.com/v1/messages \
  -H "Content-Type: application/json" \
  -H "x-api-key: your-key" \
  -d '{"model":"claude-3-5-haiku-20241022","max_tokens":64,"stream":true,"messages":[{"role":"user","content":"Count to 5 slowly"}]}'
```

The `-N` flag disables curl's output buffering. Tokens should appear as they are generated, not all at once.

Compare direct vs. proxied to isolate which layer is buffering:

```bash
# Direct (no proxy) — tokens arrive one by one here?
curl -N -s http://localhost:8080/v1/messages ...

# Through nginx — same cadence?
curl -N -s https://api.example.com/v1/messages ...
```

### Test the console

```bash
curl -sv https://console.example.com/ | head -20
# Should return 200 with HTML content
```

### Confirm proxy headers are correct

```bash
curl -s https://api.example.com/v1/messages \
  -H "x-api-key: your-key" \
  -X OPTIONS \
  -H "Origin: https://myapp.example.com" \
  -v 2>&1 | grep -i "access-control"
# Should show Access-Control-Allow-Headers including anthropic-version
```

## Troubleshooting

**Stream arrives all at once at the end**

`proxy_buffering` is on somewhere. Verify your config is actually loaded:
```bash
docker compose exec nginx nginx -T | grep proxy_buffering
# Should show: proxy_buffering off;  (in the /v1/ location block)
```

**Connection drops at exactly 60s**

`proxy_read_timeout` is at the default. Check with:
```bash
docker compose exec nginx nginx -T | grep proxy_read_timeout
# Should show: proxy_read_timeout 3600s;
```

**Connection drops at exactly 100s**

Cloudflare Free/Pro hard idle cap. See the [README warning](./README.md#️-cloudflare-warning).

**`502 Bad Gateway`**

nginx cannot reach the `vkdg` container. Check the Docker network:
```bash
docker network inspect proxy | grep -A3 vkdg
docker compose exec nginx ping vkdg -c 2
```

**gzip breaking streaming**

Verify `gzip off` is in the `/v1/` location block, not just at server level:
```bash
docker compose exec nginx nginx -T | grep -A5 "location ~ \^/v1"
```
