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

# Build for Linux x86_64 — musl static (requires `cross` or linux host)
# Install cross: cargo install cross --git https://github.com/cross-rs/cross
build-linux-amd64:
    cd apps/console && bun run build
    cross build --release -p vkdg --target x86_64-unknown-linux-musl
    @echo "Binary: target/x86_64-unknown-linux-musl/release/vkdg"

# Build for Linux ARM64 — musl static
build-linux-arm64:
    cd apps/console && bun run build
    cross build --release -p vkdg --target aarch64-unknown-linux-musl
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
