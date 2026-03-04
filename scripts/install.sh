#!/bin/bash
# One-liner installer for ahma_mcp and ahma_simplify
# Usage: curl -sSf https://raw.githubusercontent.com/paulirotta/ahma/main/scripts/install.sh | bash

set -euo pipefail

# Detect OS and Architecture
OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"

# Map architecture names
case "$ARCH" in
    x86_64) ARCH="x86_64" ;;
    arm64|aarch64) ARCH="arm64" ;;
    *)
        echo "Error: Unsupported architecture: $ARCH"
        exit 1
        ;;
esac

# Map OS names
case "$OS" in
    linux) ;;
    darwin)
        if [ "$ARCH" = "x86_64" ]; then
            echo "Error: macOS Intel (x86_64) is no longer supported. Prebuilt binaries are only available for Apple Silicon (arm64)."
            echo "You can still build from source: cargo build --release"
            exit 1
        fi
        ;;
    *)
        echo "Error: Unsupported operating system: $OS"
        exit 1
        ;;
esac

# Construct platform identifier (e.g., darwin-arm64, linux-x86_64)
# Note: macOS is 'darwin', Linux is 'linux'
PLATFORM="${OS}-${ARCH}"
INSTALL_DIR="$HOME/.local/bin"

echo "Installing Ahma MCP for ${PLATFORM}..."

# Create install directory
mkdir -p "$INSTALL_DIR"

# Fetch latest release data
echo "Fetching latest release info..."
RELEASES_URL="https://api.github.com/repos/paulirotta/ahma/releases/tags/latest"

if command -v curl >/dev/null 2>&1; then
    RELEASE_JSON=$(curl -s "$RELEASES_URL")
elif command -v wget >/dev/null 2>&1; then
    RELEASE_JSON=$(wget -qO- "$RELEASES_URL")
else
    echo "Error: Neither curl nor wget is available."
    exit 1
fi

# Extract download URL for the platform-specific tarball
# Expected asset name format: ahma-release-{platform}.tar.gz
ASSET_NAME="ahma-release-${PLATFORM}.tar.gz"

# Use grep/cut to parse JSON (avoiding jq dependency for maximum portability)
DOWNLOAD_URL=$(echo "$RELEASE_JSON" | grep "browser_download_url" | grep "$ASSET_NAME" | cut -d '"' -f 4 || true)

if [ -z "$DOWNLOAD_URL" ]; then
    echo "Error: Could not find release asset '$ASSET_NAME'."
    echo "Please check https://github.com/paulirotta/ahma/releases for available binaries."
    exit 1
fi

echo "Downloading ${DOWNLOAD_URL}..."

# create temporary directory
TEMP_DIR=$(mktemp -d)
trap 'rm -rf "$TEMP_DIR"' EXIT

# Download and extract
if command -v curl >/dev/null 2>&1; then
    curl -sL "$DOWNLOAD_URL" | tar -xz -C "$TEMP_DIR"
elif command -v wget >/dev/null 2>&1; then
    wget -qO- "$DOWNLOAD_URL" | tar -xz -C "$TEMP_DIR"
fi

# Install binaries
echo "Installing binaries to ${INSTALL_DIR}..."
if [ -f "$TEMP_DIR/ahma_mcp" ]; then
    mv "$TEMP_DIR/ahma_mcp" "$INSTALL_DIR/"
    chmod +x "$INSTALL_DIR/ahma_mcp"
else
    echo "Error: ahma_mcp binary not found in archive"
    exit 1
fi

if [ -f "$TEMP_DIR/ahma_simplify" ]; then
    mv "$TEMP_DIR/ahma_simplify" "$INSTALL_DIR/"
    chmod +x "$INSTALL_DIR/ahma_simplify"
fi

"$INSTALL_DIR/ahma_mcp" --version
if [ -x "$INSTALL_DIR/ahma_simplify" ]; then
    "$INSTALL_DIR/ahma_simplify" --version
fi
echo "Success! Installed ahma_mcp and ahma_simplify to ${INSTALL_DIR}"
echo ""
echo "Please ensure ${INSTALL_DIR} is in your PATH:"
echo "  export PATH=\"\$HOME/.local/bin:\$PATH\""
