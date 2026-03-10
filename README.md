# Ahma

_Create MCP tools agents from your command line tools with one JSON file, then watch them complete your work faster with **true multi-threaded tool-use agentic AI workflows**. Built with a **security-first** philosophy, enforcing hard kernel-level boundaries by default._

|                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |                                     |
| ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------: |
| [![CI](https://github.com/paulirotta/ahma/actions/workflows/build.yml/badge.svg)](https://github.com/paulirotta/ahma/actions/workflows/build.yml) [![Coverage Report](https://img.shields.io/badge/Coverage-Report-blue)](https://paulirotta.github.io/ahma/html/) [![Rust Docs](https://img.shields.io/badge/Rust-Docs-blue)](https://paulirotta.github.io/ahma/doc/) [![Code Simplicity](https://img.shields.io/badge/Code-Simplicity-green)](https://paulirotta.github.io/ahma/CODE_SIMPLICITY.html) [![Prebuilt Binaries](https://img.shields.io/badge/Prebuilt-Binaries-blueviolet)](https://github.com/paulirotta/ahma/actions/workflows/build.yml?query=branch%3Amain+event%3Apush+is%3Asuccess) [![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT) [![License: Apache: 2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://www.apache.org/licenses/LICENSE-2020) [![Rust](https://img.shields.io/badge/Rust-1.93%2B-B7410E.svg)](https://www.rust-lang.org/) | ![Ahma Logo](./assets/ahma.png) |

Ahma is
- **secure by default**: this is a toolbox for AI to use command line tools safely. It helps move past the  'do you trust this tool/author?' prompts. Trust is not a security model. Asking the user working fast on several tasks for permission to `rm -rf ~` is irresponsible, not a security model.
- **fast by default**: command line tool calls become *deterministic and asynchronous subagents*. This saves time by allowing AI agents to continue thinking and planning while awaiting one or more long-running command line tasks.
- **principle of least privilege (PoLP)**: You may optionally disalbe direct calls to `sandboxed_shell` and instead specify the allowed arguments to each command line tool by creating a `.ahma/toolname.json` file.
- **batteries included**: Bundled tools can be selectively enabled, e.g. `--simplify` in your `mcp.json` for vibe code complexity reduction to improve maintainability.
- **flexible**: Supporting agentic development workflows or powering business agents are just two use cases. The rest is up to your creative imagination.
- **actively developed**: We are currently smoothing out the edges and adding features like deterministic tool  use, progressive tool disclosure and live log monitoring to proactively inform AI agents of issues as they occur.

### What Ahma does

Ahma complements developer and business agent tools by adding security and async execution.
MCP Clients such as developer IDEs and CLIs (Antigravity, Claude, Codex, Cursor, Open Code, Roo, VS Code, etc.) often have a built-in terminal that the AI can use. That terminal is powerful but often not sandboxed or easy to by default scope down the blast radius of AI errors and attacks. Business agent frameworks generally do not offer a terminal for AI to use.

| Capability | Native IDE/CLI terminal | Ahma `sandboxed_shell` |
|---|---|---|
| **Write protection** | None — full filesystem access | Kernel-enforced to workspace only (Seatbelt on macOS, Landlock on Linux) |
| **Async execution** | Synchronous — AI blocks until done | Async-first — AI continues working while commands run in background |
| **Parallel operations** | Sequential tool calls | True concurrent operations with per-operation status tracking |
| **Structured tool schema** | Raw shell strings | Typed parameters, validation, subcommands via `.ahma/*.json` |
| **Progressive disclosure** | All tools always listed | Bundles revealed on demand — preserves AI context window |
| **Live log monitoring** | Raw output only | Pattern-matched alerts streamed to AI (error/warn/info levels) |
| **PoLP enforcement** | Any command, any argument | Optionally restrict to declared arguments per tool |

## Supported OS

Linux and macOS are fully supported with kernel-level sandboxing. **Windows x64 support is in active development** — the runtime (PowerShell-first shell pool, file URI parsing) is implemented but the Windows sandbox backend is not yet complete. Windows builds fail closed in strict mode until the sandbox backend ships.

Raspberry Pi is supported with `--no-sandbox` until the kernel is updated to support Landlock (kernel ≥ 5.13).

**Windows requirements (when available):** PowerShell 7+ (`pwsh`) must be installed — download from https://aka.ms/powershell

## Installation Script

The installation script detects your OS and architecture, downloads the latest release from GitHub, and installs `ahma-mcp` and `ahma-simplify` to your local bin directory.

**Supported platforms:** Linux x86_64, Linux ARM64, Linux ARMv7 (Raspberry Pi 2/3), macOS ARM64 (Apple Silicon), Windows x86_64 (in-progress). Musl builds are available for Linux x86_64 and ARM64 (auto-detected on Alpine/musl systems, or set `AHMA_PREFER_MUSL=1`). Windows releases are distributed as `.zip` archives.

**Linux / macOS** — installs to `~/.local/bin`:

```bash
curl -sSf https://raw.githubusercontent.com/paulirotta/ahma/main/scripts/install.sh | bash
```

**Windows (PowerShell 5.1+)** — installs to `$HOME\.local\bin`:

```powershell
irm https://raw.githubusercontent.com/paulirotta/ahma/main/scripts/install.ps1 | iex
```

## Source Installation

**Linux / macOS:**

```bash
git clone https://github.com/paulirotta/ahma.git
cd ahma
cargo build --release
mv target/release/ahma-mcp /usr/local/bin/
mv target/release/ahma-simplify /usr/local/bin/
```

**Windows (PowerShell):**

```powershell
git clone https://github.com/paulirotta/ahma.git
cd ahma
cargo build --release
Copy-Item target\release\ahma-mcp.exe, target\release\ahma-simplify.exe "$HOME\.local\bin\"
```

## Concepts

If you are an AI agent interacting with this repository:
- **Sandbox Boundary**: You have full access within `${workspaceFolder}` but zero access outside it. Use `sandboxed_shell` for multi-step tasks.
- **Async Concurrency**: Most tools are async by default. Use `status` to monitor progress and continue with other tasks.
- **MTDF Schema**: Reference [docs/mtdf-schema.json](docs/mtdf-schema.json) defines the schema for creating or modifying tool configurations in `.ahma/*.json`.

## Key Features

- **Kernel-Level Sandboxing**: Security by default. Hard kernel boundaries prevent accessing any files outside the workspace, regardless of how an AI constructs its commands.
- **Asynchronous By Default with Sync Override**: Operations run asynchronously by default, allowing the LLM to continue work while awaiting results. **Automatic async** reduces round-trips for fast commands: if an async operation completes within 5 seconds, its result is returned inline without requiring a separate `await` call. Use `--sync` flag or set `"synchronous": true` in tool config for operations that must complete before proceeding. Supports multiple concurrent long-running operations (builds, tests).
- **Easy Tool Definition**: Add any command-line tool to your AI's arsenal by creating a single JSON file. No recompilation needed.
- **Multi-Step Workflows (Preferred)**: Run multi-command pipelines via `sandboxed_shell` (e.g., `cargo fmt --all && cargo clippy --all-targets && cargo test`).

## Security Sandbox

Security by default is a project core value. Unlike other tools that "yolo" system access or rely on easily bypassed string filters, Ahma enforces hard kernel-level boundaries. The sandbox scope is set once at server startup and cannot be changed during the session—the AI has full access within the sandbox but zero access outside it.

### Why Kernel-Level Sandboxing?

Security by default is a core value of this project. While other tools rely on fragile string-based filtering that AI can easily bypass, Ahma enforces hard kernel-level boundaries that cannot be circumvented. This prevents "yolo" security risks where users might otherwise trust an AI with broad system access.

### Sandbox Scope

The sandbox scope defines the root directory boundary for all file system operations:

- **STDIO mode**: Defaults to the current working directory when the server starts. The IDE sets `cwd` in `mcp.json` to `${workspaceFolder}`, so this "just works".
- **HTTP mode**: Set once when the HTTP server starts (cannot be changed during session). Configure via:
  1. `--sandbox-scope <path>` command-line parameter (highest priority)
  2. `AHMA_SANDBOX_SCOPE` environment variable
  3. Current working directory when server starts (default)
- **Single sandbox per server**: In HTTP mode, all clients share the same sandbox. For multiple projects, run separate server instances.

### Platform-Specific Enforcement

#### Linux (Landlock)

On Linux, Ahma uses [Landlock](https://docs.kernel.org/userspace-api/landlock.html) for kernel-level file system sandboxing. No additional installation is required—Landlock is built into Linux kernels 5.13 and newer.

**Requirements**: 
- **Linux kernel 5.13 or newer** (released June 2021)
- The server will **refuse to start** on older kernels unless sandbox is explicitly disabled
- For legacy kernels, you can run with `--no-sandbox` or `AHMA_NO_SANDBOX=1`; Ahma logs a warning and runs without its internal sandbox
- Check your kernel version with: `uname -r`

#### Raspberry Pi and Older Kernels

Some platforms (notably older Raspberry Pi OS kernels) ship with kernels that
do not support the Landlock LSM required for Ahma's Linux sandboxing. If your
kernel is older than 5.13 or `/sys/kernel/security/lsm` does not list
`landlock`, Ahma's kernel-enforced sandbox cannot be applied.

Recommended options when running on such systems:

- Quick check (shows kernel):

```bash
uname -r
cat /sys/kernel/security/lsm  # optional - shows active LSMs
```

- Run Ahma in compatibility (unsandboxed) mode explicitly using the env var
  or CLI flag. This makes the system usable on older kernels but disables
  Ahma's internal sandbox protections.

```bash
# Environment variable (preferred for mcp.json / CI setups)
export AHMA_NO_SANDBOX=1
ahma-mcp --mode http --tools-dir .ahma

# Or use the CLI flag
ahma-mcp --no-sandbox --mode http --tools-dir .ahma
```

- **Raspberry Pi OS**: Older kernels (pre-5.13) do not support Landlock. Ahma requires `--no-sandbox` (or `AHMA_NO_SANDBOX=1`) until the kernel is upgraded to a version that supports Landlock.

Security note: running with `AHMA_NO_SANDBOX=1` or `--no-sandbox` disables
Ahma's kernel-level protections. Only use this mode on machines that are
already sandboxed or otherwise trusted (for example, inside a container or
managed development environment). Prefer upgrading the kernel to 5.13+ where
possible to restore full Landlock enforcement.

#### macOS (Seatbelt/sandbox-exec)

On macOS, Ahma uses Apple's built-in `sandbox-exec` with [Seatbelt profiles](https://reverse.put.as/wp-content/uploads/2011/09/Apple-Sandbox-Guide-v1.0.pdf) for kernel-level file system sandboxing. No additional installation is required—`sandbox-exec` is built into macOS.

**Requirements**: Any modern version of macOS. The server generates a Seatbelt profile (SBPL) that restricts write access to only the sandbox scope.

### Nested Sandbox Environments (Cursor, VS Code, Docker)

When running inside another sandboxed environment (like Cursor IDE, VS Code, or Docker), Ahma automatically detects that it cannot apply its own sandbox. By default, Ahma exits with clear instructions to explicitly disable its internal sandbox. This is because:

- **Cursor/VS Code**: These IDEs sandbox MCP servers, preventing nested `sandbox-exec` calls on macOS
- **Docker**: Containers may restrict the system calls needed for sandboxing

**Auto-detection**: Ahma tests if `sandbox-exec` can be applied at startup. If it detects a nested sandbox, it fails fast with guidance to use `--no-sandbox` or `AHMA_NO_SANDBOX=1`.

**Manual override**: You can also explicitly disable Ahma's sandbox:

```bash
# Via command-line flag
ahma-mcp --no-sandbox

# Via environment variable (useful for Raspberry Pi or older kernels)
export AHMA_NO_SANDBOX=1
ahma-mcp
```

Example `mcp.json` for VS Code:

```json
{
    "servers": {
        "Ahma": {
            "type": "stdio",
            "command": "ahma-mcp",
            "args": [
                "--tmp",
                "--livelog",
                "--simplify"
            ]
        }
    }
}```

Example `mcp.json` for Cursor:

```json
{
    "mcpServers": {
        "Ahma": {
            "type": "stdio",
            "command": "ahma-mcp",
            "args": [
                "--tmp",
                "--livelog",
                "--simplify"
            ]
        }
    }
}
```


Example `mcp.json` for Claude Code:

```json
{
    "mcpServers": {
        "Ahma": {
            "type": "stdio",
            "command": "ahma-mcp",
            "args": [
                "--tmp",
                "--livelog",
                "--simplify"
            ]
        }
    }
}
```


Antigravity does not send the working directory in 'roots' so sandbox scoping must be manual or use '--no-sandbox'

Example `mcp.json` for Antigravity:

```json
{
  "mcpServers": {
    "Ahma": {
      "command": "bash",
      "args": [
        "-c",
        "ahma-mcp --simplify --rust --sandbox-scope $HOME/github"
      ]
    }
  }
}
```

## MCP Server Connection Modes

`ahma-mcp` supports three modes of operation:

### 1. STDIO Mode (Default)

Direct MCP server over stdio for integration with MCP clients:

```bash
ahma-mcp --mode stdio --tools-dir ./tools
```

### 2. HTTP Bridge Mode

HTTP server that proxies requests to the stdio MCP server:

```bash
# Start HTTP bridge on default port (3000)
# Clients that provide roots/list can lock sandbox from roots
ahma-mcp --mode http

# Explicit sandbox scope for clients like Antigravity and LM Studio that do not send roots/list
ahma-mcp --mode http --sandbox-scope /path/to/your/project

# Or via environment variable
export AHMA_SANDBOX_SCOPE=/path/to/your/project
ahma-mcp --mode http

# Custom port
ahma-mcp --mode http --http-port 8080 --http-host 127.0.0.1
```

**Important**: In HTTP mode, sandbox scope is bound **per client session** via the MCP `roots/list` protocol.
If a client does not provide roots/list, you **must** configure an explicit scope via `--sandbox-scope` or `AHMA_SANDBOX_SCOPE`.

**Security Note**: HTTP mode is for local development only. Do not expose to untrusted networks.

| Feature | STDIO Mode | HTTP Mode |
|---|---|---|
| Sandbox scope | Set by IDE via `cwd` in mcp.json | Set per session via `roots/list` |
| Per-project isolation | Automatic (IDE spawns per workspace) | Automatic (per session) |
| Configuration | `mcp.json` in IDE | CLI args or env vars |
| Use case | Standard IDE integration | Debugging, advanced setups |

For most users, **STDIO mode is recommended** because the IDE handles sandbox configuration automatically.

Send JSON-RPC requests to `http://localhost:3000/mcp`:

```bash
curl -X POST http://localhost:3000/mcp \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}'
```

## Contributing

Issues and pull requests are welcome. This project is AI friendly and provides the following:

- **`AGENTS.md`/`CLAUDE.md`**: Instructions for AI agents to use the MCP server to contribute to the project.
- **`SPEC.md`**: This is the **single source of truth** for the project requirements. AI keeps it up to date as you work on the project.

## License

Licensed under either [Apache License 2.0](APACHE_LICENSE.txt) or [MIT License](MIT_LICENSE.txt).
