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
                '$env:AHMA_SANDBOX_SCOPE = $env:USERPROFILE; $env:AHMA_TMP_ACCESS = ''1''; $env:AHMA_LOG_MONITOR = ''1''; ahma-mcp serve stdio --tools rust,simplify'
            )
        }
    } else {
        return [ordered]@{
            type    = 'stdio'
            command = 'ahma-mcp'
            args    = @('serve', 'stdio', '--tools', 'rust,simplify')
            env     = [ordered]@{
                AHMA_TMP_ACCESS  = '1'
                AHMA_LOG_MONITOR = '1'
            }
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
    Write-Host "  1) VS Code      ($HOME\.vscode\mcp.json)"
    Write-Host "  2) Claude Code  ($HOME\.claude\mcp.json)"
    Write-Host "  3) Cursor       ($HOME\.cursor\mcp.json)"
    Write-Host "  4) Antigravity  ($HOME\.antigravity\mcp.json)"
    Write-Host ""
    $platformsInput = Read-Host "  Selection [default: 1,2,3,4 -- all]"
    if ([string]::IsNullOrWhiteSpace($platformsInput)) { $platformsInput = '1,2,3,4' }
    $selectedNums = $platformsInput -split ',' | ForEach-Object { $_.Trim() }

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

    # ── Configure each selected platform ───────────────────────────────────
    $configuredTools = [ref]''

    $platforms = @(
        @{ Num = '1'; Display = 'VS Code';     Path = "$HOME\.vscode\mcp.json";      Key = 'servers';    Type = 'standard'    },
        @{ Num = '2'; Display = 'Claude Code'; Path = "$HOME\.claude\mcp.json";      Key = 'mcpServers'; Type = 'standard'    },
        @{ Num = '3'; Display = 'Cursor';      Path = "$HOME\.cursor\mcp.json";      Key = 'mcpServers'; Type = 'standard'    },
        @{ Num = '4'; Display = 'Antigravity'; Path = "$HOME\.antigravity\mcp.json"; Key = 'mcpServers'; Type = 'antigravity' }
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

# Run the MCP setup wizard
Invoke-AhmaMcpSetup
