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

# Skill version — keep in sync with [workspace.package] version in Cargo.toml.
# CI guardrails verify this matches. Bump alongside Cargo.toml on every release.
AHMA_VERSION="0.5.6"

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
                "AHMA_SANDBOX_SCOPE=\$HOME ahma-mcp serve stdio --tools rust,simplify --tmp --log-monitor"
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
                "rust,simplify",
                "--tmp",
                "--log-monitor"
            ]
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
        ENTRY='{"command":"bash","args":["-c","AHMA_SANDBOX_SCOPE=$HOME ahma-mcp serve stdio --tools rust,simplify --tmp --log-monitor"]}'
    else
        ENTRY='{"type":"stdio","command":"ahma-mcp","args":["serve","stdio","--tools","rust,simplify","--tmp","--log-monitor"]}'
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

# ── Generate TOML config for Codex CLI ─────────────────────────────────────
# Codex uses ~/.codex/config.toml with TOML [mcp_servers.<name>] tables.
# Args: transport ("stdio"|"http")
# Output: TOML block on stdout
_ahma_new_codex_toml() {
    local TRANS="$1"
    if [ "$TRANS" = "http" ]; then
        cat <<'EOF'
[mcp_servers.Ahma]
url = "http://localhost:3000/mcp"
EOF
    else
        cat <<'EOF'
[mcp_servers.Ahma]
command = "ahma-mcp"
args = ["serve", "stdio", "--tools", "rust,simplify", "--tmp", "--log-monitor"]
EOF
    fi
}

# Configure Codex CLI's ~/.codex/config.toml (TOML format, not JSON).
# Reads global: AHMA_TRANSPORT
# Writes global: AHMA_CONFIGURED_TOOLS (appends "|Codex CLI")
_ahma_configure_codex() {
    local CPATH="${HOME}/.codex/config.toml"
    local TOML_ENTRY
    TOML_ENTRY=$(_ahma_new_codex_toml "$AHMA_TRANSPORT")

    echo ""
    echo "  ─── Codex CLI ──────────────────────────────────────────"
    echo "  Config: ${CPATH}"

    local CONFIRM
    if [ ! -f "$CPATH" ]; then
        # New file
        echo ""
        echo "  File does not exist. Proposed new file:"
        echo ""
        echo "$TOML_ENTRY" | sed 's/^/    /'
        echo ""
        printf "  Create this file? [Y/n]: "
        IFS= read -r CONFIRM < /dev/tty
        case "$CONFIRM" in
            [Nn]*) echo "  Skipped."; return 0 ;;
        esac
        mkdir -p "$(dirname "$CPATH")"
        printf '%s\n' "$TOML_ENTRY" > "$CPATH"
        echo "  ✓ Created ${CPATH}:"
        echo ""
        echo "$TOML_ENTRY" | sed 's/^/    /'
        echo ""
        AHMA_CONFIGURED_TOOLS="${AHMA_CONFIGURED_TOOLS}|Codex CLI"

    elif grep -q "^\[mcp_servers\.Ahma\]" "$CPATH" 2>/dev/null; then
        # Section already exists — propose replacement
        echo ""
        echo "  [mcp_servers.Ahma] entry already exists in ${CPATH}."
        echo ""
        echo "  Proposed replacement:"
        echo ""
        echo "$TOML_ENTRY" | sed 's/^/    /'
        echo ""
        printf "  Replace existing entry? [y/N]: "
        IFS= read -r CONFIRM < /dev/tty
        case "$CONFIRM" in
            [Yy]*)
                # Remove old section with awk, then append new entry
                awk '
                    skip && substr($0, 1, 1) == "[" { skip = 0 }
                    index($0, "[mcp_servers.Ahma]") == 1 { skip = 1; next }
                    !skip { print }
                ' "$CPATH" > "${CPATH}.tmp" && mv "${CPATH}.tmp" "$CPATH"
                printf '\n%s\n' "$TOML_ENTRY" >> "$CPATH"
                echo "  ✓ Updated ${CPATH}"
                AHMA_CONFIGURED_TOOLS="${AHMA_CONFIGURED_TOOLS}|Codex CLI"
                ;;
            *)
                echo "  Skipped."
                ;;
        esac

    else
        # File exists but no [mcp_servers.Ahma] section — append
        echo ""
        echo "  Appending [mcp_servers.Ahma] to ${CPATH}:"
        echo ""
        echo "$TOML_ENTRY" | sed 's/^/    /'
        echo ""
        printf "  Add to file? [Y/n]: "
        IFS= read -r CONFIRM < /dev/tty
        case "$CONFIRM" in
            [Nn]*) echo "  Skipped."; return 0 ;;
        esac
        printf '\n%s\n' "$TOML_ENTRY" >> "$CPATH"
        echo "  ✓ Updated ${CPATH}"
        AHMA_CONFIGURED_TOOLS="${AHMA_CONFIGURED_TOOLS}|Codex CLI"
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
    echo "  5) Codex CLI     (${HOME}/.codex/config.toml)"
    echo ""
    printf "  Selection [default: 1,2,3,4,5 — all]: "
    local PLATFORMS
    IFS= read -r PLATFORMS < /dev/tty
    case "$PLATFORMS" in
        ""|all|ALL|All) PLATFORMS="1,2,3,4,5" ;;
    esac

    # Confirm platform selection
    echo ""
    echo "✓ Selected platforms:"
    [ "$(_ahma_list_has "$PLATFORMS" 1 && echo y)" = "y" ] && echo "    • VS Code"
    [ "$(_ahma_list_has "$PLATFORMS" 2 && echo y)" = "y" ] && echo "    • Claude Code"
    [ "$(_ahma_list_has "$PLATFORMS" 3 && echo y)" = "y" ] && echo "    • Cursor"
    [ "$(_ahma_list_has "$PLATFORMS" 4 && echo y)" = "y" ] && echo "    • Antigravity"
    [ "$(_ahma_list_has "$PLATFORMS" 5 && echo y)" = "y" ] && echo "    • Codex CLI"

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

    # Confirm transport selection
    echo ""
    if [ "$AHMA_TRANSPORT" = "http" ]; then
        echo "✓ Transport mode: http (one shared server)"
    else
        echo "✓ Transport mode: stdio (recommended for most users)"
    fi

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
    if _ahma_list_has "$PLATFORMS" 5; then
        _ahma_configure_codex
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

# Skill Setup Wizard
# Installs Ahma agent skills to ~/.agents/skills/
#   ahma          — comprehensive usage guide (sandboxed_shell, bundles, sandbox, etc.)
#   ahma-simplify — code complexity analysis and hotspot fixing workflow
# Compatible with VS Code (GitHub Copilot), Cursor, and Claude Code — all index .agents/skills/
# ─────────────────────────────────────────────────────────────────────────────

_ahma_main_skill_content() {
    cat << 'AHMA_SKILL_EOF'
---
name: ahma
version: __AHMA_VERSION__
author: Paul Houghton
description: >
  Comprehensive guide for using Ahma (ahma-mcp) as an AI agent. USE THIS SKILL when you need
  to understand how to run tools, activate bundles, use the sandbox, monitor logs, author custom
  tools, or configure ahma-mcp. Trigger phrases: "use ahma", "run with ahma", "ahma tool",
  "activate bundle", "sandboxed_shell", "ahma async", "ahma serve", "mcp.json ahma",
  "ahma sandbox", "ahma livelog", "ahma monitor", "custom tool .ahma", "ahma-mcp", "await tool",
  "cancel operation", "tool bundle", "progressive disclosure", "activate_tools".
user-invocable: true
---

<!-- version: __AHMA_VERSION__ | author: Paul Houghton -->

# Ahma Skill — Comprehensive AI Usage Guide

**Ahma** (`ahma-mcp`) is a kernel-sandboxed MCP server that wraps command-line tools for AI
agents. It exposes shell tools (cargo, git, python, file utilities, etc.) as MCP tools with
kernel-level filesystem sandboxing, async execution, and live log monitoring.

---

## Quick Start: mcp.json Setup

MCP stdio servers auto-start when the IDE needs tools — the only step is getting
the config in place. There are several approaches, from zero-friction to global:

### 1. Commit to the repo (recommended — zero setup for teammates)

**The Ahma project already provides `.vscode/mcp.json` with three configurations to try:**

- `ahma` — stdio mode (recommended, automatic per-client instances)
- `ahma-http` — shared HTTP server on port 3000 (run `ahma-mcp serve http --tools rust,git,fileutils --tmp --log-monitor`)
- `ahma-unix` — shared HTTP server over Unix socket (run `ahma-mcp serve http --socket-path /tmp/ahma-mcp.sock --tools rust,git,fileutils --tmp --log-monitor`)

You can copy or customize this for your own projects. Create `.vscode/mcp.json` in your project root and commit it. Every VS Code user
who opens the project gets Ahma configured automatically (prompted to trust once):

```json
{
  "servers": {
    "ahma": {
      "type": "stdio",
      "command": "ahma-mcp",
      "args": ["serve", "stdio", "--tools", "rust,git,fileutils", "--tmp", "--log-monitor"]
    }
  }
}
```

### 2. User-level config (available in all workspaces)

| IDE | Config file |
|-----|-------------|
| **VS Code** | `~/.config/Code/User/mcp.json` (or run `MCP: Open User Configuration`) |
| **Cursor** | `~/.cursor/mcp.json` |
| **Claude Code** | `~/.claude.json` → `"mcpServers"` key |
| **Claude Desktop** | `~/Library/Application Support/Claude/claude_desktop_config.json` |
| **Codex CLI** | `~/.codex/config.toml` → `[mcp_servers.Ahma]` |

Same JSON structure as above. The server starts automatically when chat is opened.

### 3. VS Code auto-start setting

Enable globally in VS Code settings:
```json
{ "chat.mcp.autoStart": true }
```
This auto-(re)starts MCP servers when configuration changes are detected, so
Ahma spins up as soon as VS Code sees the config — no need to open chat first.

### 4. VS Code sandbox integration (auto-approve tool calls)

VS Code provides its own sandbox for MCP stdio servers. When enabled, tool calls
are auto-approved (no confirmation dialogs). Pairs well with Ahma's kernel sandbox:

```json
{
  "servers": {
    "ahma": {
      "type": "stdio",
      "command": "ahma-mcp",
      "args": ["serve", "stdio", "--tools", "rust,git,fileutils", "--tmp"],
      "sandboxEnabled": true,
      "sandbox": {
        "filesystem": { "allowWrite": ["${workspaceFolder}"] }
      }
    }
  }
}
```

### 5. Install script (multi-IDE setup)

Run the install script to configure Ahma across all supported IDEs at once:
```bash
curl -fsSL https://raw.githubusercontent.com/paulirotta/ahma/main/scripts/install.sh | bash
```

### 6. Dev containers

Add to `.devcontainer/devcontainer.json` for Codespaces / container users:
```json
{
  "features": {},
  "postCreateCommand": "cargo install ahma-mcp",
  "customizations": {
    "vscode": {
      "settings": { "chat.mcp.autoStart": true }
    }
  }
}
```
Combine with `.vscode/mcp.json` committed to the repo.

### AI Agent: Helping Users Set Up

If a user asks you to use Ahma but it isn't configured, help them by:
1. Check if `.vscode/mcp.json` exists — if not, offer to create it
2. Ask which bundles they need (rust, git, python, etc.)
3. Create the config and tell them to reload the window (`Developer: Reload Window`)

---

## Tool Bundles & Progressive Disclosure

By default, Ahma hides bundled tools to save AI context ("progressive disclosure"). You first
see only: `sandboxed_shell`, `status`, `await`, `cancel`, and `activate_tools`.

### Discovering and Activating Bundles

```
activate_tools(action="list")           # See available bundles
activate_tools(action="reveal", bundle="rust")   # Unlock Cargo tools
activate_tools(action="reveal", bundle="git")    # Unlock Git tools
```

### Available Bundles

| Bundle | Activate with | Key tools | When to use |
|--------|--------------|-----------|-------------|
| `rust` | `--tools rust` | cargo build/test/clippy/fmt/nextest/add | Rust/Cargo projects |
| `git` | `--tools git` | git status/commit/push/log/diff | Version control |
| `fileutils` | `--tools fileutils` | ls, cp, mv, rm, grep, find, diff | File operations |
| `python` | `--tools python` | python script execution | Python projects |
| `kotlin` | `--tools kotlin` | gradle build/test/lint | Android/Kotlin |
| `github` | `--tools github` | gh pr/issue/run/release | GitHub CLI operations |
| `simplify` | `--tools simplify` | Code complexity analysis | Code quality work |

**Load bundles but keep hidden** (default — LLM calls `activate_tools` to reveal):
```json
"args": ["serve", "stdio", "--tools", "rust,git,fileutils"]
```

**Auto-reveal at startup** (recommended — all `--tools` bundles visible immediately):
```json
"args": ["serve", "stdio", "--tools", "rust,git,fileutils", "--auto-reveal"]
```

---

## Built-in Tools (Always Available)

### `sandboxed_shell` — Run any shell command

```
sandboxed_shell(
  command="cargo build --release",
  working_directory="/path/to/project",
  timeout_seconds=300
)
```

- Runs inside the kernel sandbox (cannot write outside project scope)
- Supports pipes, redirects, variables, multi-command strings
- `monitor_level` ("error"/"warn"/"info") and `monitor_stream` ("stderr"/"stdout"/"both")
  trigger LLM log alerts when issues are detected

### `status` — Check async operation progress

```
status(operation_id="op_abc123")
```

Returns current state: `running`, `complete`, `failed`, `cancelled`, or `timeout`.
Non-blocking — safe to call repeatedly.

### `await` — Wait for an async operation to finish

```
await(operation_id="op_abc123", timeout_seconds=60)
```

Blocks until the operation completes or times out. Use sparingly — prefer `status` polling
when you want to continue other work in parallel.

### `cancel` — Cancel a running operation

```
cancel(operation_id="op_abc123")
```

Sends cancellation signal. The process is terminated and resources are freed.

---

## Async-First Workflow

Most tools run **asynchronously** by default — they return an `operation_id` immediately.

```
# 1. Start a long operation
result = cargo_build(subcommand="build")
# → { "operation_id": "op_abc123", "status": "started" }

# 2. Check progress (non-blocking)
status(operation_id="op_abc123")
# → { "status": "running", "output_so_far": "..." }

# 3. Wait for completion when needed
await(operation_id="op_abc123", timeout_seconds=120)
# → { "status": "complete", "exit_code": 0, "output": "..." }

# Or: cancel if taking too long
cancel(operation_id="op_abc123")
```

**Force synchronous** for state-modifying commands (e.g., `cargo add`):
- Set `"synchronous": true` in the tool's MTDF JSON, or
- Start server with `--sync` flag, or set `AHMA_SYNC=1`

---

## Sandbox — Filesystem Security

Ahma enforces **kernel-level** filesystem boundaries set once at startup.

### Scope Rules
- **STDIO mode**: Scope = `cwd` from mcp.json (usually `${workspaceFolder}`)
- **HTTP mode**: Scope = workspace roots from MCP `roots/list` response
- **Override**: `AHMA_SANDBOX_SCOPE=/path/a:/path/b` (colon-separated on Unix)

### Temp Directory
```json
"args": ["serve", "stdio", "--tmp"]   # or AHMA_TMP_ACCESS=1
```
Adds `/tmp` (or `%TEMP%` on Windows) to the scope. Required for compilers, build tools.

### Nested Sandbox Detection
If running inside Cursor, VS Code, or Docker, Ahma auto-disables its internal sandbox
(outer sandbox already provides protection). Override: `AHMA_DISABLE_SANDBOX=1` to
suppress the warning message.

### Platform Enforcement
- **Linux**: Landlock LSM (requires kernel 5.13+)
- **macOS**: `sandbox-exec` (Seatbelt, built-in)
- **Windows**: Job Objects + AppContainer (in progress)

---

## Live Log Monitoring

Two flavors of log monitoring:

### 1. `--log-monitor` flag — Monitor Ahma's own server logs

```json
"args": ["serve", "stdio", "--log-monitor"]
```

Tails Ahma's rolling log files (`./log/ahma_mcp.log.*`), analyzes chunks with an LLM, and
pushes `LogAlert` MCP progress notifications when errors or anomalies are detected.

Configure minimum seconds between alerts: `--monitor-rate-limit 60` (default 60).

### 2. `livelog` tool type — Monitor any streaming command

For tools defined in `.ahma/` with `"tool_type": "livelog"`:
```json
{
  "name": "logcat",
  "tool_type": "livelog",
  "livelog": {
    "source_command": "adb",
    "source_args": ["-d", "logcat", "-v", "threadtime"],
    "detection_prompt": "Look for crashes, ANR errors, or exceptions.",
    "llm_provider": { "base_url": "http://localhost:11434/v1", "model": "llama3.2" },
    "chunk_max_lines": 50,
    "chunk_max_seconds": 30,
    "cooldown_seconds": 60
  }
}
```

Built-in examples (activate with `--tools`): `android-logcat`, `rust-log-monitor`.

---

## Custom Tools — `.ahma/` Directory

Place `*.json` files in `.ahma/` at the project root to define project-local tools.
Ahma auto-detects and loads them at startup. Override path: `AHMA_TOOLS_DIR=/path/to/dir`.

### Minimal MTDF tool definition

```json
{
  "name": "deploy",
  "description": "Deploy the application to staging",
  "command": "scripts/deploy.sh",
  "enabled": true,
  "synchronous": true
}
```

### With subcommands and options

```json
{
  "name": "myapp",
  "description": "Build and run the application",
  "command": "python",
  "subcommand": [
    {
      "name": "build",
      "description": "Build the app",
      "options": [
        { "name": "release", "type": "boolean", "description": "Optimized build" }
      ]
    },
    {
      "name": "run",
      "description": "Run the app",
      "options": [
        { "name": "port", "type": "integer", "description": "Port number", "default": 8080 }
      ]
    }
  ]
}
```

### Sequence tools (multi-step workflows)

```json
{
  "name": "check",
  "description": "Format, lint, and test in one command",
  "command": "sequence",
  "sequences": [
    { "tool": "cargo", "subcommand": "fmt", "args": { "all": true } },
    { "tool": "cargo", "subcommand": "clippy", "args": {} },
    { "tool": "cargo", "subcommand": "nextest_run", "args": {} }
  ]
}
```

Validate tool configs: `ahma-mcp tool validate .ahma/`

Hot-reload while authoring (dev only): `AHMA_HOT_RELOAD=1 ahma-mcp serve stdio`

---

## Key Environment Variables

| Variable | Default | Purpose |
|----------|---------|---------|
| `AHMA_TOOLS_DIR` | `.ahma/` | Custom tools directory path |
| `AHMA_TIMEOUT` | `360` | Default tool timeout (seconds) |
| `AHMA_SYNC` | off | Force all tools synchronous |
| `AHMA_HOT_RELOAD` | off | Reload tool JSON on file change (dev only) |
| `AHMA_DISABLE_SANDBOX` | off | Disable kernel sandbox (UNSAFE) |
| `AHMA_SANDBOX_SCOPE` | cwd | Colon-separated scope paths |
| `AHMA_TMP_ACCESS` | off | Add temp dir to sandbox scope |
| `AHMA_DISABLE_TEMP` | off | Block all temp dir access |
| `AHMA_LOG_TARGET` | file | Set `stderr` to log to stderr |
| `AHMA_LOG_MONITOR` | off | Enable live log monitoring |
| `AHMA_MONITOR_RATE_LIMIT` | `60` | Min seconds between log alerts |
| `AHMA_PROGRESSIVE_DISCLOSURE_OFF` | off | Expose all tools immediately |
| `RUST_LOG` | `info` | Log verbosity (e.g., `ahma_mcp=debug`) |

Full reference: [environment-variables.md](https://github.com/paulirotta/ahma/blob/main/docs/environment-variables.md)

---

## CLI Reference

```bash
# Start MCP server (stdio — for IDE integration)
ahma-mcp serve stdio [--tools rust,git] [--tmp] [--log-monitor]

# Start HTTP server (local development, multiple clients)
ahma-mcp serve http [--port 3000] [--host 127.0.0.1] [--disable-quic]

# Start Unix socket server (IPC / Kubernetes sidecars)
ahma-mcp serve unix [--socket-path /tmp/ahma.sock]

# Run a single tool from the CLI
ahma-mcp tool run cargo_build -- --release
ahma-mcp tool run sandboxed_shell -- "echo hello"

# Validate .ahma/ tool configs
ahma-mcp tool validate [.ahma/]

# List all configured tools
ahma-mcp tool list [--http http://localhost:3000] [--format json]

# Show locally configured tools with descriptions
ahma-mcp tool info [--tools rust,git]
```

---

## Common Recipes

### Rust project — full quality pipeline

```
activate_tools(action="reveal", bundle="rust")
activate_tools(action="reveal", bundle="git")
cargo_fmt(subcommand="fmt")
cargo_clippy(subcommand="clippy")
cargo_nextest_run(subcommand="nextest run")
```

### Run arbitrary shell commands

```
sandboxed_shell(command="npm ci && npm run build", working_directory="/project")
sandboxed_shell(command="docker compose up -d", timeout_seconds=60)
```

### Check what bundles are available

```
activate_tools(action="list")
```

Returns: bundle names, descriptions, AI hints for when each is useful.

### Monitor Android app logs

```
activate_tools(action="reveal", bundle="kotlin")
android_logcat(...)   # if defined in .ahma/android-logcat.json
```

---

## Troubleshooting

**Tool not found**: Call `activate_tools(action="list")` to see unrevealed bundles.
Then `activate_tools(action="reveal", bundle="<name>")`.

**Timeout**: Set `AHMA_TIMEOUT=600` in mcp.json env, or pass `timeout_seconds` per tool call.

**Permission denied / sandbox error**: The file is outside the sandbox scope.
Check `AHMA_SANDBOX_SCOPE` or add `--tmp` if needed for temp files.

**Nested sandbox warning**: Ahma detected an outer sandbox (Cursor, VS Code, Docker).
Internal sandbox auto-disabled. Set `AHMA_DISABLE_SANDBOX=1` to suppress the warning.

**Tool still running**: Use `status(operation_id)` to check, or `cancel(operation_id)`.

**Linux old kernel**: Landlock requires kernel 5.13+. Set `AHMA_DISABLE_SANDBOX=1` on
older systems (Raspberry Pi OS bullseye, etc.).

---

**See also**: [security-sandbox.md](https://github.com/paulirotta/ahma/blob/main/docs/security-sandbox.md) ·
[live-log-monitoring.md](https://github.com/paulirotta/ahma/blob/main/docs/live-log-monitoring.md) ·
[connection-modes.md](https://github.com/paulirotta/ahma/blob/main/docs/connection-modes.md) ·
[environment-variables.md](https://github.com/paulirotta/ahma/blob/main/docs/environment-variables.md) ·
[mtdf-schema.json](https://github.com/paulirotta/ahma/blob/main/docs/mtdf-schema.json)
AHMA_SKILL_EOF
}

_ahma_skill_content() {
    cat << 'SKILL_EOF'
---
name: ahma-simplify
version: __AHMA_VERSION__
author: Paul Houghton
description: >
  Use this skill when the user asks about code complexity, simplification, maintainability, or
  refactoring. Trigger phrases: "simplify", "reduce complexity", "too complex", "hard to read",
  "refactor", "maintainability", "cognitive complexity", "cyclomatic complexity", "what's the
  most complex file", "code quality metrics", "simplicity score", "ahma-simplify", "hotspot".
  Runs ahma-simplify (via the simplify MCP tool or CLI) to score every file 0-100%, identifies
  the worst hotspot functions, and returns a structured prompt to fix them with minimal, targeted
  changes. Always verifies improvement after editing.
user-invocable: true
---

<!-- version: __AHMA_VERSION__ | author: Paul Houghton -->

# ahma-simplify Skill

Analyze code complexity across any supported language, identify the worst hotspot functions, fix
them with minimal targeted changes, and verify measurable improvement.

Supports: Rust, Python, JavaScript, TypeScript, Kotlin, C, C++, Java, C#, Go, CSS, HTML.

---

## When This Skill Applies

Auto-load this skill when the user:
- Asks to simplify, clean up, or reduce complexity in any file or directory
- Mentions "cognitive complexity", "cyclomatic complexity", or "maintainability index"
- Asks which file or function is hardest to read or maintain
- Wants a code quality report or simplicity score
- Asks you to refactor for readability without changing behavior
- Uses the `simplify` MCP tool or `ahma-simplify` CLI directly

Skip this skill for:
- Pure functional changes (adding/removing features)
- Performance optimization unrelated to code clarity
- Style changes like formatting or renaming

---

## Prerequisites

One of the following must be available:

**Option A — MCP tool (preferred in VS Code / Cursor / Claude Code):**
The `simplify` tool is active in the current ahma-mcp session (enabled with `--tools simplify`
or `--tools rust,simplify`).

**Option B — Direct CLI:**
`ahma-simplify` is on PATH. Install: `cargo install --path ahma_simplify` or use the install
script at `scripts/install.sh` / `scripts/install.ps1`.

To check availability, run: `ahma-simplify --version`

---

## Core Workflow

Follow this sequence. Do not skip steps.

### Step 1 — Run complexity analysis

**Via MCP tool:**
```
simplify(directory="<project-root>", ai_fix=1)
```

**Via CLI:**
```
ahma-simplify <project-root> --ai-fix 1
```

The tool returns:
1. An overall project simplicity score (0–100%)
2. A ranked list of files by complexity (worst first)
3. Function-level hotspots for the top issue (top 5 functions by cognitive complexity)
4. A structured fix prompt for that specific file

### Step 2 — Read the structured fix prompt

The `--ai-fix N` output ends with a structured prompt block. It contains:
- The exact file path to edit
- The hotspot functions (by name, line range, and metrics)
- Constraints for what to change

**Follow the prompt's constraints exactly:**
- Edit **only** the listed hotspot functions
- Do not refactor the whole file
- Do not change function signatures, public APIs, or behavior
- Do not reduce line count at the expense of clarity

### Step 3 — Apply targeted changes

Common patterns for reducing complexity:
- Extract deeply nested logic into well-named helper functions
- Replace complex boolean chains with named predicates
- Replace multi-branch match/switch arms with a lookup table or strategy function
- Flatten early-return cascades (guard clauses)
- Break apart functions with high SLOC alongside high cognitive complexity

**For test files specifically:** If a hotspot is a test file with many small test functions,
skip it. High test count is expected; do not consolidate tests. If a single test function is
individually complex (large setup, many assertions), consider splitting it.

### Step 4 — Verify improvement

After editing, re-analyze the modified file:

**Via MCP tool:**
```
simplify(directory="<project-root>", verify="<path-to-edited-file>")
```

**Via CLI:**
```
ahma-simplify <project-root> --verify <path-to-edited-file>
```

The output shows before/after metrics with a verdict:
- **Significant improvement** (>=10% score gain) — success, continue
- **Modest improvement** (1-9% gain) — acceptable, move to next issue
- **No change** — review if the hotspot functions were actually modified
- **Regression** — revert and try a different approach

### Step 5 — Iterate

Move to the next most complex file:

```
simplify(directory="<project-root>", ai_fix=2)
```

Or CLI: `ahma-simplify <project-root> --ai-fix 2`

Continue iterating until the project score is satisfactory or the user stops.

---

## Score Interpretation

Each file receives a composite score:

```
Score = 0.6 x Maintainability Index + 0.2 x Cognitive Score + 0.2 x Cyclomatic Score
```

| Score Range | Status | Guidance |
|-------------|--------|----------|
| 85-100% | Excellent | No action needed |
| 70-84% | Good | Acceptable; fix only the worst outliers |
| 55-69% | Fair | Plan a simplification sprint |
| 40-54% | Poor | Prioritize before adding features |
| 0-39% | Critical | Address now; maintenance cost is high |

A project overall score below 70% is a signal to run --ai-fix on the top 3-5 files.

---

## MCP Tool Reference

Tool name: `simplify`

| Argument | Type | Default | Purpose |
|----------|------|---------|---------|
| `directory` | path (required) | — | Project root to analyze |
| `ai_fix` | integer | — | Issue number to generate fix prompt for (1 = worst file) |
| `limit` | integer | 50 | Number of issues to include in report |
| `verify` | path | — | Re-analyze a file and compare to baseline |
| `extensions` | array | all | Restrict to specific file types (e.g. ["rs","py"]) |
| `exclude` | array | — | Additional glob patterns to exclude |
| `output_path` | path | — | Write report to directory instead of stdout |
| `html` | boolean | false | Also generate HTML report |

---

## Anti-Patterns to Avoid

1. **Do not refactor the whole file** when only 1-2 functions are hotspots. Follow the fix
   prompt's hotspot list exactly.

2. **Do not add comments to reduce complexity scores.** The Maintainability Index is not
   improved by comments alone; structural simplification is needed.

3. **Do not inline complex logic to reduce function count.** Fewer functions with more
   complexity each makes scores worse, not better.

4. **Do not run --ai-fix without reading the structured prompt.** The prompt contains
   file-specific context that prevents generic, incorrect refactors.

5. **Do not skip Step 4 (verify).** Complexity improvements are only real if the metrics
   confirm it. Syntactically cleaner code can still have higher cognitive complexity.

---

## Quick Reference

```bash
# Analyze and get fix prompt for worst file
ahma-simplify . --ai-fix 1

# Analyze and get fix prompt for 2nd worst file
ahma-simplify . --ai-fix 2

# Verify improvement after editing
ahma-simplify . --verify src/my_module.rs

# Full report to file
ahma-simplify . --output-path ./reports

# HTML report, open in browser
ahma-simplify . --heml

# Restrict to Rust files only
ahma-simplify . --extensions rs --ai-fix 1

# Exclude generated code
ahma-simplify . --exclude '**/generated/**,**/vendor/**' --ai-fix 1
```
SKILL_EOF
}

setup_skill() {
    set +e  # Skill wizard exit codes must not abort the script

    # Skip if there is no terminal to interact with (e.g. CI, non-interactive pipe)
    if [ ! -e /dev/tty ]; then
        return 0
    fi

    echo ""
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "  Agent Skill Setup"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo ""
    echo "  Two agent skills are available for AI tools (VS Code, Cursor, Claude Code):"
    echo ""
    echo "    ahma          — usage guide: sandboxed_shell, bundles, sandbox, etc."
    echo "    ahma-simplify — code complexity analysis and hotspot fixing workflow"
    echo ""
    echo "  Skills install to ~/.agents/skills/ — the universal cross-platform skill path."
    echo "  They are automatically used by the AI when relevant tasks are requested."
    echo ""
    printf "Install Ahma agent skills? [Y/n]: "
    local CHOICE
    IFS= read -r CHOICE < /dev/tty
    case "$CHOICE" in
        [Nn]*) return 0 ;;
    esac

    # ── Semver parser: outputs MAJOR MINOR PATCH (space-separated) ──────────
    _ahma_parse_semver() {
        local VER="$1"
        # Strip leading 'v' if present
        VER="${VER#v}"
        local MAJOR MINOR PATCH
        MAJOR="$(echo "$VER" | cut -d. -f1)"
        MINOR="$(echo "$VER" | cut -d. -f2)"
        PATCH="$(echo "$VER" | cut -d. -f3)"
        # Validate all three parts are numeric
        case "$MAJOR$MINOR$PATCH" in
            *[!0-9]*) return 1 ;;
        esac
        echo "$MAJOR $MINOR $PATCH"
    }

    # ── Semver comparator: returns 0=equal, 1=first>second, 2=first<second ──
    _ahma_semver_compare() {
        local A="$1" B="$2"
        local A_PARTS B_PARTS
        A_PARTS="$(_ahma_parse_semver "$A")" || return 3
        B_PARTS="$(_ahma_parse_semver "$B")" || return 3
        local A_MAJ A_MIN A_PAT B_MAJ B_MIN B_PAT
        read -r A_MAJ A_MIN A_PAT <<< "$A_PARTS"
        read -r B_MAJ B_MIN B_PAT <<< "$B_PARTS"
        if   [ "$A_MAJ" -gt "$B_MAJ" ]; then return 1
        elif [ "$A_MAJ" -lt "$B_MAJ" ]; then return 2
        elif [ "$A_MIN" -gt "$B_MIN" ]; then return 1
        elif [ "$A_MIN" -lt "$B_MIN" ]; then return 2
        elif [ "$A_PAT" -gt "$B_PAT" ]; then return 1
        elif [ "$A_PAT" -lt "$B_PAT" ]; then return 2
        else return 0
        fi
    }

    # ── Helper: install one skill by name, content function, and version ──────
    _ahma_install_one_skill() {
        local NAME="$1"
        local VERSION="$2"
        local CONTENT_FN="$3"

        local SKILL_DIR="${HOME}/.agents/skills/${NAME}"
        local SKILL_PATH="${SKILL_DIR}/SKILL.md"

        if [ -f "$SKILL_PATH" ]; then
            local EXISTING_VER
            EXISTING_VER=$(grep '^version:' "$SKILL_PATH" 2>/dev/null | head -1 | awk '{print $2}' | tr -d '"'"'" )

            if [ -z "$EXISTING_VER" ]; then
                # No parseable version — treat as upgrade
                echo ""
                echo "  Existing ${NAME} skill has no version tag. Installing v${VERSION}."
            else
                local CMP_RESULT
                _ahma_semver_compare "$VERSION" "$EXISTING_VER" && CMP_RESULT=$? || CMP_RESULT=$?

                local NEW_MAJ
                NEW_MAJ="$(echo "${VERSION#v}" | cut -d. -f1)"
                local OLD_MAJ
                OLD_MAJ="$(echo "${EXISTING_VER#v}" | cut -d. -f1)"

                case "$CMP_RESULT" in
                    0)  # Same version
                        echo ""
                        echo "  ${NAME} skill v${VERSION} is already installed."
                        printf "  Reinstall (overwrite)? [y/N]: "
                        local CONFIRM
                        IFS= read -r CONFIRM < /dev/tty
                        case "$CONFIRM" in
                            [Yy]*) ;;
                            *) echo "  Skipped."; return 0 ;;
                        esac
                        ;;
                    1)  # New > old (upgrade)
                        if [ "$NEW_MAJ" != "$OLD_MAJ" ]; then
                            # Major version bump — ask
                            echo ""
                            printf "  Major version upgrade for ${NAME} skill: v${EXISTING_VER} → v${VERSION}. Install? [Y/n]: "
                            local CONFIRM
                            IFS= read -r CONFIRM < /dev/tty
                            case "$CONFIRM" in
                                [Nn]*) echo "  Skipped."; return 0 ;;
                            esac
                        else
                            # Minor/patch upgrade — auto-install
                            echo ""
                            echo "  Upgrading ${NAME} skill v${EXISTING_VER} → v${VERSION}..."
                        fi
                        ;;
                    2)  # New < old (downgrade)
                        echo ""
                        printf "  Downgrade ${NAME} skill v${EXISTING_VER} → v${VERSION}? [y/N]: "
                        local CONFIRM
                        IFS= read -r CONFIRM < /dev/tty
                        case "$CONFIRM" in
                            [Yy]*) ;;
                            *) echo "  Skipped."; return 0 ;;
                        esac
                        ;;
                    3)  # Unparseable version — treat as upgrade
                        echo ""
                        echo "  Existing ${NAME} skill v${EXISTING_VER:-unknown} found. Installing v${VERSION}."
                        ;;
                esac
            fi
        fi

        mkdir -p "$SKILL_DIR"
        "$CONTENT_FN" | sed "s/__AHMA_VERSION__/${VERSION}/g" > "$SKILL_PATH"
        echo "  ✓ ${SKILL_PATH}"
    }

    echo ""
    echo "  Installing skills..."
    _ahma_install_one_skill "ahma"          "$AHMA_VERSION" "_ahma_main_skill_content"
    _ahma_install_one_skill "ahma-simplify" "$AHMA_VERSION" "_ahma_skill_content"

    echo ""
    echo "  Skills are automatically available in:"
    echo "    • VS Code (GitHub Copilot) — auto-loaded when relevant"
    echo "    • Cursor — attach with @ahma or @ahma-simplify in chat"
    echo "    • Claude Code — loaded from ~/.agents/skills/"
    echo ""
    echo "  To also enable per-project (commit to your repo):"
    echo "    mkdir -p .agents/skills/ahma .agents/skills/ahma-simplify"
    echo "    cp ~/.agents/skills/ahma/SKILL.md .agents/skills/ahma/SKILL.md"
    echo "    cp ~/.agents/skills/ahma-simplify/SKILL.md .agents/skills/ahma-simplify/SKILL.md"
    echo ""
}

# Run the MCP setup wizard
setup_mcp

# Run the skill setup wizard
setup_skill
