#!/bin/bash
# One-liner installer for ahma-mcp and ahma-simplify
# Usage: curl -sSf https://raw.githubusercontent.com/paulirotta/ahma/main/scripts/install.sh | bash
#
# Supported platforms:
#   - Linux x86_64 (glibc and musl)
#   - Linux ARM64/aarch64 (glibc and musl)
#   - Linux ARMv7 (Raspberry Pi 2/3)
#   - macOS ARM64 (Apple Silicon)
#
# Environment variables:
#   AHMA_PREFER_MUSL=1    - Force musl binary on Linux (more portable, no glibc dependency)

set -euo pipefail

# Detect OS and Architecture
OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"
LIBC=""

# Detect libc type on Linux
detect_libc() {
    if [ "$OS" != "linux" ]; then
        return
    fi
    
    # Check if we're on a musl-based system (Alpine, Void, etc.)
    if command -v ldd >/dev/null 2>&1; then
        if ldd --version 2>&1 | grep -qi musl; then
            LIBC="musl"
            return
        fi
    fi
    
    # Check for Alpine specifically
    if [ -f /etc/alpine-release ]; then
        LIBC="musl"
        return
    fi
    
    # Default to glibc
    LIBC="glibc"
}

# Map architecture names
case "$ARCH" in
    x86_64) ARCH="x86_64" ;;
    arm64|aarch64) ARCH="arm64" ;;
    armv7l|armv7) ARCH="armv7" ;;
    *)
        echo "Error: Unsupported architecture: $ARCH"
        echo "Supported: x86_64, arm64/aarch64, armv7"
        exit 1
        ;;
esac

# Map OS names and validate combinations
case "$OS" in
    linux)
        detect_libc
        ;;
    darwin)
        if [ "$ARCH" = "x86_64" ]; then
            echo "Error: macOS Intel (x86_64) is no longer supported. Prebuilt binaries are only available for Apple Silicon (arm64)."
            echo "You can still build from source: cargo build --release"
            exit 1
        fi
        ;;
    *)
        echo "Error: Unsupported operating system: $OS"
        echo "Supported: linux, darwin (macOS)"
        exit 1
        ;;
esac

# Construct platform identifier
# Format: {os}-{arch}[-musl]
if [ "$OS" = "linux" ]; then
    # Use musl if detected or explicitly requested
    if [ "${AHMA_PREFER_MUSL:-}" = "1" ] || [ "$LIBC" = "musl" ]; then
        if [ "$ARCH" = "armv7" ]; then
            # armv7 only has glibc build
            PLATFORM="linux-armv7"
            echo "Note: ARMv7 only has glibc build available"
        else
            PLATFORM="linux-${ARCH}-musl"
        fi
    else
        if [ "$ARCH" = "armv7" ]; then
            PLATFORM="linux-armv7"
        else
            PLATFORM="linux-${ARCH}"
        fi
    fi
else
    PLATFORM="${OS}-${ARCH}"
fi

INSTALL_DIR="$HOME/.local/bin"
RELEASE_JSON=""

# Fetch latest release data from GitHub (cached in RELEASE_JSON to avoid duplicate calls)
fetch_release_json() {
    if [ -n "$RELEASE_JSON" ]; then
        return
    fi
    RELEASES_URL="https://api.github.com/repos/paulirotta/ahma/releases/tags/latest"
    if command -v curl >/dev/null 2>&1; then
        RELEASE_JSON=$(curl -s "$RELEASES_URL")
    elif command -v wget >/dev/null 2>&1; then
        RELEASE_JSON=$(wget -qO- "$RELEASES_URL")
    else
        echo "Error: Neither curl nor wget is available."
        exit 1
    fi
}

# Check for existing installation and compare versions
EXISTING_BIN=""
if command -v ahma-mcp >/dev/null 2>&1; then
    EXISTING_BIN="$(command -v ahma-mcp)"
elif [ -x "$INSTALL_DIR/ahma-mcp" ]; then
    EXISTING_BIN="$INSTALL_DIR/ahma-mcp"
fi

if [ -n "$EXISTING_BIN" ]; then
    INSTALLED_VER=$("$EXISTING_BIN" --version 2>&1 | awk '{print $2}' || true)

    echo "Fetching latest release info..."
    fetch_release_json
    LATEST_VER=$(echo "$RELEASE_JSON" | grep '"tag_name"' | head -1 | cut -d'"' -f4 | sed 's/^v//')

    if [ "$INSTALLED_VER" != "$LATEST_VER" ] && [ -n "$LATEST_VER" ]; then
        echo "Upgrading ahma-mcp from ${INSTALLED_VER} to ${LATEST_VER}..."
    else
        echo "Ahma ${INSTALLED_VER} is already installed and up to date."
        echo ""
        echo "  Location : $EXISTING_BIN"
        if command -v ahma-simplify >/dev/null 2>&1; then
            echo "  Simplify : $(command -v ahma-simplify) — $( ahma-simplify --version 2>&1 || true )"
        elif [ -x "$INSTALL_DIR/ahma-simplify" ]; then
            echo "  Simplify : $INSTALL_DIR/ahma-simplify — $( "$INSTALL_DIR/ahma-simplify" --version 2>&1 || true )"
        fi
        echo ""
        if [ -e /dev/tty ]; then
            printf "Reinstall anyway? [y/N]: "
            IFS= read -r CONFIRM < /dev/tty
            case "$CONFIRM" in
                [Yy]*) echo "Reinstalling..." ;;
                *) echo "No changes made."; exit 0 ;;
            esac
        else
            echo "No changes made (non-interactive — same version already installed)."
            exit 0
        fi
    fi
else
    echo "Fetching latest release info..."
    fetch_release_json
fi

echo "Installing Ahma for ${PLATFORM}..."

# Create install directory
mkdir -p "$INSTALL_DIR"

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
if [ -f "$TEMP_DIR/ahma-mcp" ]; then
    mv "$TEMP_DIR/ahma-mcp" "$INSTALL_DIR/"
    chmod +x "$INSTALL_DIR/ahma-mcp"
else
    echo "Error: ahma-mcp binary not found in archive"
    exit 1
fi

if [ -f "$TEMP_DIR/ahma-simplify" ]; then
    mv "$TEMP_DIR/ahma-simplify" "$INSTALL_DIR/"
    chmod +x "$INSTALL_DIR/ahma-simplify"
fi

"$INSTALL_DIR/ahma-mcp" --version
if [ -x "$INSTALL_DIR/ahma-simplify" ]; then
    "$INSTALL_DIR/ahma-simplify" --version
fi
echo "Success! Installed ahma-mcp and ahma-simplify to ${INSTALL_DIR}"
echo ""
echo "Please ensure ${INSTALL_DIR} is in your PATH:"
echo "  export PATH=\"\$HOME/.local/bin:\$PATH\""

# ─────────────────────────────────────────────────────────────────────────────
# MCP Server Setup Wizard
# Configures the global mcp.json for VS Code, Claude Code, Cursor, Antigravity
# ─────────────────────────────────────────────────────────────────────────────

# Test whether comma-separated list $1 contains the exact number $2
_ahma_list_has() {
    # Accept flexible formats: "1,2,4" or "124" or "1 2 4" or mixed "1, 2,4" etc.
    # Normalize: remove all non-digit characters except newlines, then split into individual digits
    local normalized
    normalized=$(echo "$1" | tr -cd '0-9\n' | grep -o .)
    echo "$normalized" | grep -q "^${2}$"
}

# Write a complete new MCP config file for the given parameters.
# Args: servers_key  transport  platform_type("standard"|"antigravity")
# Output: formatted JSON on stdout
_ahma_new_file_json() {
    local SKEY="$1"
    local TRANS="$2"
    local PTYPE="$3"

    if [ "$TRANS" = "http" ]; then
        cat <<EOF
{
    "$SKEY": {
        "Ahma": {
            "type": "http",
            "url": "http://localhost:3000/mcp"
        }
    }
}
EOF
    elif [ "$PTYPE" = "antigravity" ]; then
        cat <<EOF
{
    "$SKEY": {
        "Ahma": {
            "command": "bash",
            "args": [
                "-c",
                "AHMA_SANDBOX_SCOPE=\$HOME AHMA_TMP_ACCESS=1 AHMA_LOG_MONITOR=1 ahma-mcp serve stdio --tools rust,simplify"
            ]
        }
    }
}
EOF
    else
        cat <<EOF
{
    "$SKEY": {
        "Ahma": {
            "type": "stdio",
            "command": "ahma-mcp",
            "args": [
                "serve",
                "stdio",
                "--tools",
                "rust,simplify"
            ],
            "env": {
                "AHMA_TMP_ACCESS": "1",
                "AHMA_LOG_MONITOR": "1"
            }
        }
    }
}
EOF
    fi
}

# Configure a single platform's global MCP config file.
# Reads globals: AHMA_TRANSPORT, AHMA_PY_SCRIPT
# Writes global: AHMA_CONFIGURED_TOOLS (appends "|display_name")
# Args: display_name  config_path  servers_key  platform_type
_ahma_configure_platform() {
    local DISPLAY="$1"
    local CPATH="$2"
    local SKEY="$3"
    local PTYPE="$4"

    # Compact entry JSON (passed to Python for merging into existing files)
    local ENTRY
    if [ "$AHMA_TRANSPORT" = "http" ]; then
        ENTRY='{"type":"http","url":"http://localhost:3000/mcp"}'
    elif [ "$PTYPE" = "antigravity" ]; then
        ENTRY='{"command":"bash","args":["-c","AHMA_SANDBOX_SCOPE=$HOME AHMA_TMP_ACCESS=1 AHMA_LOG_MONITOR=1 ahma-mcp serve stdio --tools rust,simplify"]}'
    else
        ENTRY='{"type":"stdio","command":"ahma-mcp","args":["serve","stdio","--tools","rust,simplify"],"env":{"AHMA_TMP_ACCESS":"1","AHMA_LOG_MONITOR":"1"}}'
    fi

    echo ""
    echo "  ─── ${DISPLAY} ───────────────────────────────────────"
    echo "  Config: ${CPATH}"

    local CONFIRM
    if [ ! -f "$CPATH" ]; then
        # New file — generate content without python3
        local PROPOSED
        PROPOSED=$(_ahma_new_file_json "$SKEY" "$AHMA_TRANSPORT" "$PTYPE")

        echo ""
        echo "  File does not exist. Proposed new file:"
        echo ""
        echo "$PROPOSED" | sed 's/^/    /'
        echo ""
        printf "  Create this file? [Y/n]: "
        IFS= read -r CONFIRM < /dev/tty
        case "$CONFIRM" in
            [Nn]*) echo "  Skipped."; return 0 ;;
        esac
        mkdir -p "$(dirname "$CPATH")"
        printf '%s\n' "$PROPOSED" > "$CPATH"
        echo "  ✓ Created ${CPATH}:"
        echo ""
        echo "$PROPOSED" | sed 's/^/    /'
        echo ""
        AHMA_CONFIGURED_TOOLS="${AHMA_CONFIGURED_TOOLS}|${DISPLAY}"

    else
        # Existing file — use python3 to merge
        if ! command -v python3 >/dev/null 2>&1; then
            echo ""
            echo "  File exists but python3 is not available for automatic merging."
            echo "  Please add the following entry manually under \"${SKEY}\" in:"
            echo "  ${CPATH}"
            echo ""
            echo "    \"Ahma\": ${ENTRY}"
            return 0
        fi

        local PROPOSED
        PROPOSED=$(python3 "$AHMA_PY_SCRIPT" "$CPATH" "$SKEY" "$ENTRY" 2>/dev/null)

        if [ -z "$PROPOSED" ]; then
            echo ""
            echo "  Could not parse existing file. Please add the entry manually:"
            echo "  ${CPATH} → under \"${SKEY}\":"
            echo "    \"Ahma\": ${ENTRY}"
            return 0
        fi

        echo ""
        echo "  Proposed file after adding Ahma entry:"
        echo ""
        echo "$PROPOSED" | sed 's/^/    /'
        echo ""
        printf "  Update this file? [Y/n]: "
        IFS= read -r CONFIRM < /dev/tty
        case "$CONFIRM" in
            [Nn]*) echo "  Skipped."; return 0 ;;
        esac
        printf '%s\n' "$PROPOSED" > "$CPATH"
        echo "  ✓ Updated ${CPATH}:"
        echo ""
        echo "$PROPOSED" | sed 's/^/    /'
        echo ""
        AHMA_CONFIGURED_TOOLS="${AHMA_CONFIGURED_TOOLS}|${DISPLAY}"
    fi
}

setup_mcp() {
    set +e  # Wizard exit codes must not abort the script

    # Skip if there is no terminal to interact with (e.g. CI, non-interactive pipe)
    if [ ! -e /dev/tty ]; then
        echo ""
        echo "Tip: Run 'ahma-mcp serve stdio --help' to learn about MCP server modes."
        echo "     See https://github.com/paulirotta/ahma for mcp.json setup examples."
        return 0
    fi

    echo ""
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "  MCP Server Setup"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo ""
    printf "Configure ahma-mcp as a global MCP server for your AI tools? [Y/n]: "
    local CHOICE
    IFS= read -r CHOICE < /dev/tty
    case "$CHOICE" in
        [Nn]*) return 0 ;;
    esac

    # ── Compute platform-specific VS Code user config path ─────────────────
    local VSCODE_MCP_PATH
    if [ "$OS" = "darwin" ]; then
        VSCODE_MCP_PATH="${HOME}/Library/Application Support/Code/User/mcp.json"
    else
        VSCODE_MCP_PATH="${HOME}/.config/Code/User/mcp.json"
    fi

    # ── Step 1: Platform selection ──────────────────────────────────────────
    echo ""
    echo "Select platforms to configure (comma-separated numbers, or Enter for all):"
    echo "  1) VS Code       (${VSCODE_MCP_PATH})"
    echo "  2) Claude Code   (${HOME}/.claude.json)"
    echo "  3) Cursor        (${HOME}/.cursor/mcp.json)"
    echo "  4) Antigravity   (${HOME}/.antigravity/mcp.json)"
    echo ""
    printf "  Selection [default: 1,2,3,4 — all]: "
    local PLATFORMS
    IFS= read -r PLATFORMS < /dev/tty
    case "$PLATFORMS" in
        ""|all|ALL|All) PLATFORMS="1,2,3,4" ;;
    esac

    # ── Step 2: Transport selection ─────────────────────────────────────────
    echo ""
    echo "Choose how your AI tools connect to ahma-mcp:"
    echo ""
    echo "  1) stdio  (recommended for most users)"
    echo "     Each AI tool starts its own private ahma-mcp instance automatically"
    echo "     when you open a project. No extra steps needed — it just works."
    echo ""
    echo "  2) http   (one shared server, better visibility)"
    echo "     You run 'ahma-mcp serve http --tools rust,simplify' in a terminal"
    echo "     before opening your AI tools. All tools connect to one running"
    echo "     instance, so you can watch what ahma is doing in real time."
    echo "     Best if you use multiple AI tools simultaneously."
    echo ""
    printf "  Mode [1=stdio or 2=http, default 1]: "
    local TSELECT
    IFS= read -r TSELECT < /dev/tty
    case "$TSELECT" in
        2) AHMA_TRANSPORT="http" ;;
        *) AHMA_TRANSPORT="stdio" ;;
    esac

    # ── Python helper for merging into existing JSON files ──────────────────
    AHMA_PY_SCRIPT=$(mktemp /tmp/ahma_mcp_XXXXXX.py)
    cat > "$AHMA_PY_SCRIPT" << 'PYEOF'
#!/usr/bin/env python3
"""Merge an Ahma entry into an MCP config JSON file.
Usage: script.py <config_path> <servers_key> <entry_json>
  config_path:  path to existing JSON file to merge into
  servers_key:  'servers' (VS Code) or 'mcpServers' (Cursor, Claude, Antigravity)
  entry_json:   compact JSON string for the value under "Ahma"
"""
import sys, json

config_path = sys.argv[1]
servers_key = sys.argv[2]
entry_str   = sys.argv[3]

try:
    with open(config_path, 'r') as f:
        config = json.load(f)
except (FileNotFoundError, ValueError):
    config = {}

entry = json.loads(entry_str)

if servers_key not in config or not isinstance(config.get(servers_key), dict):
    config[servers_key] = {}

config[servers_key]['Ahma'] = entry
print(json.dumps(config, indent=4))
PYEOF

    # ── Configure each selected platform ───────────────────────────────────
    AHMA_CONFIGURED_TOOLS=""

    if _ahma_list_has "$PLATFORMS" 1; then
        _ahma_configure_platform "VS Code"     "${VSCODE_MCP_PATH}"           "servers"    "standard"
    fi
    if _ahma_list_has "$PLATFORMS" 2; then
        _ahma_configure_platform "Claude Code" "${HOME}/.claude.json"          "mcpServers" "standard"
    fi
    if _ahma_list_has "$PLATFORMS" 3; then
        _ahma_configure_platform "Cursor"      "${HOME}/.cursor/mcp.json"     "mcpServers" "standard"
    fi
    if _ahma_list_has "$PLATFORMS" 4; then
        _ahma_configure_platform "Antigravity" "${HOME}/.antigravity/mcp.json" "mcpServers" "antigravity"
    fi

    rm -f "$AHMA_PY_SCRIPT"

    # ── Summary ─────────────────────────────────────────────────────────────
    echo ""
    if [ -n "$AHMA_CONFIGURED_TOOLS" ]; then
        echo "✓ MCP setup complete! Restart these tools for changes to take effect:"
        echo "$AHMA_CONFIGURED_TOOLS" | tr '|' '\n' | while IFS= read -r TOOL; do
            [ -n "$TOOL" ] && echo "    - ${TOOL}"
        done
        if [ "$AHMA_TRANSPORT" = "http" ]; then
            echo ""
            echo "  Before opening your AI tools, start the ahma HTTP server:"
            echo "    ahma-mcp serve http --tools rust,simplify"
        fi
    else
        echo "No MCP configurations were changed."
    fi
    echo ""
}

# Run the MCP setup wizard
setup_mcp
