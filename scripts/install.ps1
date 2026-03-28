# One-liner installer for ahma-mcp and ahma-simplify on Windows
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
$releasesUrl = "https://api.github.com/repos/paulirotta/ahma/releases/tags/latest"
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
        $simplifyBin = Join-Path ([System.IO.Path]::GetDirectoryName($existingBin)) 'ahma-simplify.exe'
        if (Test-Path $simplifyBin) {
            Write-Host "  Simplify : $simplifyBin — $(& $simplifyBin --version 2>&1)"
        }
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

    foreach ($bin in @("ahma-mcp.exe", "ahma-simplify.exe")) {
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
$mcpBin     = Join-Path $installDir "ahma-mcp.exe"
$simplifyBin = Join-Path $installDir "ahma-simplify.exe"

& $mcpBin --version
if (Test-Path $simplifyBin) { & $simplifyBin --version }

Write-Host ""
Write-Host "Success! Installed ahma-mcp and ahma-simplify to $installDir"
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
    Write-Host ""
    $platformsInput = Read-Host "  Selection [default: 1,2,3,4 -- all]"
    if ([string]::IsNullOrWhiteSpace($platformsInput)) { $platformsInput = '1,2,3,4' }
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
# Installs the ahma-simplify agent skill to ~/.agents/skills/ahma-simplify/SKILL.md
# Compatible with VS Code (GitHub Copilot), Cursor, and Claude Code — all index .agents/skills/
# ─────────────────────────────────────────────────────────────────────────────

function Get-AhmaSkillContent {
    return @'
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

**Option A -- MCP tool (preferred in VS Code / Cursor / Claude Code):**
The `simplify` tool is active in the current ahma-mcp session (enabled with `--tools simplify`
or `--tools rust,simplify`).

**Option B -- Direct CLI:**
`ahma-simplify` is on PATH. Install: `cargo install --path ahma_simplify` or use the install
script at `scripts/install.sh` / `scripts/install.ps1`.

To check availability, run: `ahma-simplify --version`

---

## Core Workflow

Follow this sequence. Do not skip steps.

### Step 1 -- Run complexity analysis

**Via MCP tool:**
```
simplify(directory="<project-root>", ai_fix=1)
```

**Via CLI:**
```
ahma-simplify <project-root> --ai-fix 1
```

The tool returns:
1. An overall project simplicity score (0-100%)
2. A ranked list of files by complexity (worst first)
3. Function-level hotspots for the top issue (top 5 functions by cognitive complexity)
4. A structured fix prompt for that specific file

### Step 2 -- Read the structured fix prompt

The `--ai-fix N` output ends with a structured prompt block. It contains:
- The exact file path to edit
- The hotspot functions (by name, line range, and metrics)
- Constraints for what to change

**Follow the prompt's constraints exactly:**
- Edit **only** the listed hotspot functions
- Do not refactor the whole file
- Do not change function signatures, public APIs, or behavior
- Do not reduce line count at the expense of clarity

### Step 3 -- Apply targeted changes

Common patterns for reducing complexity:
- Extract deeply nested logic into well-named helper functions
- Replace complex boolean chains with named predicates
- Replace multi-branch match/switch arms with a lookup table or strategy function
- Flatten early-return cascades (guard clauses)
- Break apart functions with high SLOC alongside high cognitive complexity

**For test files specifically:** If a hotspot is a test file with many small test functions,
skip it. High test count is expected; do not consolidate tests. If a single test function is
individually complex (large setup, many assertions), consider splitting it.

### Step 4 -- Verify improvement

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
- **Significant improvement** (>=10% score gain) -- success, continue
- **Modest improvement** (1-9% gain) -- acceptable, move to next issue
- **No change** -- review if the hotspot functions were actually modified
- **Regression** -- revert and try a different approach

### Step 5 -- Iterate

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
| `directory` | path (required) | -- | Project root to analyze |
| `ai_fix` | integer | -- | Issue number to generate fix prompt for (1 = worst file) |
| `limit` | integer | 50 | Number of issues to include in report |
| `verify` | path | -- | Re-analyze a file and compare to baseline |
| `extensions` | array | all | Restrict to specific file types (e.g. ["rs","py"]) |
| `exclude` | array | -- | Additional glob patterns to exclude |
| `output_path` | path | -- | Write report to directory instead of stdout |
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

```
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
ahma-simplify . --exclude "**/generated/**,**/vendor/**" --ai-fix 1
```
'@
}

function Invoke-AhmaSkillSetup {
    Write-Host ""
    Write-Host "======================================================="
    Write-Host "  Agent Skill Setup"
    Write-Host "======================================================="
    Write-Host ""
    Write-Host "  The ahma-simplify skill teaches AI agents (VS Code, Cursor, Claude Code)"
    Write-Host "  to analyze code complexity, fix hotspot functions, and verify improvement."
    Write-Host "  It installs to ~/.agents/skills/ -- the universal cross-platform skill path."
    Write-Host ""
    $choice = Read-Host "Install the ahma-simplify agent skill? [Y/n]"
    if ($choice -match '^[Nn]') { return }

    $skillDir  = Join-Path $HOME '.agents' 'skills' 'ahma-simplify'
    $skillPath = Join-Path $skillDir 'SKILL.md'

    # ── Check for existing installation ────────────────────────────────────
    if (Test-Path $skillPath) {
        $existingVer = (Select-String -Path $skillPath -Pattern '^version:' | Select-Object -First 1).Line
        $existingVer = if ($existingVer) { ($existingVer -split '\s+')[1].Trim("'`"") } else { 'unknown' }
        $newVer = '1.0.0'
        Write-Host ""
        if ($existingVer -eq $newVer) {
            Write-Host "  ahma-simplify skill v$newVer is already installed at:"
            Write-Host "  $skillPath"
            Write-Host ""
            $confirm = Read-Host "  Reinstall (overwrite)? [y/N]"
            if ($confirm -notmatch '^[Yy]') { Write-Host "  Skipped."; return }
        } else {
            Write-Host "  Existing skill v$existingVer found. Installing v$newVer."
        }
    }

    # ── Create skill directory and write SKILL.md ───────────────────────────
    New-Item -ItemType Directory -Force -Path $skillDir | Out-Null
    Get-AhmaSkillContent | Set-Content -Encoding UTF8 -Path $skillPath

    Write-Host ""
    Write-Host "  Skill installed: $skillPath"
    Write-Host ""
    Write-Host "  This skill is automatically available in:"
    Write-Host "    * VS Code (GitHub Copilot) -- auto-loaded when relevant"
    Write-Host "    * Cursor -- attach with @ahma-simplify in chat"
    Write-Host "    * Claude Code -- loaded from ~/.agents/skills/"
    Write-Host ""
    Write-Host "  To also enable per-project (commit to your repo):"
    Write-Host "    New-Item -ItemType Directory -Force .agents\skills\ahma-simplify"
    Write-Host "    Copy-Item $skillPath .agents\skills\ahma-simplify\SKILL.md"
    Write-Host ""
}

# Run the MCP setup wizard
Invoke-AhmaMcpSetup

# Run the skill setup wizard
Invoke-AhmaSkillSetup
