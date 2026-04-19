# Ahma

_Use your existing command line workflows through MCP with a repo-scoped sandbox, async execution, and less pressure to fall back to insecure terminal access._

## Why Ahma helps

- **When the agent only needs the repo, broad terminal access is too much**: ahma starts inside a kernel-enforced workspace boundary, so normal project work does not require wider filesystem access.
- **When builds, tests, and checks take time, blocked agents waste time**: ahma runs commands async-first so long-running work can continue in the background while the agent keeps moving.
- **When independent tasks are forced through one terminal, work gets serialized**: ahma can start separate operations concurrently and track them cleanly.
- **When safety is noisy, people disable it**: ahma aims to make the safe path the practical path, reducing pressure to use broad or insecure override modes just to get work done.

|                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |                                     |
| ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------: |
| [![CI](https://github.com/paulirotta/ahma/actions/workflows/build.yml/badge.svg)](https://github.com/paulirotta/ahma/actions/workflows/build.yml) [![Coverage Report](https://img.shields.io/badge/Coverage-Report-blue)](https://paulirotta.github.io/ahma/html/) [![Rust Docs](https://img.shields.io/badge/Rust-Docs-blue)](https://paulirotta.github.io/ahma/doc/) [![Code Simplicity](https://img.shields.io/badge/Code-Simplicity-green)](https://paulirotta.github.io/ahma/CODE_SIMPLICITY.html) [![Prebuilt Binaries](https://img.shields.io/badge/Prebuilt-Binaries-blueviolet)](https://github.com/paulirotta/ahma/actions/workflows/build.yml?query=branch%3Amain+event%3Apush+is%3Asuccess) [![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT) [![License: Apache: 2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://www.apache.org/licenses/LICENSE-2020) [![Rust](https://img.shields.io/badge/Rust-1.93%2B-B7410E.svg)](https://www.rust-lang.org/) | ![Ahma Logo](./assets/ahma.png) |

Ahma is an MCP server for running real project work through existing CLI tools with tighter filesystem boundaries and less blocking. It is aimed at the common case: builds, tests, formatters, git operations, log inspection, and other deterministic command-line tasks that agents already try to run.

## Quickstart

**Linux / macOS**

```bash
curl -sSf https://raw.githubusercontent.com/paulirotta/ahma/main/scripts/install.sh | bash
```

**Windows (PowerShell 5.1+)**

```powershell
irm https://raw.githubusercontent.com/paulirotta/ahma/main/scripts/install.ps1 | iex
```

The installer downloads `ahma-mcp`, offers to configure your MCP client, and can install optional extras. See [docs/installation.md](docs/installation.md) for platform details, source installation, and what the setup wizard changes.

### Example workflow

Ask your agent to run a normal project task such as:

> Run formatters, linting, tests, and a build for this repo. Start independent steps concurrently where possible and keep me updated on failures.

With ahma, that workflow stays inside the repo boundary and the long-running steps can begin immediately as background operations. The agent can inspect results, continue other work, or start additional safe commands without waiting on one giant terminal session.

### Without ahma / with ahma

| Workflow detail | Without ahma | With ahma |
|---|---|---|
| **Filesystem access** | Often tied to a broad terminal with a larger blast radius | Kernel-enforced to the workspace scope |
| **Approval friction** | Repeated trust decisions or pressure to relax safety settings | Repo-scoped access is established up front |
| **Long-running work** | One blocked terminal session at a time | Async-first operations with status tracking |
| **Parallel tasks** | Often serialized | Independent tasks can start and run concurrently |
| **Operational visibility** | Raw terminal output | Operation IDs, progress notifications, and structured tool calls |

### What Ahma does

Ahma complements IDE and CLI MCP clients by making normal command-line work safer and less blocking. It is most useful where the client either exposes a broad terminal directly or has no terminal model at all.

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

## Source Installation

If you prefer to build from source:

**Linux / macOS**

```bash
git clone https://github.com/paulirotta/ahma.git
cd ahma
cargo build --release
mv target/release/ahma-mcp /usr/local/bin/
```

**Windows (PowerShell)**

```powershell
git clone https://github.com/paulirotta/ahma.git
cd ahma
cargo build --release
Copy-Item target\release\ahma-mcp.exe "$HOME\.local\bin\"
```

See [docs/installation.md](docs/installation.md) for supported binary platforms and installer behavior.

## Security Sandbox

Ahma enforces **kernel-level filesystem sandboxing** by default — Landlock on Linux, Seatbelt on macOS, Job Objects on Windows. The sandbox scope is set once at startup and cannot be changed. The AI has full access within the workspace, zero access outside it, unconditionally.

See [docs/security-sandbox.md](docs/security-sandbox.md) for platform details, nested sandbox detection, temp directory access, and example `mcp.json` configs.

## Configuration Reference

Sandbox scope, logging, execution behaviour, and HTTP transport options are all configured via environment variables. See **[docs/environment-variables.md](docs/environment-variables.md)** for the full reference, including a quick-reference table of every `AHMA_*` variable.

## Live Log Monitoring

Ahma can run any streaming command (e.g. `adb logcat`, `tail -f`, `docker logs -f`) through an LLM to detect issues in real time. The tool returns an operation ID immediately; alerts are pushed as MCP progress notifications whenever the LLM finds a problem matching your description.

See [docs/live-log-monitoring.md](docs/live-log-monitoring.md) for setup, the Android logcat example, and how to use cloud or local LLM providers.

## Optional advanced topics

- **Custom tools**: If you want to expose your own command-line tools through ahma, start with [docs/custom-tools.md](docs/custom-tools.md).
- **Agent skills**: Optional agent-specific setup is documented in [docs/agent-skills.md](docs/agent-skills.md).
- **Code complexity analysis**: `ahma-mcp simplify` analyzes source files and returns structured AI fix instructions. See [SIMPLIFY.md](SIMPLIFY.md).

## MCP Server Connection Modes

`ahma-mcp` supports **STDIO** (default — IDE spawns a subprocess per workspace), **HTTP Bridge** (proxy for web clients and debugging), and **HTTP Streaming** (MCP Streamable HTTP with event replay and full-duplex).

See [docs/connection-modes.md](docs/connection-modes.md) for `mcp.json` examples for VS Code, Cursor, Claude Code, and Antigravity, plus HTTP streaming usage.

## Contributing

Issues and pull requests are welcome. This project is AI friendly and provides the following:

- **`AGENTS.md`/`CLAUDE.md`**: Instructions for AI agents to use the MCP server to contribute to the project.
- **`SPEC.md`**: This is the **single source of truth** for the project requirements. AI keeps it up to date as you work on the project.

## License

Licensed under either [Apache License 2.0](APACHE_LICENSE.txt) or [MIT License](MIT_LICENSE.txt).
