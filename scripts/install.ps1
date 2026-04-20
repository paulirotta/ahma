# One-liner installer for ahma-mcp on Windows
# Usage: irm https://raw.githubusercontent.com/paulirotta/ahma/main/scripts/install.ps1 | iex
#
# Supported platforms:
#   - Windows x86_64 (x64)
#
# Requirements:
#   - PowerShell 5.1+ (built in to Windows 10/11)
#   - Internet access to GitHub releases
#
# Environment variables:
#   AHMA_INSTALL_DIR     - Override install directory (default: $HOME\.local\bin)

#Requires -Version 5

$ErrorActionPreference = 'Stop'

# ── Detect architecture ────────────────────────────────────────────────────────
$arch = $env:PROCESSOR_ARCHITECTURE
if ($arch -ne "AMD64") {
    Write-Error "Unsupported architecture: $arch. Only x86_64 (AMD64) Windows builds are available."
    exit 1
}

$platform = "windows-x86_64"

# ── Install directory ──────────────────────────────────────────────────────────
$installDir = if ($env:AHMA_INSTALL_DIR) {
    $env:AHMA_INSTALL_DIR
} else {
    Join-Path $HOME ".local\bin"
}

# ── Fetch latest release metadata ─────────────────────────────────────────────
$releasesUrl = "https://api.github.com/repos/paulirotta/ahma/releases/latest"
Write-Host "Fetching latest release info..."

try {
    $releaseJson = Invoke-RestMethod -Uri $releasesUrl -UseBasicParsing
} catch {
    Write-Error "Failed to fetch release info from $releasesUrl : $_"
    exit 1
}

$latestVer = ($releaseJson.tag_name -replace '^v', '')

# ── Check for existing installation and compare versions ──────────────────────
$existingCmd   = Get-Command ahma-mcp -ErrorAction SilentlyContinue
$existingInDir = Join-Path $installDir 'ahma-mcp.exe'
$existingBin   = if ($existingCmd) { $existingCmd.Source } elseif (Test-Path $existingInDir) { $existingInDir } else { $null }

if ($existingBin) {
    $installedVerRaw = (& $existingBin --version 2>&1) -join ''
    $installedVer = ($installedVerRaw -split '\s+' | Select-Object -Last 1).Trim()

    if ($installedVer -ne $latestVer -and $latestVer) {
        Write-Host "Upgrading ahma-mcp from $installedVer to $latestVer ..."
    } else {
        Write-Host "Ahma $installedVer is already installed and up to date."
        Write-Host ''
        Write-Host "  Location : $existingBin"
        Write-Host "  Simplify : available via 'ahma-mcp simplify --help'"
        Write-Host ''
        $confirm = Read-Host "Reinstall anyway? [y/N]"
        if ($confirm -notmatch '^[Yy]') {
            Write-Host 'No changes made.'
            exit 0
        }
        Write-Host 'Reinstalling...'
    }
}

Write-Host "Installing Ahma for $platform to $installDir ..."
New-Item -ItemType Directory -Force -Path $installDir | Out-Null

$assetName = "ahma-release-$platform.zip"
$asset = $releaseJson.assets | Where-Object { $_.name -eq $assetName } | Select-Object -First 1

if (-not $asset) {
    Write-Error @"
Could not find release asset '$assetName'.
Please check https://github.com/paulirotta/ahma/releases for available binaries.
"@
    exit 1
}

$downloadUrl = $asset.browser_download_url
Write-Host "Downloading $downloadUrl ..."

# ── Download and extract ───────────────────────────────────────────────────────
$tempDir = Join-Path ([System.IO.Path]::GetTempPath()) ([System.IO.Path]::GetRandomFileName())
New-Item -ItemType Directory -Force -Path $tempDir | Out-Null

try {
    $zipPath = Join-Path $tempDir $assetName
    Invoke-WebRequest -Uri $downloadUrl -OutFile $zipPath -UseBasicParsing
    Expand-Archive -Path $zipPath -DestinationPath $tempDir -Force

    # ── Install binaries ───────────────────────────────────────────────────────
    Write-Host "Installing binaries to $installDir ..."

    foreach ($bin in @("ahma-mcp.exe")) {
        $src = Join-Path $tempDir $bin
        if (Test-Path $src) {
            Copy-Item -Path $src -Destination $installDir -Force
            Write-Host "  Installed $bin"
        } else {
            if ($bin -eq "ahma-mcp.exe") {
                Write-Error "ahma-mcp.exe not found in archive"
                exit 1
            }
        }
    }
} finally {
    Remove-Item -Recurse -Force -Path $tempDir -ErrorAction SilentlyContinue
}

# ── Verify and report ──────────────────────────────────────────────────────────
$mcpBin = Join-Path $installDir "ahma-mcp.exe"

& $mcpBin --version

# Remove legacy ahma-simplify.exe if present
$legacyCandidates = @(
    (Join-Path $installDir "ahma-simplify.exe"),
    (Join-Path "$HOME\.local\bin" "ahma-simplify.exe")
)
foreach ($legacy in $legacyCandidates) {
    if (Test-Path $legacy) {
        Remove-Item -Force $legacy -ErrorAction SilentlyContinue
        Write-Host "Removed legacy binary: $legacy"
        Write-Host "  Code complexity analysis is now built into ahma-mcp."
        Write-Host "  New command: ahma-mcp simplify <directory> --ai-fix 1"
    }
}

Write-Host ""
Write-Host "Success! Installed ahma-mcp to $installDir"
Write-Host ""
Write-Host "Ensure $installDir is in your PATH."
Write-Host "To add permanently, run:"
Write-Host "  [Environment]::SetEnvironmentVariable('PATH', `"`$env:PATH;$installDir`", 'User')"
Write-Host ""
Write-Host "PowerShell (built into Windows 10/11) is used at runtime. No additional installation needed."

# ─────────────────────────────────────────────────────────────────────────────
# MCP Server Setup Wizard
# ─────────────────────────────────────────────────────────────────────────────

function Get-AhmaEntryObject {
    param(
        [string]$Transport,
        [string]$PlatformType
    )
    if ($Transport -eq 'http') {
        return [ordered]@{
            type = 'http'
            url  = 'http://localhost:3000/mcp'
        }
    } elseif ($PlatformType -eq 'antigravity') {
        return [ordered]@{
            command = 'powershell'
            args    = @(
                '-NoProfile',
                '-Command',
                '$env:AHMA_SANDBOX_SCOPE = $env:USERPROFILE; ahma-mcp serve stdio --tools rust,simplify --tmp --log-monitor'
            )
        }
    } else {
        return [ordered]@{
            type    = 'stdio'
            command = 'ahma-mcp'
            args    = @('serve', 'stdio', '--tools', 'rust,simplify', '--tmp', '--log-monitor')
        }
    }
}

function Invoke-AhmaMcpPlatform {
    param(
        [string]$DisplayName,
        [string]$ConfigPath,
        [string]$ServersKey,
        [string]$PlatformType,
        [string]$Transport,
        [ref]$ConfiguredTools
    )

    Write-Host ""
    Write-Host "  --- $DisplayName ---"
    Write-Host "  Config: $ConfigPath"

    $ahmaEntry = Get-AhmaEntryObject -Transport $Transport -PlatformType $PlatformType

    if (-not (Test-Path $ConfigPath)) {
        # New file
        $newConfig = [ordered]@{ $ServersKey = [ordered]@{ Ahma = $ahmaEntry } }
        $proposed  = $newConfig | ConvertTo-Json -Depth 10

        Write-Host ""
        Write-Host "  File does not exist. Proposed new file:"
        Write-Host ""
        $proposed -split "`n" | ForEach-Object { Write-Host "    $_" }
        Write-Host ""
        $confirm = Read-Host "  Create this file? [Y/n]"
        if ($confirm -match '^[Nn]') { Write-Host "  Skipped."; return }

        New-Item -ItemType Directory -Force -Path (Split-Path $ConfigPath) | Out-Null
        $proposed | Set-Content -Encoding UTF8 -Path $ConfigPath
        Write-Host "  Created."
        $ConfiguredTools.Value += "|$DisplayName"

    } else {
        # Existing file — merge
        try {
            $raw    = Get-Content -Raw -Path $ConfigPath
            $config = $raw | ConvertFrom-Json
        } catch {
            Write-Host "  Could not parse existing file. Add manually under `"$ServersKey`":"
            Write-Host "  $ConfigPath"
            $snippet = ([ordered]@{ Ahma = $ahmaEntry }) | ConvertTo-Json -Depth 10
            $snippet -split "`n" | ForEach-Object { Write-Host "    $_" }
            return
        }

        # Ensure the servers key exists
        if (-not ($config | Get-Member -Name $ServersKey -MemberType NoteProperty)) {
            $config | Add-Member -NotePropertyName $ServersKey -NotePropertyValue ([PSCustomObject]@{}) -Force
        }
        # Add/replace the Ahma entry
        $config.$ServersKey | Add-Member -NotePropertyName 'Ahma' -NotePropertyValue $ahmaEntry -Force

        $proposed = $config | ConvertTo-Json -Depth 10

        Write-Host ""
        Write-Host "  Proposed file after adding Ahma entry:"
        Write-Host ""
        $proposed -split "`n" | ForEach-Object { Write-Host "    $_" }
        Write-Host ""
        $confirm = Read-Host "  Update this file? [Y/n]"
        if ($confirm -match '^[Nn]') { Write-Host "  Skipped."; return }

        $proposed | Set-Content -Encoding UTF8 -Path $ConfigPath
        Write-Host "  Updated."
        $ConfiguredTools.Value += "|$DisplayName"
    }
}

function Get-AhmaCodexToml {
    param([string]$Transport)
    if ($Transport -eq 'http') {
        return @"
[mcp_servers.Ahma]
url = "http://localhost:3000/mcp"
"@
    } else {
        return @"
[mcp_servers.Ahma]
command = "ahma-mcp"
args = ["serve", "stdio", "--tools", "rust,simplify", "--tmp", "--log-monitor"]
"@
    }
}

function Invoke-AhmaCodexPlatform {
    param(
        [string]$Transport,
        [ref]$ConfiguredTools
    )
    $configPath = Join-Path $HOME '.codex' 'config.toml'
    $tomlEntry  = Get-AhmaCodexToml -Transport $Transport

    Write-Host ""
    Write-Host "  --- Codex CLI ---"
    Write-Host "  Config: $configPath"

    if (-not (Test-Path $configPath)) {
        # New file
        Write-Host ""
        Write-Host "  File does not exist. Proposed new file:"
        Write-Host ""
        $tomlEntry -split "`n" | ForEach-Object { Write-Host "    $_" }
        Write-Host ""
        $confirm = Read-Host "  Create this file? [Y/n]"
        if ($confirm -match '^[Nn]') { Write-Host "  Skipped."; return }

        New-Item -ItemType Directory -Force -Path (Split-Path $configPath) | Out-Null
        $tomlEntry | Set-Content -Encoding UTF8 -Path $configPath
        Write-Host "  Created."
        $ConfiguredTools.Value += "|Codex CLI"

    } else {
        $content    = Get-Content -Raw -Path $configPath
        $hasSection = $content -match '(?m)^\[mcp_servers\.Ahma\]'

        if ($hasSection) {
            # Section already exists -- propose replacement
            Write-Host ""
            Write-Host "  [mcp_servers.Ahma] entry already exists in $configPath."
            Write-Host ""
            Write-Host "  Proposed replacement:"
            Write-Host ""
            $tomlEntry -split "`n" | ForEach-Object { Write-Host "    $_" }
            Write-Host ""
            $confirm = Read-Host "  Replace existing entry? [y/N]"
            if ($confirm -notmatch '^[Yy]') { Write-Host "  Skipped."; return }

            # Remove old section line-by-line, then append new entry
            $lines  = $content -split '\r?\n'
            $result = [System.Collections.Generic.List[string]]::new()
            $skip   = $false
            foreach ($line in $lines) {
                if ($skip -and $line -match '^\[') { $skip = $false }
                if ($line -match '^\[mcp_servers\.Ahma\]') { $skip = $true; continue }
                if (-not $skip) { $result.Add($line) }
            }
            $newContent = (($result -join "`n").TrimEnd("`n")) + "`n`n" + $tomlEntry + "`n"
            $newContent | Set-Content -Encoding UTF8 -Path $configPath
            Write-Host "  Updated."
            $ConfiguredTools.Value += "|Codex CLI"

        } else {
            # File exists but no [mcp_servers.Ahma] -- append
            Write-Host ""
            Write-Host "  Appending [mcp_servers.Ahma] to ${configPath}:"
            Write-Host ""
            $tomlEntry -split "`n" | ForEach-Object { Write-Host "    $_" }
            Write-Host ""
            $confirm = Read-Host "  Add to file? [Y/n]"
            if ($confirm -match '^[Nn]') { Write-Host "  Skipped."; return }

            Add-Content -Encoding UTF8 -Path $configPath -Value "`n$tomlEntry"
            Write-Host "  Updated."
            $ConfiguredTools.Value += "|Codex CLI"
        }
    }
}

function Invoke-AhmaMcpSetup {
    Write-Host ""
    Write-Host "======================================================="
    Write-Host "  MCP Server Setup"
    Write-Host "======================================================="
    Write-Host ""
    $choice = Read-Host "Configure ahma-mcp as a global MCP server for your AI tools? [Y/n]"
    if ($choice -match '^[Nn]') { return }

    # ── Platform selection ──────────────────────────────────────────────────
    Write-Host ""
    Write-Host "Select platforms to configure (comma-separated numbers, or Enter for all):"
    Write-Host "  1) VS Code      ($env:APPDATA\Code\User\mcp.json)"
    Write-Host "  2) Claude Code  ($HOME\.claude.json)"
    Write-Host "  3) Cursor       ($HOME\.cursor\mcp.json)"
    Write-Host "  4) Antigravity  ($HOME\.antigravity\mcp.json)"
    Write-Host "  5) Codex CLI    ($HOME\.codex\config.toml)"
    Write-Host ""
    $platformsInput = Read-Host "  Selection [default: 1,2,3,4,5 -- all]"
    if ([string]::IsNullOrWhiteSpace($platformsInput)) { $platformsInput = '1,2,3,4,5' }
    # Accept flexible formats: "1,2,4" or "124" or "1 2 4" or mixed "1, 2,4" etc.
    # Extract all individual digits from the input
    $selectedNums = [regex]::Matches($platformsInput, '\d') | ForEach-Object { $_.Value }

    # Confirm platform selection
    Write-Host ""
    Write-Host "Selected platforms:"
    if ($selectedNums -contains '1') { Write-Host "    * VS Code" }
    if ($selectedNums -contains '2') { Write-Host "    * Claude Code" }
    if ($selectedNums -contains '3') { Write-Host "    * Cursor" }
    if ($selectedNums -contains '4') { Write-Host "    * Antigravity" }
    if ($selectedNums -contains '5') { Write-Host "    * Codex CLI" }

    # ── Transport selection ─────────────────────────────────────────────────
    Write-Host ""
    Write-Host "Choose how your AI tools connect to ahma-mcp:"
    Write-Host ""
    Write-Host "  1) stdio  (recommended for most users)"
    Write-Host "     Each AI tool starts its own private ahma-mcp instance automatically"
    Write-Host "     when you open a project. No extra steps needed -- it just works."
    Write-Host ""
    Write-Host "  2) http   (one shared server, better visibility)"
    Write-Host "     You run 'ahma-mcp serve http --tools rust,simplify' in a terminal"
    Write-Host "     before opening your AI tools. All tools connect to one running"
    Write-Host "     instance, so you can watch what ahma is doing in real time."
    Write-Host "     Best if you use multiple AI tools simultaneously."
    Write-Host ""
    $tselect = Read-Host "  Mode [1=stdio or 2=http, default 1]"
    $transport = if ($tselect -eq '2') { 'http' } else { 'stdio' }

    # Confirm transport selection
    Write-Host ""
    if ($transport -eq 'http') {
        Write-Host "Transport mode: http (one shared server)"
    } else {
        Write-Host "Transport mode: stdio (recommended for most users)"
    }

    # ── Configure each selected platform ───────────────────────────────────
    $configuredTools = [ref]''

    $platforms = @(
        @{ Num = '1'; Display = 'VS Code';     Path = "$env:APPDATA\Code\User\mcp.json"; Key = 'servers';    Type = 'standard'    },
        @{ Num = '2'; Display = 'Claude Code'; Path = "$HOME\.claude.json";               Key = 'mcpServers'; Type = 'standard'    },
        @{ Num = '3'; Display = 'Cursor';      Path = "$HOME\.cursor\mcp.json";           Key = 'mcpServers'; Type = 'standard'    },
        @{ Num = '4'; Display = 'Antigravity'; Path = "$HOME\.antigravity\mcp.json";      Key = 'mcpServers'; Type = 'antigravity' }
    )

    foreach ($p in $platforms) {
        if ($selectedNums -contains $p.Num) {
            Invoke-AhmaMcpPlatform `
                -DisplayName   $p.Display `
                -ConfigPath    $p.Path `
                -ServersKey    $p.Key `
                -PlatformType  $p.Type `
                -Transport     $transport `
                -ConfiguredTools $configuredTools
        }
    }
    if ($selectedNums -contains '5') {
        Invoke-AhmaCodexPlatform -Transport $transport -ConfiguredTools $configuredTools
    }

    # ── Summary ─────────────────────────────────────────────────────────────
    Write-Host ""
    if ($configuredTools.Value -ne '') {
        Write-Host "MCP setup complete! Restart these tools for changes to take effect:"
        $configuredTools.Value -split '\|' | Where-Object { $_ -ne '' } | ForEach-Object {
            Write-Host "    - $_"
        }
        if ($transport -eq 'http') {
            Write-Host ""
            Write-Host "  Before opening your AI tools, start the ahma HTTP server:"
            Write-Host "    ahma-mcp serve http --tools rust,simplify"
        }
    } else {
        Write-Host "No MCP configurations were changed."
    }
    Write-Host ""
}

# Skill Setup Wizard
# Installs Ahma agent skills to ~/.agents/skills/
#   ahma — comprehensive usage guide (sandboxed_shell, bundles, sandbox, simplify, etc.)
# Compatible with VS Code (GitHub Copilot), Cursor, and Claude Code — all index .agents/skills/
# ─────────────────────────────────────────────────────────────────────────────

function Get-AhmaMainSkillContent {
    return @'
---
name: ahma
version: 0.6.1
author: Paul Houghton
description: >
  Comprehensive guide for using Ahma (ahma-mcp) as an AI agent. USE THIS SKILL when you need
  to understand how to run tools, activate bundles, use the sandbox, monitor logs, author custom
  tools, or configure ahma-mcp. Also handles code complexity analysis via /ahma simplify.
  Trigger phrases: "use ahma", "run with ahma", "ahma tool", "activate bundle",
  "sandboxed_shell", "ahma async", "ahma serve", "mcp.json ahma", "ahma sandbox",
  "ahma livelog", "ahma monitor", "custom tool .ahma", "ahma-mcp", "await tool",
  "cancel operation", "tool bundle", "progressive disclosure", "activate_tools",
  "simplify", "reduce complexity", "too complex", "hard to read", "refactor",
  "maintainability", "cognitive complexity", "cyclomatic complexity", "simplicity score",
  "code quality metrics", "hotspot", "ahma simplify", "ahma help", "ahma ?".
user-invocable: true
---

<!-- version: 0.6.1 | author: Paul Houghton -->

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
| **Claude Code** | `~/.claude.json` -> `"mcpServers"` key |
| **Claude Desktop** | `~/Library/Application Support/Claude/claude_desktop_config.json` |
| **Codex CLI** | `~/.codex/config.toml` -> `[mcp_servers.Ahma]` |

Same JSON structure as above. The server starts automatically when chat is opened.

### 3. VS Code auto-start setting

Enable globally in VS Code settings:
```json
{ "chat.mcp.autoStart": true }
```
This auto-(re)starts MCP servers when configuration changes are detected, so
Ahma spins up as soon as VS Code sees the config -- no need to open chat first.

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
```powershell
irm https://raw.githubusercontent.com/paulirotta/ahma/main/scripts/install.ps1 | iex
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
1. Check if `.vscode/mcp.json` exists -- if not, offer to create it
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

**Load bundles but keep hidden** (default -- LLM calls `activate_tools` to reveal):
```json
"args": ["serve", "stdio", "--tools", "rust,git,fileutils"]
```

**Auto-reveal at startup** (recommended -- all `--tools` bundles visible immediately):
```json
"args": ["serve", "stdio", "--tools", "rust,git,fileutils", "--auto-reveal"]
```

---

## Built-in Tools (Always Available)

### `sandboxed_shell` -- Run any shell command

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

### `status` -- Check async operation progress

```
status(operation_id="op_abc123")
```

Returns current state: `running`, `complete`, `failed`, `cancelled`, or `timeout`.
Non-blocking -- safe to call repeatedly.

### `await` -- Wait for an async operation to finish

```
await(operation_id="op_abc123", timeout_seconds=60)
```

Blocks until the operation completes or times out. Use sparingly -- prefer `status` polling
when you want to continue other work in parallel.

### `cancel` -- Cancel a running operation

```
cancel(operation_id="op_abc123")
```

Sends cancellation signal. The process is terminated and resources are freed.

---

## Async-First Workflow

Most tools run **asynchronously** by default -- they return an `operation_id` immediately.

```
# 1. Start a long operation
result = cargo_build(subcommand="build")
# -> { "operation_id": "op_abc123", "status": "started" }

# 2. Check progress (non-blocking)
status(operation_id="op_abc123")
# -> { "status": "running", "output_so_far": "..." }

# 3. Wait for completion when needed
await(operation_id="op_abc123", timeout_seconds=120)
# -> { "status": "complete", "exit_code": 0, "output": "..." }

# Or: cancel if taking too long
cancel(operation_id="op_abc123")
```

**Force synchronous** for state-modifying commands (e.g., `cargo add`):
- Set `"synchronous": true` in the tool's MTDF JSON, or
- Start server with `--sync` flag, or set `AHMA_SYNC=1`

---

## Sandbox -- Filesystem Security

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

### 1. `--log-monitor` flag -- Monitor Ahma's own server logs

```json
"args": ["serve", "stdio", "--log-monitor"]
```

Tails Ahma's rolling log files (`./log/ahma_mcp.log.*`), analyzes chunks with an LLM, and
pushes `LogAlert` MCP progress notifications when errors or anomalies are detected.

Configure minimum seconds between alerts: `--monitor-rate-limit 60` (default 60).

### 2. `livelog` tool type -- Monitor any streaming command

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

## Custom Tools -- `.ahma/` Directory

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
# Start MCP server (stdio -- for IDE integration)
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

### Rust project -- full quality pipeline

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

**See also**: [security-sandbox.md](https://github.com/paulirotta/ahma/blob/main/docs/security-sandbox.md) *
[live-log-monitoring.md](https://github.com/paulirotta/ahma/blob/main/docs/live-log-monitoring.md) *
[connection-modes.md](https://github.com/paulirotta/ahma/blob/main/docs/connection-modes.md) *
[environment-variables.md](https://github.com/paulirotta/ahma/blob/main/docs/environment-variables.md) *
[mtdf-schema.json](https://github.com/paulirotta/ahma/blob/main/docs/mtdf-schema.json) *
[SIMPLIFY.md](https://github.com/paulirotta/ahma/blob/main/SIMPLIFY.md)
'@
}

function Invoke-AhmaSkillSetup {
    Write-Host ""
    Write-Host "======================================================="
    Write-Host "  Agent Skill Setup"
    Write-Host "======================================================="
    Write-Host ""
    Write-Host "  One agent skill is available for AI tools (VS Code, Cursor, Claude Code):"
    Write-Host ""
    Write-Host "    ahma -- comprehensive usage guide including sandboxed_shell, bundles, sandbox,"
    Write-Host "             code complexity analysis (/ahma simplify), and more."
    Write-Host ""
    Write-Host "  Skills install to ~/.agents/skills/ -- the universal cross-platform skill path."
    Write-Host "  They are automatically used by the AI when relevant tasks are requested."
    Write-Host ""
    $choice = Read-Host "Install Ahma agent skills? [Y/n]"
    if ($choice -match '^[Nn]') { return }

    function Install-OneSkill {
        param(
            [string]$Name,
            [string]$Version,
            [scriptblock]$ContentFn
        )
        $skillDir  = Join-Path $HOME '.agents' 'skills' $Name
        $skillPath = Join-Path $skillDir 'SKILL.md'

        if (Test-Path $skillPath) {
            $existingVerLine = (Select-String -Path $skillPath -Pattern '^version:' | Select-Object -First 1).Line
            $existingVer = if ($existingVerLine) { ($existingVerLine -split '\s+')[1].Trim("'`"") } else { 'unknown' }
            Write-Host ""
            if ($existingVer -eq $Version) {
                Write-Host "  $Name skill v$Version is already installed."
                $confirm = Read-Host "  Reinstall (overwrite)? [y/N]"
                if ($confirm -notmatch '^[Yy]') { Write-Host "  Skipped."; return }
            } else {
                Write-Host "  Existing $Name skill v$existingVer found. Installing v$Version."
            }
        }

        New-Item -ItemType Directory -Force -Path $skillDir | Out-Null
        & $ContentFn | Set-Content -Encoding UTF8 -Path $skillPath
        Write-Host "  OK $skillPath"
    }

    Write-Host ""
    Write-Host "  Installing skills..."
    Install-OneSkill -Name 'ahma' -Version '0.6.1' -ContentFn { Get-AhmaMainSkillContent }

    Write-Host ""
    Write-Host "  Skills are automatically available in:"
    Write-Host "    * VS Code (GitHub Copilot) -- auto-loaded when relevant"
    Write-Host "    * Cursor -- attach with @ahma in chat"
    Write-Host "    * Claude Code -- loaded from ~/.agents/skills/"
    Write-Host ""
    Write-Host "  To also enable per-project (commit to your repo):"
    Write-Host "    New-Item -ItemType Directory -Force .agents\skills\ahma"
    Write-Host "    Copy-Item (Join-Path $HOME .agents skills ahma SKILL.md) .agents\skills\ahma\SKILL.md"
    Write-Host ""
}

# Run the MCP setup wizard
Invoke-AhmaMcpSetup

# Run the skill setup wizard
Invoke-AhmaSkillSetup
