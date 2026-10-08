# syntax=docker/dockerfile:1.7
# check=error=true
#
# One Dockerfile, three ways to get the same runtime image:
#
#   docker build -f deploy/Dockerfile .                       # full build (CI, servers)
#   docker build -f deploy/Dockerfile --target prebuilt .     # package a binary built on the host
#   docker build -f deploy/Dockerfile --build-arg PROFILE=ship .   # faster compile for dev deploys
#
# `just ship-dev HOST` picks between full (on the host) and prebuilt (local
# cross-compile) automatically; see scripts/ship-dev.sh.
#
# Layering, cheapest-to-change last:
#   console   bun deps (lockfile) → console source → build      [parallel with rust]
#   chef      pinned toolchain + cargo-chef (rarely changes)
#   planner   recipe.json = f(Cargo.toml, Cargo.lock) only
#   deps      `cargo chef cook`: every dependency, cached until manifests change
#   builder   workspace source + console → vkdg binary
#   runtime   scratch + CA bundle + binary (~32 MB, no shell, no package manager)

ARG RUST_VERSION=1.99
ARG ALPINE_VERSION=3.24
ARG BUN_VERSION=1.4.2
ARG CARGO_CHEF_VERSION=0.1.78
ARG TARGET=x86_64-unknown-linux-musl
# release: thin LTO, 1 codegen unit (production). ship: no LTO, 16 units,
# same opt-level; ~3x faster to compile, for dev deploys.
ARG PROFILE=release

# ── console: SvelteKit static build, embedded into the binary later ─────────
# $BUILDPLATFORM: runs natively on the builder's arch (output is plain files).
FROM --platform=$BUILDPLATFORM oven/bun:${BUN_VERSION}-alpine AS console
WORKDIR /src/apps/console
COPY apps/console/package.json apps/console/bun.lock ./
RUN --mount=type=cache,id=vkdg-bun,target=/root/.bun/install/cache \
    bun install --frozen-lockfile
COPY apps/console/ ./
RUN bun run build

# ── chef: pinned toolchain + cargo-chef ──────────────────────────────────────
FROM rust:${RUST_VERSION}-alpine${ALPINE_VERSION} AS chef
ARG CARGO_CHEF_VERSION
RUN apk add --no-cache musl-dev
RUN --mount=type=cache,id=vkdg-cargo-registry,target=/usr/local/cargo/registry,sharing=locked \
    cargo install cargo-chef --locked --version ${CARGO_CHEF_VERSION}
WORKDIR /src

# ── planner: dependency recipe (stable across source-only edits) ────────────
FROM chef AS planner
COPY . .
# rust-embed checks this folder exists at macro-expansion time.
RUN mkdir -p apps/console/build && cargo chef prepare --recipe-path recipe.json

# ── builder: deps (cached by recipe) then the workspace ──────────────────────
FROM chef AS builder
ARG TARGET
ARG PROFILE
COPY --from=planner /src/recipe.json recipe.json
RUN mkdir -p apps/console/build
# Shared cache ids: deps and final build reuse one target dir; `locked`
# serializes concurrent builds instead of corrupting it.
RUN --mount=type=cache,id=vkdg-cargo-registry,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,id=vkdg-cargo-git,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,id=vkdg-target-${TARGET},target=/src/target,sharing=locked \
    cargo chef cook --profile ${PROFILE} --target ${TARGET} --workspace --recipe-path recipe.json
COPY . .
COPY --from=console /src/apps/console/build ./apps/console/build
RUN --mount=type=cache,id=vkdg-cargo-registry,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,id=vkdg-cargo-git,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,id=vkdg-target-${TARGET},target=/src/target,sharing=locked \
    cargo build --profile ${PROFILE} --target ${TARGET} -p vkdg \
    && install -D target/${TARGET}/${PROFILE}/vkdg /out/vkdg

# ── certs: CA bundle for upstream HTTPS (tiny; keeps `prebuilt` toolchain-free)
FROM alpine:${ALPINE_VERSION} AS certs
RUN apk add --no-cache ca-certificates

# ── runtime base: everything except the binary ──────────────────────────────
FROM scratch AS runtime-base
ARG VCS_REF=unknown
ARG BUILD_DATE=unknown
LABEL org.opencontainers.image.title="vkdg" \
      org.opencontainers.image.description="VKDG AI gateway" \
      org.opencontainers.image.source="https://github.com/vkdprojects/vkdg" \
      org.opencontainers.image.revision="${VCS_REF}" \
      org.opencontainers.image.created="${BUILD_DATE}" \
      org.opencontainers.image.licenses="MIT"
COPY --from=certs /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/
EXPOSE 8080 9090
# No shell in scratch: the binary probes itself.
HEALTHCHECK --interval=15s --timeout=3s --start-period=10s --retries=3 \
    CMD ["/vkdg", "healthcheck"]
ENTRYPOINT ["/vkdg"]
CMD ["serve"]

# ── prebuilt: binary cross-compiled on the host (scripts/ship-dev.sh) ───────
FROM runtime-base AS prebuilt
ARG TARGET
ARG PROFILE
COPY target/${TARGET}/${PROFILE}/vkdg /vkdg

# ── runtime (default): binary compiled in this build ─────────────────────────
FROM runtime-base AS runtime
COPY --from=builder /out/vkdg /vkdg
