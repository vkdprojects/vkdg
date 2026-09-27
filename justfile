# List available recipes
default:
    @just --list

# ── Dev ──────────────────────────────────────────────────────────────────
# Build console then Rust binary (current host, debug)
dev:
    cd apps/console && bun run build
    cargo run -p vkdg -- serve

# Start full dev stack via docker compose
dev-docker:
    docker compose -f deploy/docker-compose.dev.yml up --build

# Run all tests
test:
    cargo test --workspace --tests

# Type-check + lint everything
check:
    cargo check --workspace
    cd apps/console && bun run check

# Format all code
fmt:
    cargo fmt --all
    cd apps/console && bunx prettier --write src/

# ── Build ─────────────────────────────────────────────────────────────────
# Build release binary for current host
build:
    @echo "Building console..."
    cd apps/console && bun run build
    @echo "Building gateway binary..."
    cargo build --release -p vkdg
    @echo "Binary: target/release/vkdg"

# Build for Linux x86_64 — musl static, cross-compiled from any host.
#
# Uses cargo-zigbuild: `rustc` runs natively (including on Apple Silicon) and
# emits x86, so nothing is emulated. Measured 1m39s on an M4 Pro against 14m28s
# compiling inside a Docker image on a 4-core x86 server.
#
# Setup once:  brew install zig && cargo install cargo-zigbuild
#              rustup target add x86_64-unknown-linux-musl
#
# Cross-compile a static amd64 binary from any host (no emulation)
build-linux-amd64:
    cd apps/console && bun run build
    cargo zigbuild --release -p vkdg --target x86_64-unknown-linux-musl
    @echo "Binary: target/x86_64-unknown-linux-musl/release/vkdg"

# Build for Linux ARM64 — musl static
build-linux-arm64:
    cd apps/console && bun run build
    cargo zigbuild --release -p vkdg --target aarch64-unknown-linux-musl
    @echo "Binary: target/aarch64-unknown-linux-musl/release/vkdg"

# Build for macOS Apple Silicon (local)
build-mac-arm64:
    cd apps/console && bun run build
    cargo build --release -p vkdg --target aarch64-apple-darwin
    @echo "Binary: target/aarch64-apple-darwin/release/vkdg"

# ── Docker ────────────────────────────────────────────────────────────────
# Build all Docker images
docker-build:
    docker build -f deploy/Dockerfile.gateway -t vkdg:latest .
    docker build -f deploy/Dockerfile.console -t vkdg-console:latest apps/console

# Tag and push to registry (set REGISTRY env var)
docker-push REGISTRY="ghcr.io/vkdprojects":
    docker tag vkdg:latest {{REGISTRY}}/vkdg:latest
    docker tag vkdg-console:latest {{REGISTRY}}/vkdg-console:latest
    docker push {{REGISTRY}}/vkdg:latest
    docker push {{REGISTRY}}/vkdg-console:latest

# Start production stack locally
prod-local:
    docker compose -f deploy/docker-compose.yml up -d

# ── Dev images (fast loop, no CI) ─────────────────────────────────────────
# Skips the Rust stage inside Docker, so the image build takes seconds rather than
# minutes. Requires the zigbuild setup from `build-linux-amd64`.
#
# Build an amd64 dev image from a host cross-compile
image-dev REGISTRY="ghcr.io/vkdprojects":
    #!/usr/bin/env bash
    set -euo pipefail
    just build-linux-amd64
    SHA=$(git rev-parse --short HEAD)
    DIRTY=$(test -n "$(git status --porcelain)" && echo "-dirty" || echo "")
    TAG="dev-${SHA}${DIRTY}"
    docker build --platform linux/amd64 -f deploy/Dockerfile.dev \
        -t "{{REGISTRY}}/vkdg:${TAG}" -t "{{REGISTRY}}/vkdg:dev" .
    echo "Built {{REGISTRY}}/vkdg:${TAG}"

# Build and push a dev image. Needs `docker login ghcr.io` once.
push-dev REGISTRY="ghcr.io/vkdprojects":
    #!/usr/bin/env bash
    set -euo pipefail
    just image-dev {{REGISTRY}}
    SHA=$(git rev-parse --short HEAD)
    DIRTY=$(test -n "$(git status --porcelain)" && echo "-dirty" || echo "")
    docker push "{{REGISTRY}}/vkdg:dev-${SHA}${DIRTY}"
    docker push "{{REGISTRY}}/vkdg:dev"

# Ship a dev image straight to a host over SSH, no registry involved.
#
# The image is streamed over the SSH connection, so this works against a box that
# cannot pull from a private registry. HOST is an ssh target or alias.
#
#   just ship-dev omni-vixpi
#   just ship-dev omni-vixpi /srv/vkdg     # compose lives elsewhere
#
# Build, stream and deploy a dev image to HOST over SSH
ship-dev HOST DIR="/opt/vkdg":
    #!/usr/bin/env bash
    set -euo pipefail
    just image-dev local
    SHA=$(git rev-parse --short HEAD)
    DIRTY=$(test -n "$(git status --porcelain)" && echo "-dirty" || echo "")
    TAG="dev-${SHA}${DIRTY}"
    echo "Streaming local/vkdg:${TAG} to {{HOST}}…"
    docker save "local/vkdg:${TAG}" | gzip -1 | ssh {{HOST}} 'gunzip | docker load'
    # Keep the previous image tagged so a rollback is one retag away.
    ssh {{HOST}} "cd {{DIR}} && \
        docker tag vkdg-gateway:latest vkdg-gateway:rollback 2>/dev/null || true; \
        docker tag local/vkdg:${TAG} vkdg-gateway:latest && \
        docker compose up -d --force-recreate gateway"
    echo "Deployed ${TAG} to {{HOST}}. Roll back with: just rollback-dev {{HOST}} {{DIR}}"

# Restore the image that was running before the last `ship-dev`.
rollback-dev HOST DIR="/opt/vkdg":
    ssh {{HOST}} "cd {{DIR}} && docker tag vkdg-gateway:rollback vkdg-gateway:latest && docker compose up -d --force-recreate gateway"
    @echo "Rolled back {{HOST}}."

# ── Package / Release ─────────────────────────────────────────────────────
# Create release package for current host (binary + config example + install script)
package:
    @echo "Packaging VKDG..."
    mkdir -p dist
    just build
    cp target/release/vkdg dist/vkdg
    cp config.example.yaml dist/config.example.yaml
    cp install.sh dist/install.sh
    chmod +x dist/vkdg dist/install.sh
    cd dist && tar -czf vkdg-$(uname -s | tr '[:upper:]' '[:lower:]')-$(uname -m).tar.gz vkdg config.example.yaml install.sh
    @echo "Package: dist/vkdg-$(uname -s | tr '[:upper:]' '[:lower:]')-$(uname -m).tar.gz"

# Create release packages for all Linux targets (requires cross)
release-linux:
    just build-linux-amd64
    just build-linux-arm64
    mkdir -p dist
    cp target/x86_64-unknown-linux-musl/release/vkdg dist/vkdg-linux-amd64
    cp target/aarch64-unknown-linux-musl/release/vkdg dist/vkdg-linux-arm64
    cd dist && sha256sum vkdg-linux-* > checksums.txt
    @echo "Artifacts in dist/ with checksums.txt"

# ── Utilities ─────────────────────────────────────────────────────────────
# Validate a config file
config-check PATH:
    cargo run -p vkdg -- config check {{PATH}}

# Show routing decision for a model
config-explain MODEL:
    cargo run -p vkdg -- config explain --model {{MODEL}}

# Run doctor diagnostics
doctor:
    cargo run -p vkdg -- doctor

# Clean all build artifacts
clean:
    cargo clean
    rm -rf apps/console/build dist
    @echo "Cleaned."

# Browser end-to-end tests against the real binary with the console embedded.
e2e:
    cd apps/console && bun run build && bun run e2e
