# Ahma

_Create MCP tools agents from your command line tools with one JSON file, then watch them complete your work faster with **true multi-threaded tool-use agentic AI workflows**. Built with a **security-first** philosophy, enforcing hard kernel-level boundaries by default._

## Ahma solves

- **Unsafe AI terminal access**: Most AI terminal workflows rely on trust prompts, not real containment. Ahma enforces kernel-level sandbox boundaries so AI can operate inside your project scope without unrestricted filesystem access.
- **Agents and LLMs blocked by long-running synchronous commands**: Build, test, and deploy tasks can stall an agent for minutes. Ahma runs tool calls async-first, so agents can keep planning and executing while work completes in the background.
- **Slow tool onboarding for AI workflows**: Ahma makes your custom tools AI-friendly. It gives you no-code JSON definitions for CLI tools, with optional hot reload during tool development when you explicitly enable `--hot-reload-tools`.
- **Too much privilege by default**: Generic shell access is often broader than needed. Ahma supports least-privilege tool definitions so you can constrain arguments and reduce blast radius.
- **Do more work in less time**: Light up the full capabilities of your command line tools by telling AI to split your deterministic work into multiple background operations and fire them all at once. Ahma provides operation IDs, progress notifications, and built-in controls (`status`, `await`, `cancel`) for deterministic orchestration. Max concurrency is equal to the number of cores on your CPU.

|                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |                                     |
| ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------: |
| [![CI](https://github.com/paulirotta/ahma/actions/workflows/build.yml/badge.svg)](https://github.com/paulirotta/ahma/actions/workflows/build.yml) [![Coverage Report](https://img.shields.io/badge/Coverage-Report-blue)](https://paulirotta.github.io/ahma/html/) [![Rust Docs](https://img.shields.io/badge/Rust-Docs-blue)](https://paulirotta.github.io/ahma/doc/) [![Code Simplicity](https://img.shields.io/badge/Code-Simplicity-green)](https://paulirotta.github.io/ahma/CODE_SIMPLICITY.html) [![Prebuilt Binaries](https://img.shields.io/badge/Prebuilt-Binaries-blueviolet)](https://github.com/paulirotta/ahma/actions/workflows/build.yml?query=branch%3Amain+event%3Apush+is%3Asuccess) [![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT) [![License: Apache: 2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://www.apache.org/licenses/LICENSE-2020) [![Rust](https://img.shields.io/badge/Rust-1.93%2B-B7410E.svg)](https://www.rust-lang.org/) | ![Ahma Logo](./assets/ahma.png) |

## Ahma is
- **secure by default**: this is a toolbox for AI to use command line tools safely. It helps move past the  'do you trust this tool/author?' prompts. Trust is not a security model. Asking the user working fast on several tasks for permission to `rm -rf ~` is irresponsible, not a security model.
- **fast by default**: command line tool calls become *deterministic and asynchronous subagents*. HTTP/3 (QUIC) is the preferred transport for all HTTP clients, delivering lower latency through 0-RTT connection establishment and improved multiplexing. HTTP streaming (MCP Streamable HTTP) is the default transport for the HTTP bridge, enabling full-duplex communication with event replay. AI agents continue thinking and planning while awaiting one or more long-running command line tasks.
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
| **PoLP enforcement** | Any command, any argument | Call directly, or define a JSON file to restrict which arguments can be passed to a command line tool |

## OS Support

- **macOS** — Full support with kernel-level sandboxing (Seatbelt)
- **Linux (Ubuntu, RHEL)** — Intel and ARM. Full support with Landlock (kernel ≥ 5.13)
- **Raspberry Pi** — 64-bit and 32-bit. Use `--disable-sandbox` until kernel-level sandboxing is supported (Landlock requires kernel ≥ 5.13)
- **Windows** — Full support. Uses the built-in PowerShell (5.1+) included with Windows 10/11

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

After installing, the script offers an **interactive MCP setup wizard** that configures ahma as a global MCP server for your AI tools. You choose which platforms to configure (VS Code, Claude Code, Cursor, Antigravity), select stdio or HTTP connection mode, and the script creates or updates each tool's global `mcp.json` for you — showing the proposed changes and asking for confirmation before writing anything.

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
- **Multi-Step Workflows (Preferred)**: Run multi-command pipelines via `sandboxed_shell` (e.g., `cargo fmt --all && cargo clippy --all-targets && cargo nextest run`).

## Security Sandbox

Ahma enforces **kernel-level filesystem sandboxing** by default — Landlock on Linux, Seatbelt on macOS, Job Objects on Windows. The sandbox scope is set once at startup and cannot be changed. The AI has full access within the workspace, zero access outside it, unconditionally.

See [docs/security-sandbox.md](docs/security-sandbox.md) for platform details, nested sandbox detection, temp directory access, and example `mcp.json` configs.

## Configuration Reference

Sandbox scope, logging, execution behaviour, and HTTP transport options are all configured via environment variables. See **[docs/environment-variables.md](docs/environment-variables.md)** for the full reference, including a quick-reference table of every `AHMA_*` variable.

## Live Log Monitoring

Ahma can run any streaming command (e.g. `adb logcat`, `tail -f`, `docker logs -f`) through an LLM to detect issues in real time. The tool returns an operation ID immediately; alerts are pushed as MCP progress notifications whenever the LLM finds a problem matching your description.

See [docs/live-log-monitoring.md](docs/live-log-monitoring.md) for setup, the Android logcat example, and how to use cloud or local LLM providers.

## MCP Server Connection Modes

`ahma-mcp` supports **STDIO** (default — IDE spawns a subprocess per workspace), **HTTP Bridge** (proxy for web clients and debugging), and **HTTP Streaming** (MCP Streamable HTTP with event replay and full-duplex).

See [docs/connection-modes.md](docs/connection-modes.md) for `mcp.json` examples for VS Code, Cursor, Claude Code, and Antigravity, plus HTTP streaming usage.

## Contributing

Issues and pull requests are welcome. This project is AI friendly and provides the following:

- **`AGENTS.md`/`CLAUDE.md`**: Instructions for AI agents to use the MCP server to contribute to the project.
- **`SPEC.md`**: This is the **single source of truth** for the project requirements. AI keeps it up to date as you work on the project.

## License

Licensed under either [Apache License 2.0](APACHE_LICENSE.txt) or [MIT License](MIT_LICENSE.txt).
