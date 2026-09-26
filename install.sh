#!/bin/sh
# VKDG install script
# Usage: curl -fsSL https://raw.githubusercontent.com/vkdprojects/vkdg/main/install.sh | sh
#   or:  curl -fsSL https://raw.githubusercontent.com/vkdprojects/vkdg/main/install.sh | VKDG_VERSION=0.2.0 sh
set -eu

VKDG_VERSION="${VKDG_VERSION:-latest}"
INSTALL_DIR="${INSTALL_DIR:-/usr/local/bin}"
GITHUB_REPO="vkdprojects/vkdg"

main() {
    # Detect OS and architecture
    OS="$(uname -s)"
    ARCH="$(uname -m)"

    case "$OS" in
        Linux)
            case "$ARCH" in
                x86_64)  TARGET="x86_64-unknown-linux-musl" ;;
                aarch64|arm64) TARGET="aarch64-unknown-linux-musl" ;;
                *) error "Unsupported architecture: $ARCH" ;;
            esac
            ;;
        Darwin)
            case "$ARCH" in
                arm64)   TARGET="aarch64-apple-darwin" ;;
                x86_64)  TARGET="x86_64-apple-darwin" ;;
                *) error "Unsupported architecture: $ARCH" ;;
            esac
            ;;
        *)
            error "Unsupported OS: $OS. See https://github.com/$GITHUB_REPO for manual install."
            ;;
    esac

    say "Installing VKDG for $TARGET..."

    # Resolve version
    if [ "$VKDG_VERSION" = "latest" ]; then
        VKDG_VERSION="$(resolve_latest_version)"
    fi
    say "Version: $VKDG_VERSION"

    # Build download URL
    TARBALL="vkdg-${VKDG_VERSION}-${TARGET}.tar.gz"
    BASE_URL="https://github.com/${GITHUB_REPO}/releases/download/${VKDG_VERSION}"
    TARBALL_URL="${BASE_URL}/${TARBALL}"
    CHECKSUMS_URL="${BASE_URL}/checksums.txt"

    # Download to temp dir
    TMP_DIR="$(mktemp -d)"
    trap 'rm -rf "$TMP_DIR"' EXIT

    say "Downloading $TARBALL..."
    download "$TARBALL_URL" "$TMP_DIR/$TARBALL"
    download "$CHECKSUMS_URL" "$TMP_DIR/checksums.txt"

    # Verify checksum
    say "Verifying checksum..."
    verify_checksum "$TMP_DIR/$TARBALL" "$TMP_DIR/checksums.txt"

    # Extract and install
    tar -xzf "$TMP_DIR/$TARBALL" -C "$TMP_DIR"

    if [ ! -w "$INSTALL_DIR" ]; then
        say "Need sudo to install to $INSTALL_DIR"
        SUDO="sudo"
    else
        SUDO=""
    fi

    $SUDO install -m 755 "$TMP_DIR/vkdg" "$INSTALL_DIR/vkdg"
    say "Installed: $INSTALL_DIR/vkdg"

    # Post-install
    if [ "$OS" = "Linux" ] && command -v systemctl >/dev/null 2>&1; then
        say ""
        say "Run 'vkdg install' to set up as a systemd service, or:"
        say "  vkdg setup     — first-run configuration wizard"
        say "  vkdg serve     — start the gateway directly"
    else
        say ""
        say "Run 'vkdg setup' for the first-run configuration wizard."
    fi

    say ""
    say "VKDG $VKDG_VERSION installed successfully."
    say "Documentation: https://github.com/$GITHUB_REPO"
}

resolve_latest_version() {
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "https://api.github.com/repos/${GITHUB_REPO}/releases/latest" \
            | grep '"tag_name"' | sed 's/.*"tag_name": *"\([^"]*\)".*/\1/'
    elif command -v wget >/dev/null 2>&1; then
        wget -qO- "https://api.github.com/repos/${GITHUB_REPO}/releases/latest" \
            | grep '"tag_name"' | sed 's/.*"tag_name": *"\([^"]*\)".*/\1/'
    else
        error "Neither curl nor wget found. Install one and retry."
    fi
}

download() {
    URL="$1"
    DEST="$2"
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL --progress-bar "$URL" -o "$DEST" || error "Download failed: $URL"
    elif command -v wget >/dev/null 2>&1; then
        wget -q --show-progress "$URL" -O "$DEST" || error "Download failed: $URL"
    else
        error "Neither curl nor wget found."
    fi
}

verify_checksum() {
    FILE="$1"
    SUMS="$2"
    BASENAME="$(basename "$FILE")"

    if command -v sha256sum >/dev/null 2>&1; then
        EXPECTED="$(grep "$BASENAME" "$SUMS" | cut -d' ' -f1)"
        ACTUAL="$(sha256sum "$FILE" | cut -d' ' -f1)"
    elif command -v shasum >/dev/null 2>&1; then
        EXPECTED="$(grep "$BASENAME" "$SUMS" | cut -d' ' -f1)"
        ACTUAL="$(shasum -a 256 "$FILE" | cut -d' ' -f1)"
    else
        say "Warning: sha256sum/shasum not found, skipping checksum verification."
        return
    fi

    if [ "$EXPECTED" != "$ACTUAL" ]; then
        error "Checksum mismatch for $BASENAME. Expected: $EXPECTED, got: $ACTUAL"
    fi
    say "Checksum verified."
}

say() { printf "vkdg: %s\n" "$*"; }
error() { say "error: $*" >&2; exit 1; }

main "$@"
