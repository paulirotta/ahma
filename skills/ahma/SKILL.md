---
name: ahma
version: 0.6.0
author: Paul Houghton
description: >
  Comprehensive guide for using Ahma (ahma-mcp) as an AI agent. USE THIS SKILL when you need
  to understand how to run tools, activate bundles, use the sandbox, monitor logs, author custom
  tools, or configure ahma-mcp. Also handles code complexity analysis via `/ahma simplify`.
  Trigger phrases: "use ahma", "run with ahma", "ahma tool", "activate bundle",
  "sandboxed_shell", "ahma async", "ahma serve", "mcp.json ahma", "ahma sandbox",
  "ahma livelog", "ahma monitor", "custom tool .ahma", "ahma-mcp", "await tool",
  "cancel operation", "tool bundle", "progressive disclosure", "activate_tools",
  "simplify", "reduce complexity", "too complex", "hard to read", "refactor",
  "maintainability", "cognitive complexity", "cyclomatic complexity", "simplicity score",
  "code quality metrics", "hotspot", "ahma simplify", "ahma help", "ahma ?".
user-invocable: true
---

<!-- version: 0.6.0 | author: Paul Houghton -->

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

By default, Ahma shows only built-in tools (`sandboxed_shell`, `status`, `await`) plus
`activate_tools` (when bundles are loaded but not yet specified via `--tools`).
Bundles specified with `--tools` are **always revealed immediately** — no extra flag needed.
Bundles NOT in `--tools` remain hidden and can be unlocked on demand via `activate_tools`.

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

**Specify bundles at startup** (tools visible immediately — no extra step required):
```json
"args": ["serve", "stdio", "--tools", "rust,git,fileutils"]
```

**Disable progressive disclosure entirely** (show all loaded tools, no `activate_tools`):
```json
"args": ["serve", "stdio", "--tools", "rust,git", "--disable-progressive-disclosure"]
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

---

## User-Invocable Subcommands

The `/ahma` skill supports these user-invocable subcommands in chat:

| Command | Alias | Purpose |
|---------|-------|---------|
| `/ahma help` | `/ahma ?` | List all available subcommands and their usage |
| `/ahma simplify [lang] [n]` | — | Analyze code complexity and get fix instructions |

---

## `/ahma help` — List Subcommands

When the user types `/ahma help` or `/ahma ?`, respond with a concise list of all available
user-invocable subcommands and a one-line description of each:

```
/ahma help         — Show this help list
/ahma ?            — Alias for /ahma help
/ahma simplify     — Analyze code complexity and get AI fix instructions for the worst file
```

Also mention the key flags for configure, e.g., `--tools`, `--tmp`, `--log-monitor`.

---

## `/ahma simplify` — Code Complexity Analysis

When the user types `/ahma simplify [language] [n]`, run the full code complexity workflow.

### Syntax

```
/ahma simplify              # Analyze all supported file types, fix worst file (#1)
/ahma simplify rust         # Rust files only
/ahma simplify kotlin 2     # Kotlin files, get fix prompt for 2nd worst file
/ahma simplify rust python  # Multiple languages
```

Language names are case-insensitive and expand to their extensions automatically.
A trailing integer sets the `ai_fix` issue number (default: 1).

### Supported Languages

| Name | Extensions |
|------|------------|
| `rust` | `.rs` |
| `kotlin` | `.kt`, `.kts` |
| `python` | `.py` |
| `javascript` | `.js`, `.jsx` |
| `typescript` | `.ts`, `.tsx` |
| `java` | `.java` |
| `c++` / `cpp` | `.cpp`, `.cc`, `.hpp`, `.hh` |
| `c#` / `csharp` | `.cs` |
| `go` | `.go` |
| `html` | `.html`, `.htm` |
| `css` | `.css` |

### Prerequisites

**Via MCP tool (preferred):** The `simplify` tool must be active — start Ahma with `--tools simplify`
or `--tools rust,simplify`.

**Via CLI:** `ahma-mcp simplify` is the subcommand. Run `ahma-mcp simplify --help` to verify.

### Workflow — Follow This Sequence

#### Step 1 — Run complexity analysis

**Via MCP tool:**
```
simplify(directory="<project-root>", ai_fix=1)
```

**Via CLI:**
```bash
ahma-mcp simplify <project-root> --ai-fix 1
```

The output contains:
1. Overall project simplicity score (0–100%)
2. Ranked file list (worst first)
3. Function-level hotspots for the top issue (top 5 by cognitive complexity)
4. A structured fix prompt for that specific file

#### Step 2 — Read and follow the structured fix prompt

The `--ai-fix N` output ends with a structured prompt. It specifies:
- The exact file path to edit
- Hotspot functions (name, line range, metrics)
- Constraints on what to change

**Follow the prompt's constraints exactly:**
- Edit **only** the listed hotspot functions
- Do not refactor the whole file
- Do not change function signatures, public APIs, or behavior

#### Step 3 — Apply targeted changes

Common complexity-reduction patterns:
- Extract deeply nested logic into well-named helper functions
- Replace complex boolean chains with named predicates
- Replace long match/switch arms with lookup tables
- Flatten early-return cascades (guard clauses)

**For test files:** High test count is expected. Skip unless a single test function is individually complex.

#### Step 4 — Verify improvement

**Via MCP tool:**
```
simplify(directory="<project-root>", verify="<path-to-edited-file>")
```

**Via CLI:**
```bash
ahma-mcp simplify <project-root> --verify <path-to-edited-file>
```

| Verdict | Meaning |
|---------|---------|
| Significant improvement (≥10%) | Success — move to next issue |
| Modest improvement (1–9%) | Acceptable |
| No change | Hotspot functions may not have been modified |
| Regression | Revert and try a different approach |

#### Step 5 — Iterate

```
simplify(directory="<project-root>", ai_fix=2)
```

Continue until the project score is satisfactory.

### Score Interpretation

```
Score = 0.4 × MI + 0.3 × Cognitive Density + 0.2 × Peak Cognitive + 0.1 × Length Score
```

| Score | Status | Action |
|-------|--------|--------|
| 85–100% | Excellent | No action needed |
| 70–84% | Good | Fix only worst outliers |
| 55–69% | Fair | Plan a simplification sprint |
| 40–54% | Poor | Prioritize before new features |
| 0–39% | Critical | Address now |

### MCP Tool Reference (`simplify`)

| Argument | Type | Default | Purpose |
|----------|------|---------|---------|
| `directory` | path (required) | — | Project root to analyze |
| `ai_fix` | integer | — | Issue number for fix prompt (1 = worst file) |
| `limit` | integer | 50 | Issues to include in report |
| `verify` | path | — | Re-analyze a file vs. baseline |
| `extensions` | array | all | Restrict to file types (e.g. `["rs","py"]`) |
| `exclude` | array | — | Additional glob patterns to exclude |
| `output_path` | path | — | Write report to directory instead of stdout |
| `html` | boolean | false | Also generate HTML report |

### CLI Quick Reference

```bash
# Analyze and get fix prompt for worst file
ahma-mcp simplify . --ai-fix 1

# Rust files only
ahma-mcp simplify . --extensions rust --ai-fix 1

# Multiple languages
ahma-mcp simplify . --extensions rust,python --ai-fix 1

# 2nd worst file
ahma-mcp simplify . --ai-fix 2

# Verify improvement after editing
ahma-mcp simplify . --verify src/my_module.rs

# Full report to file
ahma-mcp simplify . --output-path ./reports

# HTML report
ahma-mcp simplify . --html

# Exclude generated code
ahma-mcp simplify . --exclude '**/generated/**,**/vendor/**' --ai-fix 1
```

### Anti-Patterns to Avoid

1. **Do not refactor the whole file** — follow the hotspot list exactly.
2. **Do not add comments to improve scores** — structural change is needed.
3. **Do not inline complex logic** — fewer functions with more complexity each makes scores worse.
4. **Do not run `--ai-fix` without reading the structured prompt.**
5. **Do not skip Step 4 (verify)** — complexity improvements must be confirmed by metrics.

---

**See also**: [security-sandbox.md](https://github.com/paulirotta/ahma/blob/main/docs/security-sandbox.md) ·
[live-log-monitoring.md](https://github.com/paulirotta/ahma/blob/main/docs/live-log-monitoring.md) ·
[connection-modes.md](https://github.com/paulirotta/ahma/blob/main/docs/connection-modes.md) ·
[environment-variables.md](https://github.com/paulirotta/ahma/blob/main/docs/environment-variables.md) ·
[mtdf-schema.json](https://github.com/paulirotta/ahma/blob/main/docs/mtdf-schema.json) ·
[SIMPLIFY.md](https://github.com/paulirotta/ahma/blob/main/SIMPLIFY.md)
