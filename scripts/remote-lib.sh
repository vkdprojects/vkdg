# shellcheck shell=bash
# Shared helpers for scripts/remote-*.sh. Source, do not execute.
# Builds run on the server; never run cargo / docker build on this machine.

REMOTE_HOST="${VKDG_REMOTE_HOST:-vkdg-prod}"
REMOTE_BUILD_DIR="${VKDG_REMOTE_BUILD_DIR:-/root/vkdg-build}"
REMOTE_SRC="$REMOTE_BUILD_DIR/src"
REMOTE_LOCK="$REMOTE_BUILD_DIR/lock"
BUILDER_IMAGE="vkdg-builder:latest"
MIN_FREE_GIB=3

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

die() { echo "error: $*" >&2; exit 1; }

rssh() { ssh -o BatchMode=yes "$REMOTE_HOST" "$@"; }

# Abort when the docker/data filesystem has less than MIN_FREE_GIB free.
remote_check_disk() {
  local avail
  avail="$(rssh 'root=$(docker info -f "{{.DockerRootDir}}" 2>/dev/null || echo /var/lib/docker); df --output=avail -B1 "$root" | tail -n1 | tr -d " "')" \
    || die "cannot query free space on $REMOTE_HOST"
  [[ "$avail" =~ ^[0-9]+$ ]] || die "unexpected df output: $avail"
  local need=$((MIN_FREE_GIB * 1024 * 1024 * 1024))
  echo "remote free space: $((avail / 1024 / 1024 / 1024)) GiB (minimum ${MIN_FREE_GIB} GiB)" >&2
  if ((avail < need)); then
    die "less than ${MIN_FREE_GIB} GiB free on $REMOTE_HOST docker/data filesystem; refusing to continue"
  fi
}

# Sync the working tree (tracked + untracked, minus ignored) into $REMOTE_SRC,
# replacing the previous copy so deleted files disappear.
remote_sync() {
  remote_check_disk
  cd "$REPO_ROOT"
  echo "syncing working tree to $REMOTE_HOST:$REMOTE_SRC" >&2
  # Secrets and heavy dirs never leave this machine, even if not gitignored.
  local exclude='(^|/)(\.git|target|node_modules)(/|$)|(^|/)\.env($|\.)'
  local keep='(^|/)\.env\.(example|sample|template)$'
  local list
  list="$(mktemp)"
  trap 'rm -f "$list"' RETURN
  git ls-files -z -co --exclude-standard --deduplicate \
    | { grep -zvE "$exclude" || true; } >"$list"
  # Re-admit .env.example-style files that the exclude pattern caught.
  git ls-files -z -co --exclude-standard --deduplicate \
    | { grep -zE "$keep" || true; } >>"$list"
  # Drop paths still in the index but deleted on disk, so any tar works
  # (GNU or macOS bsdtar; no --ignore-failed-read needed).
  local present
  present="$(mktemp)"
  trap 'rm -f "$list" "$present"' RETURN
  while IFS= read -r -d '' f; do [[ -e "$f" ]] && printf '%s\0' "$f"; done <"$list" >"$present"
  # macOS bsdtar: no ._* AppleDouble files, no xattr headers GNU tar warns about.
  local tar_flags=()
  tar --version 2>/dev/null | grep -q bsdtar && tar_flags=(--no-xattrs --no-mac-metadata)
  COPYFILE_DISABLE=1 tar ${tar_flags[@]+"${tar_flags[@]}"} --null -T "$present" -czf - \
    | rssh "set -e
      mkdir -p '$REMOTE_BUILD_DIR'
      rm -rf '$REMOTE_SRC.new'
      mkdir -p '$REMOTE_SRC.new'
      nice -n 19 tar -xzf - -C '$REMOTE_SRC.new'
      # RustEmbed (bin/vkdg) needs apps/console/build/ at compile time. Gitignored, so
      # never synced; give the SERVER copy an empty dir + placeholder (never local).
      # Lint/test gates do not need real assets; remote-deploy builds the real console in Docker.
      mkdir -p '$REMOTE_SRC.new/apps/console/build'
      [ -e '$REMOTE_SRC.new/apps/console/build/index.html' ] || echo '<!doctype html><title>vkdg console placeholder</title>' >'$REMOTE_SRC.new/apps/console/build/index.html'
      exec 9>'$REMOTE_LOCK'
      flock 9
      rm -rf '$REMOTE_SRC.old'
      [ -d '$REMOTE_SRC' ] && mv '$REMOTE_SRC' '$REMOTE_SRC.old'
      mv '$REMOTE_SRC.new' '$REMOTE_SRC'
      rm -rf '$REMOTE_SRC.old'" \
    || die "sync failed"
}

# Build vkdg-builder:latest on the server once (same base/toolchain as the
# builder stage of deploy/Dockerfile.gateway: rust:1-alpine + musl-dev
# pkgconfig openssl-dev). VKDG_BUILDER_REBUILD=1 forces a rebuild.
remote_ensure_builder() {
  if [[ "${VKDG_BUILDER_REBUILD:-0}" != 1 ]] && rssh "docker image inspect $BUILDER_IMAGE >/dev/null 2>&1"; then
    return 0
  fi
  echo "building $BUILDER_IMAGE on $REMOTE_HOST (one-time)" >&2
  rssh "mkdir -p '$REMOTE_BUILD_DIR' && exec 9>'$REMOTE_LOCK' && flock 9 && nice -n 19 docker build -t $BUILDER_IMAGE -" <<'DOCKERFILE' || die "builder image build failed"
FROM rust:1-alpine
RUN apk add --no-cache musl-dev pkgconfig openssl-dev git bash curl
RUN rustup component add rustfmt clippy
# Optional gate tools: a failure here only means the gate is reported as skipped.
RUN cargo install --locked typos-cli \
    || echo "WARN: typos not installed; that gate will be skipped"
# cargo-machete's jemalloc build needs make.
RUN apk add --no-cache make \
    && cargo install --locked cargo-machete \
    || echo "WARN: cargo-machete not installed; that gate will be skipped"
RUN rm -rf /usr/local/cargo/registry /usr/local/cargo/git
WORKDIR /src
DOCKERFILE
}

# remote_run <command...>: run inside the builder container against the synced
# source, niced and serialized on the remote lock. Output streams; exit code
# propagates.
remote_run() {
  remote_ensure_builder
  local quoted
  quoted="$(printf '%q ' "$@")"
  rssh -T "mkdir -p '$REMOTE_BUILD_DIR' && exec 9>'$REMOTE_LOCK' && flock 9 && exec nice -n 19 docker run --rm \
    -v '$REMOTE_SRC':/src -w /src \
    -v vkdg-cargo-registry:/usr/local/cargo/registry \
    -v vkdg-cargo-target:/src/target \
    -e CARGO_TERM_COLOR=${CARGO_TERM_COLOR:-never} -e CARGO_INCREMENTAL=0 \
    $BUILDER_IMAGE $quoted"
}


# Free bytes on the docker/data filesystem.
remote_free_bytes() {
  rssh 'root=$(docker info -f "{{.DockerRootDir}}" 2>/dev/null || echo /var/lib/docker); df --output=avail -B1 "$root" | tail -n1 | tr -d " "'
}

# Wipe the contents of the cargo target volume (costs a rebuild; keeps the volume).
remote_wipe_target() {
  echo "wiping vkdg-cargo-target contents" >&2
  rssh "docker run --rm -v vkdg-cargo-target:/t $BUILDER_IMAGE find /t -mindepth 1 -delete"
}
