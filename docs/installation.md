# Installation

The installation scripts detect your OS and architecture, download the latest release from GitHub, and install `ahma-mcp` and `ahma-simplify` to your local bin directory.

## Install with the script

**Linux / macOS** — installs to `~/.local/bin`:

```bash
curl -sSf https://raw.githubusercontent.com/paulirotta/ahma/main/scripts/install.sh | bash
```

**Windows (PowerShell 5.1+)** — installs to `$HOME\.local\bin`:

```powershell
irm https://raw.githubusercontent.com/paulirotta/ahma/main/scripts/install.ps1 | iex
```

Supported binary platforms: Linux x86_64, Linux ARM64, Linux ARMv7 (Raspberry Pi 2/3), macOS ARM64 (Apple Silicon), Windows x86_64 (in progress). Musl builds are available for Linux x86_64 and ARM64. Windows releases are distributed as `.zip` archives.

## What the installer does

After installation, the script offers an interactive MCP setup wizard that can configure ahma as a global MCP server for supported clients such as VS Code, Claude Code, Cursor, and Antigravity.

The wizard:

- lets you choose which clients to configure
- lets you choose stdio or HTTP connection mode
- shows the proposed `mcp.json` changes before writing them
- can install the optional `ahma-simplify` skill

If you want the optional skill setup details, see [docs/agent-skills.md](agent-skills.md).

## Build from source

**Linux / macOS**

```bash
git clone https://github.com/paulirotta/ahma.git
cd ahma
cargo build --release
mv target/release/ahma-mcp /usr/local/bin/
mv target/release/ahma-simplify /usr/local/bin/
```

**Windows (PowerShell)**

```powershell
git clone https://github.com/paulirotta/ahma.git
cd ahma
cargo build --release
Copy-Item target\release\ahma-mcp.exe, target\release\ahma-simplify.exe "$HOME\.local\bin\"
```

## After installation

For client configuration examples, see [docs/connection-modes.md](connection-modes.md).

For the practical day-to-day usage model and sandbox behavior, return to [README.md](../README.md) and [docs/security-sandbox.md](security-sandbox.md).