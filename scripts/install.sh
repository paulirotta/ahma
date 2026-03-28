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

    # Confirm platform selection
    echo ""
    echo "✓ Selected platforms:"
    [ "$(_ahma_list_has "$PLATFORMS" 1 && echo y)" = "y" ] && echo "    • VS Code"
    [ "$(_ahma_list_has "$PLATFORMS" 2 && echo y)" = "y" ] && echo "    • Claude Code"
    [ "$(_ahma_list_has "$PLATFORMS" 3 && echo y)" = "y" ] && echo "    • Cursor"
    [ "$(_ahma_list_has "$PLATFORMS" 4 && echo y)" = "y" ] && echo "    • Antigravity"

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
# Installs the ahma-simplify agent skill to ~/.agents/skills/ahma-simplify/SKILL.md
# Compatible with VS Code (GitHub Copilot), Cursor, and Claude Code — all index .agents/skills/
# ─────────────────────────────────────────────────────────────────────────────

_ahma_skill_content() {
    cat << 'SKILL_EOF'
---
name: ahma-simplify
description: >
  Use this skill when the user asks about code complexity, simplification, maintainability, or
  refactoring. Trigger phrases: "simplify", "reduce complexity", "too complex", "hard to read",
  "refactor", "maintainability", "cognitive complexity", "cyclomatic complexity", "what's the
  most complex file", "code quality metrics", "simplicity score", "ahma-simplify", "hotspot".
  Runs ahma-simplify (via the simplify MCP tool or CLI) to score every file 0-100%, identifies
  the worst hotspot functions, and returns a structured prompt to fix them with minimal, targeted
  changes. Always verifies improvement after editing.
version: 1.0.0
author: ahma project
user-invocable: true
---

# ahma-simplify Skill

Analyze code complexity across any supported language, identify the worst hotspot functions, fix
them with minimal targeted changes, and verify measurable improvement.

Supports: Rust, Python, JavaScript, TypeScript, C, C++, Java, C#, Go, CSS, HTML.

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
    echo "  The ahma-simplify skill teaches AI agents (VS Code, Cursor, Claude Code)"
    echo "  to analyze code complexity, fix hotspot functions, and verify improvement."
    echo "  It installs to ~/.agents/skills/ — the universal cross-platform skill path."
    echo ""
    printf "Install the ahma-simplify agent skill? [Y/n]: "
    local CHOICE
    IFS= read -r CHOICE < /dev/tty
    case "$CHOICE" in
        [Nn]*) return 0 ;;
    esac

    local SKILL_DIR="${HOME}/.agents/skills/ahma-simplify"
    local SKILL_PATH="${SKILL_DIR}/SKILL.md"

    # ── Check for existing installation ────────────────────────────────────
    if [ -f "$SKILL_PATH" ]; then
        local EXISTING_VER
        EXISTING_VER=$(grep '^version:' "$SKILL_PATH" 2>/dev/null | head -1 | awk '{print $2}' | tr -d '"'"'" )
        local NEW_VER="1.0.0"
        echo ""
        if [ "$EXISTING_VER" = "$NEW_VER" ]; then
            echo "  ahma-simplify skill v${NEW_VER} is already installed at:"
            echo "  ${SKILL_PATH}"
            echo ""
            printf "  Reinstall (overwrite)? [y/N]: "
            local CONFIRM
            IFS= read -r CONFIRM < /dev/tty
            case "$CONFIRM" in
                [Yy]*) ;;
                *) echo "  Skipped."; return 0 ;;
            esac
        else
            echo "  Existing skill v${EXISTING_VER:-unknown} found. Installing v${NEW_VER}."
        fi
    fi

    # ── Create skill directory and write SKILL.md ───────────────────────────
    mkdir -p "$SKILL_DIR"
    _ahma_skill_content > "$SKILL_PATH"

    echo ""
    echo "  ✓ Skill installed: ${SKILL_PATH}"
    echo ""
    echo "  This skill is automatically available in:"
    echo "    • VS Code (GitHub Copilot) — auto-loaded when relevant"
    echo "    • Cursor — attach with @ahma-simplify in chat"
    echo "    • Claude Code — loaded from ~/.agents/skills/"
    echo ""
    echo "  To also enable per-project (commit to your repo):"
    echo "    mkdir -p .agents/skills/ahma-simplify"
    echo "    cp ${SKILL_PATH} .agents/skills/ahma-simplify/SKILL.md"
    echo ""
}

# Run the MCP setup wizard
setup_mcp

# Run the skill setup wizard
setup_skill
