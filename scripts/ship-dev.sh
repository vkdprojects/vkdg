#!/usr/bin/env bash
# Usage: scripts/ship-dev.sh <host> [--plan]   (via: just ship-dev <host>)
#   --plan  print where it would build and why, then exit (no deploy)
#
# Deploy HEAD to <host>. Picks where to build:
#   local  — this machine cross-compiles (cargo-zigbuild) and packages it with
#            `deploy/Dockerfile --target prebuilt`, then streams the image over SSH.
#            ~1.5 min on an M-series Mac vs ~15 min on the server.
#   remote — sync source, build deploy/Dockerfile on the host.
#            Safe on any machine; slow.
# Either way the host does the same tag swap, restart and health check
# (scripts/remote-deploy.sh), so rollback works identically.
#
# Auto picks local when: toolchain present (cargo-zigbuild, zig, docker daemon)
# AND >= 8 CPUs AND >= 16 GiB RAM. Override: VKDG_BUILD=local|remote.
# A failed local build falls back to remote unless VKDG_BUILD=local.
# Cargo profile: VKDG_PROFILE (default `ship`: no LTO, ~3x faster; `release` for prod parity).
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

HOST="${1:?usage: $0 <host>}"
export VKDG_REMOTE_HOST="$HOST"

# ── Where to build ──────────────────────────────────────────────────────────
why=""
if [ "$(uname -s)" = Darwin ]; then
  cpus="$(/usr/sbin/sysctl -n hw.ncpu)"; mem_gib=$(( $(/usr/sbin/sysctl -n hw.memsize) / 1073741824 ))
else
  cpus="$(nproc)"; mem_gib=$(( $(awk '/MemTotal/ {print $2}' /proc/meminfo) / 1048576 ))
fi
for tool in cargo-zigbuild zig docker bun; do
  command -v "$tool" >/dev/null || { why="missing $tool"; break; }
done
[ -z "$why" ] && ! docker info >/dev/null 2>&1 && why="docker daemon not running"
[ -z "$why" ] && [ "$cpus" -lt 8 ] && why="$cpus CPUs (< 8)"
[ -z "$why" ] && [ "$mem_gib" -lt 16 ] && why="${mem_gib} GiB RAM (< 16)"

mode="${VKDG_BUILD:-auto}"
if [ "$mode" = auto ]; then
  if [ -z "$why" ]; then mode=local; else mode=remote; fi
fi
case "$mode" in
  local)  echo "▶ building locally ($cpus CPUs, ${mem_gib} GiB, profile ${VKDG_PROFILE:-ship})" >&2 ;;
  remote) echo "▶ building on $HOST, profile ${VKDG_PROFILE:-ship}${why:+ — local build skipped: $why}" >&2 ;;
  *) echo "error: VKDG_BUILD must be local, remote or auto" >&2; exit 2 ;;
esac
[ "${2:-}" = --plan ] && exit 0

if [ -n "$(git status --porcelain)" ]; then
  echo "error: uncommitted changes; commit first so the deployed tag is reproducible" >&2
  git status --short >&2
  exit 1
fi
SHA="$(git rev-parse --short=7 HEAD)"
TAG="dev-$SHA"


# ── Local build → stream → swap ─────────────────────────────────────────────
PROFILE="${VKDG_PROFILE:-ship}"
export VKDG_PROFILE="$PROFILE"
build_local() {
  (cd apps/console && bun install --frozen-lockfile && bun run build) &&
  cargo zigbuild --profile "$PROFILE" -p vkdg --target x86_64-unknown-linux-musl &&
  docker build --platform linux/amd64 -f deploy/Dockerfile --target prebuilt \
    --build-arg PROFILE="$PROFILE" --build-arg VCS_REF="$SHA" \
    --build-arg BUILD_DATE="$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    -t "vkdg-gateway:$TAG" . &&
  { echo "▶ streaming vkdg-gateway:$TAG to $HOST" >&2
    docker save "vkdg-gateway:$TAG" | gzip -1 | ssh "$HOST" 'gunzip | docker load'; }
}

start=$(date +%s)
if [ "$mode" = local ]; then
  if build_local; then
    VKDG_PREBUILT=1 scripts/remote-deploy.sh "$SHA"
  elif [ "${VKDG_BUILD:-auto}" = local ]; then
    echo "error: local build failed (VKDG_BUILD=local, no fallback)" >&2; exit 1
  else
    echo "⚠ local build failed; falling back to building on $HOST" >&2
    scripts/remote-deploy.sh "$SHA"
  fi
else
  scripts/remote-deploy.sh "$SHA"
fi
echo "✔ shipped $TAG to $HOST in $(( $(date +%s) - start ))s ($mode build). Roll back: just rollback-dev $HOST" >&2
