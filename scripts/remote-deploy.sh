#!/usr/bin/env bash
# Usage: scripts/remote-deploy.sh <sha7>
# Sync, build deploy/Dockerfile on the server as vkdg-gateway:dev-<sha7>,
# keep the previous latest as rollback + rollback-<oldsha>, swap, restart, and
# measure downtime via http://127.0.0.1:8080/health on the server.
#
# VKDG_PROFILE: cargo profile (default ship = no LTO, fast compile; release for prod).
# VKDG_PREBUILT=1: skip sync + build; vkdg-gateway:dev-<sha7> must already be
# loaded on the host (scripts/ship-dev.sh does this after a local build).
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/remote-lib.sh"

SHA="${1:-}"
[[ "$SHA" =~ ^[0-9a-f]{7}$ ]] || die "usage: $0 <sha7>  (7 hex chars)"
TAG="dev-$SHA"

if [[ "${VKDG_PREBUILT:-}" == 1 ]]; then
  rssh "docker image inspect vkdg-gateway:$TAG >/dev/null" \
    || die "VKDG_PREBUILT=1 but vkdg-gateway:$TAG is not loaded on $REMOTE_HOST"
else
  remote_sync
  echo "building vkdg-gateway:$TAG on $REMOTE_HOST (nice 19)" >&2
  rssh -T "set -e; mkdir -p '$REMOTE_BUILD_DIR'; exec 9>'$REMOTE_LOCK'; flock 9
    cd '$REMOTE_SRC'
    DOCKER_BUILDKIT=1 nice -n 19 docker build -f deploy/Dockerfile \
      --build-arg PROFILE='${VKDG_PROFILE:-ship}' --build-arg VCS_REF='$SHA' \
      --build-arg BUILD_DATE=\$(date -u +%Y-%m-%dT%H:%M:%SZ) \
      -t vkdg-gateway:$TAG ." \
    || die "docker build failed; nothing was swapped"
fi

echo "swapping tags and restarting gateway" >&2
rssh -T bash -s -- "$TAG" <<'REMOTE'
set -euo pipefail
TAG="$1"
cd /opt/vkdg

# Tag the current latest as rollback / rollback-<oldsha>.
if docker image inspect vkdg-gateway:latest >/dev/null 2>&1; then
  old_id="$(docker image inspect vkdg-gateway:latest --format '{{.Id}}')"
  oldsha="$(docker images vkdg-gateway --format '{{.Tag}} {{.ID}}' \
    | while read -r t _; do
        case "$t" in dev-*) [ "$(docker image inspect "vkdg-gateway:$t" --format '{{.Id}}')" = "$old_id" ] && echo "${t#dev-}" && break;; esac
      done)"
  [ -n "$oldsha" ] || oldsha="$(echo "${old_id#sha256:}" | cut -c1-7)"
  docker tag vkdg-gateway:latest vkdg-gateway:rollback
  docker tag vkdg-gateway:latest "vkdg-gateway:rollback-$oldsha"
  echo "previous image saved: vkdg-gateway:rollback and vkdg-gateway:rollback-$oldsha"
  echo "$oldsha" >/root/vkdg-build/last-rollback-sha
fi

docker tag "vkdg-gateway:$TAG" vkdg-gateway:latest

start="$(date +%s.%N)"
nice -n 19 docker compose up -d --force-recreate gateway

# Downtime: from just before recreate until /health first answers 2xx.
code=000
for _ in $(seq 1 600); do
  code="$(curl -s -o /dev/null -m 1 -w '%{http_code}' http://127.0.0.1:8080/health || true)"
  case "$code" in 2*) break;; esac
  sleep 0.1
done
end="$(date +%s.%N)"
echo "health http=$code; elapsed until healthy (upper bound on downtime): $(awk -v s="$start" -v e="$end" 'BEGIN{printf "%.1fs", e-s}')"

sleep 3
docker inspect vkdg --format 'restartCount={{.RestartCount}} OOMKilled={{.State.OOMKilled}} status={{.State.Status}} health={{if .State.Health}}{{.State.Health.Status}}{{else}}n/a{{end}} image={{.Config.Image}}'
echo "health after settle: http=$(curl -s -o /dev/null -m 2 -w '%{http_code}' http://127.0.0.1:8080/health || true)"

# Housekeeping, so the disk never fills: the host only runs VKDG.
# Keep latest, rollback, and the 3 newest rollback-<sha> / dev-<sha> tags.
for prefix in rollback- dev-; do
  docker images vkdg-gateway --format '{{.CreatedAt}}\t{{.Tag}}' \
    | awk -v p="$prefix" -F'\t' 'index($2, p) == 1' | sort -r | tail -n +4 | cut -f2 \
    | while read -r t; do [ "$t" = "$TAG" ] || docker rmi "vkdg-gateway:$t" >/dev/null 2>&1 || true; done
done
docker image prune -f >/dev/null
docker builder prune -f --keep-storage 3GB >/dev/null 2>&1 || true
echo "disk after deploy: $(df -h / | awk 'NR==2 {print $3 " used, " $4 " free"}')"
REMOTE

echo >&2
echo "deployed vkdg-gateway:$TAG. To roll back, run:" >&2
echo "  ssh $REMOTE_HOST 'docker tag vkdg-gateway:rollback vkdg-gateway:latest && cd /opt/vkdg && docker compose up -d --force-recreate gateway'" >&2
