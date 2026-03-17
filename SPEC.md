# Ahma Requirements

> **For AI Assistants:** This is the **single source of truth** for the project. Always read this before making changes. Update this file when requirements change, bugs are discovered, or implementation status changes.

## Quick Status

| Component | Status | Notes |
|-----------|--------|-------|
| Core Tool Execution | tests-pass | `ahma-mcp` adapter executes CLI tools via MTDF JSON |
| Async-First Operations | tests-pass | Operations return `id`, push results via MCP notifications |
| Shell Pool | tests-pass | Pre-warmed bash/PowerShell shells for 5-20ms command startup latency |
| Linux Sandbox (Landlock) | tests-pass | Kernel-level FS sandboxing on Linux 5.13+ |
| macOS Sandbox (Seatbelt) | tests-pass | Kernel-level FS sandboxing via `sandbox-exec` |
| Nested Sandbox Detection | tests-pass | Detects Cursor/VS Code/Docker outer sandboxes |
| Windows Runtime (PowerShell) | in-progress | Built-in PowerShell (5.1+) shell pool; cross-platform path security + file URI; parity tests green |
| Windows Sandbox backend | in-progress | Job Object enforcement done; AppContainer profile + DACL grant implemented (Windows CI validation required for `tests-pass`) |
| Windows Pre-built Releases | in-progress | `x86_64-pc-windows-msvc`; `.zip` CI artifacts; `install.ps1`; winget manifests + `job-publish-winget` CI job |
| STDIO Mode | tests-pass | Direct MCP server over stdio for IDE integration |
| HTTP Bridge Mode | tests-pass | HTTP/SSE proxy for web clients |
| HTTP Streaming (Streamable HTTP) | tests-pass | POST SSE with event IDs, event history, Last-Event-Id replay, full multiplexing |
| HTTP/3 (QUIC) Client Preference | tests-pass | All HTTP clients prefer HTTP/3 (QUIC) when server supports it; transparent fallback to HTTP/2 and HTTP/1.1 |
| Session Isolation (HTTP) | tests-pass | Per-session sandbox scope via MCP `roots/list` |
| Built-in `status` Tool | tests-pass | Non-blocking progress check for async operations |
| Built-in `await` Tool | tests-pass | Blocking wait for operation completion |
| Built-in `cancel` Tool | tests-pass | Cancel running operations |
| Built-in `sandboxed_shell` | tests-pass | Execute arbitrary shell commands within sandbox |
| Batteries-Included Tools | tests-pass | Built-in MTDF setups activated via CLI flags (e.g. `--rust`, `--python`) |
| MTDF Schema Validation | tests-pass | JSON schema validation at startup |
| Sequence Tools | tests-pass | Chain multiple commands into workflows |
| Tool Hot-Reload | tests-pass | Opt-in `--hot-reload-tools` watches `tools/` directory and reloads on changes |
| MCP Callback Notifications | tests-pass | Push async results via `notifications/progress` |
| HTTP MCP Client | tests-pass | Connect to external HTTP MCP servers |
| OAuth 2.0 + PKCE | tests-pass | Authentication for HTTP MCP servers |
| `ahma-mcp --validate` | tests-pass | Validate tool configs against MTDF schema |
| `generate-tool-schema` CLI | tests-pass | Generate MTDF JSON schema |
| Graceful Shutdown | tests-pass | 10-second grace period for operation completion |
| Unified Shell Output | tests-pass | stderr redirected to stdout (`2>&1`) |
| Logging (File + Stderr) | tests-pass | Daily rolling logs, `--log-to-stderr` for debug |

---

## 1. Project Overview

**Ahma** (Finnish for "wolverine") is a universal, high-performance **Model Context Protocol (MCP) server** designed to dynamically adapt any command-line tool for use by AI agents. Its purpose is to provide a consistent, powerful, and non-blocking bridge between AI and the vast ecosystem of command-line utilities.

_"Create agents from your command line tools with one JSON file, then watch them complete your work faster with **true multi-threaded tool-use agentic AI workflows**."_

### Technology Stack

| Tech | Version | Purpose |
|------|---------|---------|
| Rust | 2024 Edition (1.93+) | Core language |
| rmcp | 0.13.0 | MCP protocol implementation |
| Tokio | 1.x | Async runtime |
| Landlock | 0.4.4 | Linux kernel sandboxing |
| reqwest | 0.13.2 (http3) | HTTP client with HTTP/3 (QUIC) preference |
| schemars | 1.2.0 | JSON Schema generation |

---

## 2. Architecture

### 2.1 Core Modules

| Module | Purpose |
|--------|---------|
| `adapter` | Primary engine for executing external CLI tools (sync/async) |
| `mcp_service` | Implements `rmcp::ServerHandler` - handles `tools/list`, `tools/call`, etc. |
| `operation_monitor` | Tracks background operations (progress, timeout, cancellation) |
| `shell_pool` | Pre-warmed bash/PowerShell (5.1+) shells for 5-20ms command startup latency |
| `sandbox` | Kernel-level sandboxing (Landlock on Linux, Seatbelt on macOS) |
| `config` | MTDF (Multi-Tool Definition Format) configuration models |
| `callback_system` | Event notification system for async operations |
| `path_security` | Path validation for sandbox enforcement |

### 2.2 Built-in Internal Tools

These tools are always available regardless of JSON configuration:

| Tool | Description |
|------|-------------|
| `status` | Non-blocking progress check for async operations |
| `await` | Blocking wait for operation completion (use sparingly) |
| `cancel` | Cancel running operations |
| `sandboxed_shell` | Execute arbitrary shell commands within sandbox scope (promoted from file-based to internal) |

**Note**: These internal tools are hardcoded into the `AhmaMcpService` and are guaranteed to be available even when no `.ahma` directory exists or when all external tool configurations fail to load.

### 2.3 Async-First Architecture

```text
┌─────────────────┐         ┌──────────────────┐
│  AI Agent (IDE) │ ──MCP─▶ │  AhmaMcpService  │
└─────────────────┘         └────────┬─────────┘
                                     │
                    ┌────────────────┼────────────────┐
                    ▼                ▼                ▼
            ┌───────────┐    ┌───────────────┐  ┌─────────┐
            │  Adapter  │    │ OperationMon. │  │ Sandbox │
            └─────┬─────┘    └───────────────┘  └─────────┘
                  │
                  ▼
            ┌───────────────┐
            │  ShellPool    │ ──▶ Pre-warmed bash/PowerShell shells
            └───────────────┘
```

**Workflow:**

1. AI invokes tool → Server immediately returns `id`
2. Command executes in background via shell pool
3. On completion, result pushed via MCP `notifications/progress`
4. AI processes notification when it arrives (non-blocking)

### 2.4 Synchronous Setting Inheritance

```text
┌─────────────────────────────────────────────────────────────────┐
│                    EXECUTION MODE RESOLUTION                     │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  1. CLI Flag (highest priority)                                  │
│     └── --sync flag forces ALL tools to run synchronously        │
│                                                                  │
│  2. Subcommand Config                                            │
│     └── "synchronous": true/false in subcommand definition       │
│                                                                  │
│  3. Tool Config                                                  │
│     └── "synchronous": true/false at tool level                  │
│                                                                  │
│  4. Default (lowest priority)                                    │
│     └── ASYNC - operations run in background by default          │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

---

## 3. Core Requirements

### R1: Configuration-Driven Tools

- **R1.1**: The system **must** adapt any CLI tool for use as MCP tools based on declarative JSON configuration files.
- **R1.2**: All tool definitions **must** be stored in `.json` files within a `tools/` directory (default: `.ahma/`).
- **R1.2.1**: **Auto-Detection**: When `--tools-dir` is not explicitly provided, the system **must** check for a `.ahma` directory in the current working directory. If found, it **must** be used as the tools directory. If not found, the system **must** log a warning and operate with only the built-in internal tools (`await`, `status`, `sandboxed_shell`).
- **R1.2.2**: When `--tools-dir` is explicitly provided via CLI argument, that path **must** take precedence over auto-detection.
- **R1.3**: The system **must not** be recompiled to add, remove, or modify a tool.
- **R1.4**: **Hot-Reloading**: The system **must** watch the `tools/` directory and send `notifications/tools/list_changed` when files change.
- **R1.5**: **Progressive Disclosure** (default enabled): When progressive disclosure is active, `tools/list` **must** return only built-in tools (`await`, `status`, `sandboxed_shell`, `cancel`) and the `activate_tools` meta-tool. Bundled tools are hidden until their bundle is explicitly revealed.
- **R1.5.1**: The `activate_tools` meta-tool **must** support two actions: `list` (enumerate available bundles with name, description, tool count, and revealed status) and `reveal` (activate a named bundle).
- **R1.5.2**: After a bundle is revealed via `activate_tools reveal`, the server **must** send `notifications/tools/list_changed` and include the bundle's tools in subsequent `tools/list` responses.
- **R1.5.3**: The `--no-progressive-disclosure` CLI flag **must** restore legacy behavior where all enabled tools are listed immediately.
- **R1.5.4**: The `instructions` field in the MCP `initialize` response **must** contain sandbox routing directives instructing the model to use `sandboxed_shell` for all command execution.
- **R1.5.5**: The `activate_tools` description **must** dynamically list all loaded bundles with action-oriented hints (`ai_hint`) so the AI knows exactly when to activate each bundle.
- **R1.5.6**: CLI-enabled bundles (e.g., `--rust`, `--git`) **should** be immediately visible via auto-reveal at startup, eliminating the need for a separate reveal step.

### R2: Async-First Architecture

- **R2.1**: Operations **must** execute asynchronously by default, returning an `id` immediately.
- **R2.2**: On completion, the system **must** push results via MCP progress notifications.
- **R2.3**: Commands that modify config files (e.g., `cargo add`) **should** use `"synchronous": true` to prevent race conditions.
- **R2.4**: **Inheritance**: Subcommand-level `synchronous` overrides tool-level; tool-level overrides default (async).

### R3: Performance

- **R3.1**: The system **must** use a pre-warmed shell pool for 5-20ms command startup latency.
- **R3.2**: Shell processes are pooled per working directory and automatically cleaned up.

### R4: JSON Schema Validation

- **R4.1**: All tool configurations **must** be validated against the MTDF schema at server startup.
- **R4.2**: Invalid configurations **must** be rejected with clear error messages.
- **R4.3**: Schema supports: `string`, `boolean`, `integer`, `array`, required fields, and `"format": "path"` for security.

---

## 4. Security - Kernel-Enforced Sandboxing

The sandbox scope defines the root directory boundary. AI has **full read/write access** within the sandbox but **zero read/write access** outside it. Read access outside the sandbox is strictly limited to necessary system binaries across all platforms (Linux, macOS, Windows) and explicitly granted feature scopes (see `--livelog`).

### R5: Sandbox Scope

- **R5.1**: Sandbox scope is set once at initialization and **cannot** be changed during the session.
- **R5.2**: **STDIO mode**: Defaults to current working directory (IDE sets `cwd` to `${workspaceFolder}` in `mcp.json`).
- **R5.3**: **HTTP mode**: Set once at server start via (in order of precedence):
  1. `--sandbox-scope <path>` CLI parameter
  2. `AHMA_SANDBOX_SCOPE` environment variable
  3. Current working directory
- **R5.4**: **Write Protection**: The system **must** block any attempt to write to files outside the sandbox scope, including via command arguments (e.g., `touch /outside/file`).
- **R5.5**: **Explicit Scope Override**: If `--sandbox-scope` is provided via CLI, the system **must** respect it and **must not** attempt to expand or modify it via the MCP `roots/list` protocol (roots requests are skipped). This prevents potential security bypasses where a compromised client could widen the scope, and ensures stability for clients that do not support the roots protocol.
- **R5.6**: **Lifecycle Notifications**: The system **must** emit JSON-RPC notifications for sandbox lifecycle events:
  - `notifications/sandbox/configured`: When sandbox is successfully initialized from roots.
  - `notifications/sandbox/failed`: When sandbox initialization fails (payload: `{"error": "message"}`).
  - `notifications/sandbox/terminated`: When the session ends (payload: `{"reason": "reason"}`).
- **R5.6.1**: **Best-Effort Delivery over Pipes**: In HTTP bridge mode, lifecycle notifications are written as raw JSON-RPC to the subprocess's stdout so the bridge can intercept them. Delivery is **best-effort**: a broken-pipe error (Unix `EPIPE`, Windows OS error 232 "The pipe is being closed") during the write **must not** panic the process. This condition is expected when the bridge closes the pipe during session teardown. All stdout notification writes **must** use `utils::stdio::emit_stdout_notification`, which classifies errors as follows:
  - **Broken pipe**: logged at `debug` level, treated as non-fatal (the bridge is already shutting down).
  - **Other I/O errors**: logged at `warn` level and returned to the caller, which may choose to abort or continue.
  - Code **must not** use `println!` or `print!` for protocol data on stdout; these macros panic unconditionally on write errors.
- **R5.7**: **Path Canonicalization**: All paths **must** be canonicalized using `dunce::canonicalize` before validation to prevent symlink escape attacks. This resolves symlinks to their real targets and normalizes paths, ensuring that a symlink pointing outside the sandbox cannot be used to bypass security. The `dunce` crate is used instead of `std::fs::canonicalize` to avoid the Windows `\\?\` extended-length path prefix that can cause compatibility issues with some APIs.

### R6: Platform-Specific Enforcement

#### R6.1: Linux (Landlock)

- **R6.1.1**: Uses Landlock (kernel 5.13+) for kernel-level FS sandboxing.
- **R6.1.2**: If Landlock is unavailable and sandbox is not explicitly disabled, server **must** refuse to start with upgrade instructions.
- **R6.1.3**: If user explicitly opts into compatibility mode (`--no-sandbox` or `AHMA_NO_SANDBOX=1`), server **must** start in unsandboxed mode and emit a clear warning that Ahma sandboxing is disabled until the kernel is upgraded.

#### R6.2: macOS (Seatbelt)

- **R6.2.1**: Uses `sandbox-exec` with Seatbelt profiles (SBPL).
- **R6.2.2**: Profile uses `(deny default)` with allowed reads and writes **strictly limited** to the sandbox scope, necessary system paths, and necessary temp paths.
- **R6.2.3**: **Read Limitation**: The security guarantee is **read and write isolation**. By default, it operates identical to Landlock: standard system binaries (`/usr`, `/etc`, `~/.cargo`) are whitelisted for read/execute, and all other paths outside the scope are denied.
- **R6.2.4**: **CRITICAL**: `/var` is symlink to `/private/var` on macOS; profiles **must** use real paths.

#### R6.3: Windows (AppContainer / Job Objects) — _in-progress_

> **Security gate**: Windows GA release requires this section to reach `tests-pass` status.
> Until it does, strict mode **must** fail closed (`SandboxError::PrerequisiteFailed`) so the
> server never runs unsandboxed without explicit `--no-sandbox` opt-out.
>
> **Current status**: Job Object enforcement and AppContainer profile + DACL grant are
> **implemented** in `sandbox/windows.rs`.  Final validation (R6.3.1, R6.3.3, R6.3.7)
> requires a `windows-latest` CI run.

##### Architecture decision

The planned implementation uses two mechanisms in order of preference:

1. **AppContainer** (Windows 8+) — lowest-privilege user-space sandbox.
   An `AppContainer` SID will be given read+execute on the Windows system directory and
   full access only to the workspace root. This is the primary containment mechanism.
2. **Job Objects with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`** — applied unconditionally at
   server startup via `enforce_windows_sandbox`.  Ensures all child processes are killed
   when the server exits.  Does **not** restrict file-system access by path; AppContainer
   is required for R6.3.3.

##### Acceptance criteria (required before GA)

- **R6.3.1**: `check_windows_sandbox_available()` returns `Ok(())` when the AppContainer
  backend is confirmed ready on Windows 8+. _Status: implemented (probes `CreateAppContainerProfile`
  with an invalid name; returns `Ok(())` on Win8+, `PrerequisiteFailed` on older OS)._
- **R6.3.2**: `enforce_windows_sandbox(roots)` applies Job Object containment at server
  startup, ensuring child processes are killed on server exit. Signature mirrors
  `enforce_landlock_sandbox` (`&[PathBuf]`). _Status: **done** — Job Object with
  `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` applied; non-fatal if already inside a job._
- **R6.3.3**: Write attempts outside the sandbox scope **must** be blocked at the OS level.
  Proof: a test must show that `tools/call` inside the scope succeeds while a write to a
  path outside the scope fails with a permission error.
- **R6.3.4**: `tools/call` issued before sandbox lock (state != `Locked`) **must** return
  HTTP 409 / JSON-RPC `-32001` on Windows, identical to Linux/macOS behavior.
- **R6.3.5**: Filesystem root scopes (`C:\`, `D:\`, UNC `\\server\share`) **must** be
  rejected by `canonicalize_scopes` with `SandboxError::PrerequisiteFailed`, identical to
  Unix `/` rejection.
- **R6.3.6**: PowerShell (built into Windows 10/11) **must** be documented as a runtime requirement;
  the server should emit a clear startup error if `powershell` is absent.
- **R6.3.7**: All existing integration tests that exercise sandbox gating logic **must**
  pass on Windows CI with no `#[ignore]` waivers.
- **R6.3.8**: **Cross-Platform Test Scripts**: When tests dynamically generate and execute scripts (e.g., to verify log monitoring or stdout capture), they **must** provide equivalent logic for both `bash` (Unix) and `PowerShell` (Windows). Tests **must not** rely on `bash.exe` or `sh.exe` being present on Windows (avoids WSL dependencies). All such tests **must** use a uniform helper method (e.g., `write_cross_platform_script`) to ensure consistency and prevent platform-specific leaks.

##### Windows path model

- Sandbox scope paths use native Windows absolute paths (e.g., `C:\Users\name\project`).
- File URIs from MCP clients are parsed by `SessionManager::parse_file_uri_to_path` which
  handles `file:///C:/...` (drive letter) and `file://server/share/path` (UNC) forms.
- `normalize_path_lexically` never pops a `Prefix` or `RootDir` component (enforced by
  `scopes.rs`).

### R7: Nested Sandbox Detection

- **R7.1**: System **must** detect when running inside another sandbox (Cursor, VS Code, Docker).
- **R7.2**: Upon detection, system **must** exit with instructions to use `--no-sandbox` or `AHMA_NO_SANDBOX=1`.
- **R7.3**: When `--no-sandbox` is used, outer sandbox provides security; Ahma's internal sandbox is disabled.

---

## 5. File System Contracts and Features

### R8: Project Logging (`/log` directory)

- **R8.1**: All ahma-mcp and execution logs **must** be placed in the `log/` directory at the root of the (primary) configured sandbox scope, rather than global user cache directories (`~/.cache`).
- **R8.2**: When the project is built or the server initialized, the `log/` directory is created if it does not exist, and old `.log` files are deleted to wipe previous logs.

### R9: Safe Live Log Monitoring (`--livelog`)

- **R9.1**: The `--livelog` feature flag enables safe read-only access to specific log files located outside the sandbox scope without compromising the sandbox contract.
- **R9.2**: **Mechanisms**: During initialization (and ONLY at initialization), the system scans the `log/` directories of all configured sandbox roots for symbolic links. The targets of these symlinks are evaluated.
- **R9.3**: **Enforcement**: The resolved physical paths of those symlinks are dynamically added to the sandbox profile (across Linux, macOS, and Windows) as **read-only scopes**.
- **R9.4**: **Abuse Prevention**: Since symlinks are only resolved and granted access at startup, hostile entities or rogue AI cannot abuse this later by creating new symlinks to sensitive files (e.g. `/etc/passwd`). Existing files placed in read-only scopes are tightly controlled by the system operator running `ahma-mcp --livelog`.

---

## 5. Tool Definition (MTDF Schema)

### 5.1 Basic Structure

```json
{
  "name": "cargo",
  "description": "Rust's build tool and package manager",
  "command": "cargo",
  "enabled": true,
  "timeout_seconds": 600,
  "synchronous": false,
  "subcommand": [
    {
      "name": "build",
      "description": "Compile the current package.",
      "options": [
        { "name": "release", "type": "boolean", "description": "Build in release mode" }
      ]
    },
    {
      "name": "add",
      "description": "Add dependencies to Cargo.toml",
      "synchronous": true
    }
  ]
}
```

### 5.2 Key Fields

| Field | Description |
|-------|-------------|
| `command` | Base executable (e.g., `git`, `cargo`) |
| `subcommand` | Array of subcommands; final tool name is `{command}_{name}` |
| `synchronous` | `true` for blocking, `false`/omit for async (default) |
| `options` | Command-line flags (e.g., `--release`) |
| `positional_args` | Positional arguments |
| `format: "path"` | **CRITICAL**: Any path argument **must** include this for security validation |

### 5.3 Sequence Tools

Sequence tools chain multiple commands into a single workflow:

```json
{
  "name": "rust_quality_check",
  "description": "Format, lint, test, build",
  "command": "sequence",
  "synchronous": true,
  "step_delay_ms": 100,
  "sequence": [
    { "tool": "cargo_fmt", "subcommand": "default", "args": {} },
    { "tool": "cargo_clippy", "subcommand": "clippy", "args": {} },
    { "tool": "cargo_nextest", "subcommand": "nextest_run", "args": {} },
    { "tool": "cargo", "subcommand": "build", "args": {} }
  ]
}
```

### 5.4 Tool Availability Checks

```json
{
  "availability_check": { "command": "which cargo-nextest" },
  "install_instructions": "Install with: cargo install cargo-nextest"
}
```

---

## 6. Usage Modes

### 6.1 STDIO Mode (Default)

Direct MCP server over stdio for IDE integration:

```bash
ahma-mcp --mode stdio --tools-dir .ahma/tools
```

Alternatively, standard tool configurations are bundled directly inside the binary. Enable them using CLI flags to activate built-in fallback definitions:
```bash
ahma-mcp --mode stdio --rust --python --git --github --fileutils --simplify --gradle
```

Note: Core tools (`sandboxed_shell`, `await`, `status`, `cancel`) are always available without any flags.

**Tool loading priority**: When an `.ahma/` directory exists (auto-detected or via explicit `--tools-dir`), **all** tool definitions in it are always loaded regardless of bundle flags. Bundle flags (`--rust`, `--simplify`, etc.) additionally activate built-in tool definitions compiled into the binary, serving as **fallbacks** for tools not defined locally. Local `.ahma/` definitions override bundled defaults with the same name. If *no* `.ahma/` directory exists and no `--tools-dir` is given, only bundle-flag tools plus core built-ins are available.

### 6.2 HTTP Bridge Mode

HTTP server proxying to stdio MCP server:

```bash
# Start on default port (3000)
cd /path/to/project
ahma-mcp --mode http

# Explicit sandbox scope
ahma-mcp --mode http --sandbox-scope /path/to/project

# Custom port
ahma-mcp --mode http --http-port 8080
```

**Endpoints:**

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | `/mcp` | JSON-RPC requests |
| GET | `/mcp` | SSE stream for notifications |
| GET | `/health` | Health check |
| DELETE | `/mcp` | Terminate session (with `Mcp-Session-Id`) |

### 6.3 CLI Mode

Execute a single tool command:

```bash
ahma-mcp --tool_name cargo --tool_args '{"subcommand": "build"}'
```

### 6.4 List Tools Mode

```bash
ahma-mcp --list-tools -- /path/to/ahma-mcp --tools-dir ./tools
ahma-mcp --list-tools --http http://localhost:3000
```

---

## 7. HTTP Bridge & Session Isolation

### R8: HTTP Bridge & Streamable HTTP

- **R8.1**: HTTP bridge mode via `ahma-mcp --mode http`.
- **R8.2**: SSE at `/mcp` (GET) for server-to-client notifications.
- **R8.3**: JSON-RPC via POST at `/mcp`.
- **R8.4**: Auto-restart stdio subprocess if it crashes.
- **R8.5**: Content negotiation via `Accept` header (`text/event-stream` → SSE, `application/json` → JSON).
- **R8.6**: **HTTP Streaming (MCP Streamable HTTP)**: POST requests support SSE response streaming for full multiplexing and reconnection resilience.
  - **R8.6.1**: POST with `Accept: text/event-stream` returns SSE-formatted response and interleaved server notifications within a single stream.
  - **R8.6.2**: Per-session SSE event IDs (`id:` field) enable ordering and deduplication. Each JSON-RPC response and notification receives a monotonically-increasing session-unique ID.
  - **R8.6.3**: Event history buffer maintains recent events (bounded to 1000 events per session) for `Last-Event-Id` replay support.
  - **R8.6.4**: GET requests with `Last-Event-Id: N` replay all events with ID > N from the per-session history buffer, enabling seamless reconnection after temporary network loss.
  - **R8.6.5**: Event IDs are independent per session and start at 1. Event history is cleared when the session ends.
- **R8.7**: **HTTP/3 (QUIC) Client Preference**: All HTTP clients built with `reqwest` use the `http3` feature to prefer HTTP/3 (QUIC) transport when the server advertises support via Alt-Svc headers.
  - **R8.7.1**: HTTP/3 uses QUIC (UDP-based) for reduced connection latency and improved multiplexing compared to HTTP/2 over TCP.
  - **R8.7.2**: Transparent fallback to HTTP/2 or HTTP/1.1 when the server does not support HTTP/3.
  - **R8.7.3**: Both SSE and HTTP streaming endpoints work correctly with HTTP/3-capable clients.

### R9: Session Isolation

- **R9.1**: `--session-isolation` flag enables per-session subprocess with own sandbox scope.
- **R9.2**: Session ID (UUID) generated on `initialize`, returned via `Mcp-Session-Id` header.
- **R9.3**: Sandbox scope determined from first `roots/list` response.
- **R9.4**: Once set, sandbox scope **cannot** be changed (security invariant).
- **R9.5**: `roots/list_changed` after sandbox lock → session terminated, HTTP 403.

---

## 8. Development Workflow

### 8.1 Core Principle: Use Ahma

**Always use Ahma** instead of terminal commands:

| Instead of... | Use Ahma tool... |
|---------------|---------------------|
| `run_in_terminal("cargo build")` | `cargo` with `{"subcommand": "build"}` |
| `run_in_terminal("any command")` | `sandboxed_shell` with `{"command": "any command"}` |

**Why**: We dogfood our own product. Using Ahma catches bugs immediately, runs faster (no GUI prompts), and enforces sandbox security.

### 8.2 Quality Checks

Before committing, run (via Ahma):

1. `cargo fmt` — format code
2. `cargo nextest run` — run tests
3. `cargo clippy --fix --allow-dirty` — fix lint warnings
4. `cargo doc --no-deps` — verify docs build

### 8.3 Terminal Fallback (Rare)

Only use terminal directly when:

1. **Coverage**: `cargo llvm-cov` — instrumentation incompatible with sandboxing
2. **Ahma completely broken** — fix immediately after recovery

---

## 9. Implementation Constraints

### 9.1 Meta-Parameters

These control execution environment but **must not** be passed as CLI arguments:

- `working_directory`: Where command executes
- `execution_mode`: Sync vs async
- `timeout_seconds`: Operation timeout

### 9.2 Async I/O Hygiene

- **R10.1**: Blocking I/O (`std::fs`) **must not** be used in async functions. Use `tokio::fs` instead.
- **R10.2**: Test code is exempt (blocking acceptable in `#[tokio::test]`).
- **R10.3**: **Child Process Leaks**: All `tokio::process::Command` spawns **must** implement `.kill_on_drop(true)`. By default, dropping a tokio child process future (e.g. from a timeout) orphans the process, leaving it running in the background. This has historically caused catastrophic CLI test hangs in CI. Always explicitly enforce `kill_on_drop`.

### 9.3 Error Handling

- **R11.1**: Use `anyhow::Result` for internal error propagation.
- **R11.2**: Convert to `McpError` at MCP service boundary.
- **R11.3**: Include actionable context in error messages.

### 9.4 Unified Shell Output

- **R12.1**: All shell commands **must** redirect stderr to stdout (`2>&1`).
- **R12.2**: AI clients receive single, chronologically ordered stream.

### 9.5 Cancellation Handling

- **R13.1**: Distinguish MCP protocol cancellations from process cancellations.
- **R13.2**: Only cancel actual background operations, not synchronous MCP tool calls (`await`, `status`, `cancel`).

### 9.6 Concurrency Architecture Principles

#### R18: No-Wait State Transitions

- **R18.1**: State transitions **must never require wait loops or polling**. If code needs to "wait for" another component, the design is fundamentally broken.
- **R18.2**: Use state machines with explicit transitions. When a state change occurs, notify listeners immediately through channels or callbacks.
- **R18.3**: Example anti-pattern:

```rust
// WRONG: Polling for state change
while !session.is_sandbox_ready() {
    sleep(Duration::from_millis(100)).await;
}
```

- **R18.4**: Correct pattern:

```rust
// CORRECT: Explicit state transition notification
session.wait_for_state(SandboxState::Ready).await;
// where wait_for_state uses a channel that the setter notifies
```

#### R19: RAII for Spawned Tasks

- **R19.1**: When spawning async tasks, the caller **must not** return until the spawn is confirmed live.
- **R19.2**: Use barriers or oneshot channels to confirm task startup:

```rust
let (started_tx, started_rx) = oneshot::channel();
tokio::spawn(async move {
    started_tx.send(()).ok();  // Confirm we're running
    // ... do work ...
});
started_rx.await.ok();  // Don't return until spawn is live
```

- **R19.3**: For tasks that manage lifecycle resources (like sandbox configuration), prefer synchronous execution over spawn unless there's a specific reason for concurrent execution.

#### R20: Single Source of Truth for State

- **R20.1**: Every piece of state **must** have exactly one authoritative location.
- **R20.2**: When state needs to be observed from multiple components, use:
  - Watch channels (`tokio::sync::watch`)
  - Event listeners with guaranteed delivery
  - NOT: multiple copies of state with synchronization attempts

#### R21: Security Against Environment Pollution

- **R21.1**: Production behavior **must not** be controllable via environment variables that an attacker or malicious process could set.
- **R21.2**: Test-only behavior **should** be controlled via:
  - Compile-time features (`#[cfg(test)]`)
  - Explicit CLI parameters (e.g., `--no-sandbox`)
  - Constructor parameters passed at initialization
- **R21.3**: The following patterns are **FORBIDDEN**:
  - Any different behavior based on automatic "test mode" detection from environment variables like `NEXTEST`, `CARGO_TARGET_DIR`, etc.
  - Any environment variable that bypasses security checks

#### R22: Visual Minimalism

- **R22.1**: Public communications, including user messages, error logs, and documentation, **must** minimize the use of icons and emojis.
- **R22.2**: Standard ASCII text **should** be used for all status indications and visual cues.
- **R22.3**: Emojis are **forbidden** in source code logs and terminal output unless explicitly required for a specific standardized protocol.

---

## 10. Testing Philosophy

### 10.1 Core Principles

- **R14.1**: All new functionality **must** have tests.
- **R14.2**: Tests should be: Fast (<100ms), Isolated, Deterministic, Documented.
- **R14.3**: Bug fixes **must** include a regression test.

### 10.2 Test File Isolation (CRITICAL)

- **ALL tests MUST use temporary directories** via `tempfile` crate.
- **NEVER** create test files directly in repository structure.
- `TempDir` automatically cleans up on drop.

```rust
use tempfile::tempdir;

let temp_dir = tempdir().unwrap();
let test_file = temp_dir.path().join("test.txt");
fs::write(&test_file, "test content").unwrap();
```

### 10.3 CLI Binary Integration Tests

- All binaries (`ahma-mcp`, `generate-tool-schema`) **must** have integration tests.
- Tests in `ahma-mcp/tests/cli_binary_integration_test.rs`.
- Cover: `--help`, `--version`, basic functionality.

### 10.4 Test Utilities - Prevent Code Duplication

**R-TEST-PATH**: All binary path resolution in tests **MUST** use centralized helpers:

- **R-TEST-PATH.1**: Use `ahma_mcp::test_utils::cli::get_binary_path(package, binary)` to get binary paths
- **R-TEST-PATH.2**: Use `ahma_mcp::test_utils::cli::build_binary_cached(package, binary)` for builds with caching
- **R-TEST-PATH.3**: **NEVER** manually access `std::env::var("CARGO_TARGET_DIR")` outside of `test_utils::cli`

**Why**: CI environments may set `CARGO_TARGET_DIR` to relative paths (e.g., `target`). The centralized helpers correctly resolve these relative to the workspace root. Manual path resolution duplicates this logic and inevitably introduces bugs.

**Enforcement**: See `scripts/lint_test_paths.sh` for automated detection of violations.

### 10.5 CI-Resilient Testing Patterns

**R15**: Tests must pass reliably in CI environments with concurrent test execution.

#### R15.1: Avoid Race Conditions in Async Tests

- **R15.1.1**: Never use `tokio::select!` to race response completion against notification reception. When the response branch wins, the transport may already be closing.
- **R15.1.2**: For stdio MCP tests that verify notifications, prefer **synchronous tool execution** (`synchronous: true`). Notifications are sent **during** execution, before the response.
- **R15.1.3**: Use generous timeouts (10+ seconds) for notification waiting. CI environments are slower and more variable than local development.

#### R15.2: Test Timeout and Polling Guidelines

- **R15.2.1**: Never use fixed `sleep()` to wait for async conditions. Use `wait_for_condition()` from `test_utils`.
- **R15.2.2**: For health checks and server readiness, poll with increasing backoff instead of fixed delays.
- **R15.2.3**: When testing notifications or async events, use channel-based communication with explicit timeouts.

#### R15.3: Stdio Transport Gotchas

- **R15.3.1**: In stdio transport, notifications and responses share the same stream. The client's reader task may exit before processing all in-flight notifications.
- **R15.3.2**: The `handle_notification` callback is only invoked when rmcp's internal reader successfully parses and delivers the notification. Transport teardown can prevent this.
- **R15.3.3**: For notification tests, consider using HTTP mode with SSE instead of stdio - SSE keeps the notification stream open independently.
- **R15.3.4**: For async operations, the server immediately returns "operation started" and sends notifications in parallel. This creates an unwinnable race condition for notification testing.

#### R15.4: Coverage Overhead Mitigation

- **R15.4.1**: `llvm-cov` instrumentation significantly slows down execution (10x-20x), especially for process-heavy tests like stdio integration.
- **R15.4.2**: Integration tests involving child processes or networks **must** use generous timeouts (30s+). A 10s timeout that works in `release` mode will reliably fail in `coverage` mode.
- **R15.4.3**: Flaky failures that occur ONLY in coverage CI jobs almost always indicate timeouts being too tight for the instrumented binary overhead.

#### R15.5: Dual-Transport Test Coverage (HTTP Bridge)

The HTTP bridge exposes a single `/mcp` POST endpoint whose response format is content-negotiated via the `Accept` header:

| `Accept` value         | Handler                                  | Response                                    |
|------------------------|------------------------------------------|---------------------------------------------|
| `application/json`     | `handle_session_isolated_request`        | Single JSON-RPC response body               |
| `text/event-stream`    | `handle_session_isolated_request_sse`    | SSE stream: notifications + response event  |

**Requirement**: Every test that exercises tool execution (i.e. calls `tools/call` or `tools/list`) MUST cover BOTH response modes.

**Implementation pattern** — extract the test body into a shared `async fn run_<case>(mode: TransportMode)`, then add two `#[tokio::test]` entry points:

```rust
async fn run_my_tool_test(mode: TransportMode) {
    let Some((_server, mcp)) = setup_test_mcp(mode).await else { return; };
    // ... assertions ...
}

#[tokio::test]
async fn test_my_tool_json() { run_my_tool_test(TransportMode::Json).await; }

#[tokio::test]
async fn test_my_tool_sse()  { run_my_tool_test(TransportMode::Sse).await; }
```

**Naming convention** — append `_json` / `_sse` suffix to every test entry point that covers a specific transport mode.  Do NOT use these suffixes for tests that are transport-agnostic (e.g. pure protocol handshake tests, session lifecycle tests, or SSE-specific protocol tests such as event-ID replay).

**Infrastructure** — use `common::setup_test_mcp(mode)` (defined in `tests/common/mod.rs`).  This spawns a fresh server, completes the full MCP handshake including roots exchange, and returns an `McpTestClient` configured with the requested `TransportMode`. The client's `send_request()` / `call_tool()` / `list_tools()` methods automatically use the correct `Accept` header.

**Exemptions** — the following test files are transport-specific by design and do NOT need `_json` / `_sse` variants:
- `sse_streaming_test.rs` — validates POST SSE content-negotiation, event IDs, Last-Event-Id replay
- `sse_endpoint_test.rs`  — validates GET `/mcp` SSE notification stream and event structure
- `handshake_*.rs`       — validates session handshake protocol invariants
- `sandbox_*.rs`         — validates sandbox gating rules

**Concurrency limits** — all test files using `setup_test_mcp` spawn one server per test function.  They MUST be listed in the `threads-required = 2` override filter in the `[profile.ci.overrides]` section of `.config/nextest.toml` to prevent resource storms on GitHub Actions' 2-CPU runners (where `test-threads = "num-cpus"` = 2 and `threads-required = 2` together allow only one such test to run at a time).  The `[profile.default]` section intentionally omits `threads-required` so that local developer machines (e.g. an M4 Ultra with many cores) run tests with full parallelism.  The CI profile is activated explicitly via `cargo nextest run --profile ci` in `build.yml`; plain `cargo nextest run` always uses the default profile.

### 10.6 Testing Patterns and Helpers

> [!IMPORTANT]
> **ALL** integration tests MUST use the centralized helpers in `ahma-mcp/src/test_utils.rs`. Do NOT reinvent spawn logic, HTTP clients, or project scaffolding.

#### R16.1: Project Scaffolding (`test_utils::test_project`)
Use `create_rust_test_project` for all tests that need a filesystem. This ensures isolated unique directories via `tempfile` and no repository pollution.

#### R16.2: MCP Service Helpers
- **Stdio**: Use `setup_mcp_service_with_client()` for standard stdio handshake tests.
- **HTTP**: Use `spawn_http_bridge()` and `HttpMcpTestClient` for HTTP/SSE integration testing.

#### R16.3: Binary Resolution
Always use `cli::build_binary_cached()` to avoid redundant `cargo build` calls and ensure tests are fast and CI-friendly.

#### R16.4: Concurrent Test Helpers (`test_utils::concurrent_test_helpers`)

**Purpose**: Safe patterns for testing concurrent operations.

```rust
use ahma_mcp::test_utils::concurrent_test_helpers::*;

// Spawn tasks that start simultaneously
let results = spawn_tasks_with_barrier(5, |task_id| async move {
    // All tasks start at the exact same instant
    perform_operation(task_id).await
}).await;

// Verify no duplicates
assert_all_unique(&results);

// Bounded concurrency for resource-limited CI
let results = spawn_bounded_concurrent(items, 4, |item| async move {
    process(item).await
}).await;
```

**Why**: AI-generated concurrent tests often have subtle race conditions. Barriers ensure deterministic starts; bounded spawning prevents OOM.

#### R16.4: Timeout and Polling (`test_utils::concurrent_test_helpers`)

**Purpose**: CI-resilient waiting patterns.

```rust
use ahma_mcp::test_utils::concurrent_test_helpers::*;

// Wrap operations with clear timeout errors
let result = with_ci_timeout(
    "operation completion",
    CI_DEFAULT_TIMEOUT,
    async { monitor.wait_for_operation("op-1").await }
).await?;

// Wait with exponential backoff (more efficient)
wait_with_backoff("server ready", Duration::from_secs(10), || async {
    health_check().await.is_ok()
}).await?;
```

**Why**: Fixed `sleep()` is flaky on variable CI. Timeouts provide clear diagnostics when things hang.

#### R16.5: Async Assertions (`test_utils::async_assertions`)

**Purpose**: Assert timing behavior in async tests.

```rust
use ahma_mcp::test_utils::async_assertions::*;

// Assert operation completes in time
let result = assert_completes_within(
    Duration::from_secs(5),
    "quick operation",
    async { fetch_data().await }
).await;

// Assert condition becomes true
assert_eventually(
    Duration::from_secs(10),
    Duration::from_millis(100),
    "operation becomes complete",
    || async { monitor.is_complete("op-1").await }
).await;
```

**Why**: Standard assertions don't work with async conditions. These provide clear failure messages.

### 10.7 CI Anti-Patterns to Avoid

**R17**: Avoid these patterns that reliably cause CI failures but may work locally.

| Anti-Pattern | Problem | Solution |
|-------------|---------|----------|
| `tokio::time::sleep(Duration::from_secs(1))` | Flaky on slow CI runners | Use `wait_for_condition()` or `wait_with_backoff()` |
| `tokio::select!` racing response vs notification | Transport teardown wins | Use synchronous mode for notification tests |
| `std::fs::create_dir("./test_dir")` | Pollutes repo, conflicts between tests | Use `tempdir()` or `test_project::create_rust_test_project()` |
| `Command::new("cargo").arg("build")` | Slow, skips cached binaries | Use `cli::build_binary_cached()` |
| Spawning 100+ concurrent tasks | OOM on CI, thread exhaustion | Use `spawn_bounded_concurrent()` |
| Expecting notification order | Async execution order is undefined | Collect notifications, assert set membership |
| Hard-coded ports | Port conflicts with parallel tests | Use port 0 for auto-assignment |
| Shared mutable state without locks | Data races under concurrent tests | Use `Arc<Mutex<_>>` or channels |

#### R17.1: Example Anti-Pattern vs Correct Pattern

FAIL **WRONG**: Fixed sleep for operation completion
```rust
async fn test_operation_completes() {
    let op_id = start_operation().await;
    tokio::time::sleep(Duration::from_secs(2)).await;  // Flaky!
    assert!(is_complete(&op_id));
}
```

OK **CORRECT**: Condition-based waiting
```rust
async fn test_operation_completes() {
    let op_id = start_operation().await;
    wait_with_backoff("operation complete", Duration::from_secs(10), || async {
        is_complete(&op_id).await
    }).await?;
    // Now we know it's complete
}
```

FAIL **WRONG**: Creating files in repo directory
```rust
let f = File::create("test.txt"); // WRONG
```

OK **CORRECT**: Using temp directory
```rust
let t = tempdir();
let f = File::create(t.path().join("test.txt")); // OK
```

### 10.8 Platform-Aware Timeouts

**R18**: All test timeouts **must** use the `ahma_common::timeouts` module for platform-aware scaling.

#### R18.1: Problem Statement

Windows CI runners are 3-5x slower than Linux/macOS for:
- Process spawning and stdio communication
- File system operations (especially temp directories)
- Network socket operations
- PowerShell startup (vs bash)

Hardcoded timeouts that work locally on macOS/Linux will reliably fail on Windows CI, leading to "whack-a-mole" fixes across the codebase.

#### R18.2: Solution - Centralized Timeout Utility

The `ahma_common::timeouts` module provides:

```rust
use ahma_common::timeouts::{TestTimeouts, TimeoutCategory};

// Use semantic categories with platform-appropriate defaults
let timeout = TestTimeouts::get(TimeoutCategory::Handshake);  // 60s base, 4x on Windows

// Scale custom durations
let custom = TestTimeouts::scale_secs(5);  // 5s base, 20s on Windows

// Platform-appropriate polling interval
let interval = TestTimeouts::poll_interval();  // 100ms on Unix, 500ms on Windows
```

#### R18.3: Timeout Categories

| Category | Base (Unix) | Windows | Coverage Mode | Purpose |
|----------|-------------|---------|---------------|---------|
| `ProcessSpawn` | 30s | 120s | 240s | Binary loading, shell pool init |
| `Handshake` | 60s | 240s | 480s | MCP initialize + roots exchange |
| `ToolCall` | 30s | 120s | 240s | Individual tool execution |
| `SandboxReady` | 60s | 240s | 480s | Post-roots sandbox activation |
| `HttpRequest` | 30s | 120s | 240s | HTTP request/response cycle |
| `SseStream` | 120s | 480s | 960s | SSE stream operations |
| `HealthCheck` | 15s | 60s | 120s | Server health polling |
| `Cleanup` | 10s | 40s | 80s | Test cleanup operations |
| `Quick` | 5s | 20s | 40s | Sub-second operations |

#### R18.4: Migration Requirements

- **R18.4.1**: New tests **must** use `TestTimeouts` instead of hardcoded `Duration::from_secs()`.
- **R18.4.2**: Existing tests with Windows CI failures **should** be migrated to `TestTimeouts`.
- **R18.4.3**: When adding delays after async operations (e.g., post-SSE exchange), use `TestTimeouts::short_delay()`.
- **R18.4.4**: Polling loops **must** use `TestTimeouts::poll_interval()` instead of hardcoded intervals.

#### R18.5: Why Platform Multipliers

The 4x multiplier for Windows is based on empirical CI data:
- Windows GitHub Actions runners have ~4x slower process spawn times
- PowerShell startup is ~3x slower than bash
- Windows temp directories have higher latency than Linux tmpfs
- Coverage mode (`llvm-cov`) adds another 2x overhead

The multipliers stack: Windows + Coverage = 8x base timeout.

### 11.1 Canonical Reuse Patterns

These rules codify the architecture simplification strategy: isolate repetitive protocol/setup
details behind shared helpers so core execution algorithms remain easy to read.

#### R19: Production Helper Patterns

- **R19.1**: MCP handlers that return a single text response **should** use
  `mcp_service::handlers::common::text_result(...)` instead of inlining
  `CallToolResult::success(vec![Content::text(...)])`.
- **R19.2**: Common MCP error constructors without extra data **should** use
  `mcp_service::handlers::common::{mcp_internal, mcp_invalid_params}`.
- **R19.3**: JSON argument extraction in MCP handlers **should** use
  `mcp_service::handlers::common::{require_str, opt_str}` where applicable.
- **R19.4**: Tool-call readiness checks **must** use
  `sandbox::Sandbox::is_ready_for_tool_calls()` instead of duplicating
  `scopes().is_empty() && !is_test_mode()` checks.
- **R19.5**: Built-in tool input schemas (`await`, `status`, `sandboxed_shell`, `activate_tools`)
  **must** be generated with `mcp_service::schema` helper builders
  (`string_property`, `path_property`, enum helpers, `object_input_schema`).

#### R20: Test Harness Reuse Patterns

- **R20.1**: HTTP bridge tool tests **should** use `tests/common/setup_test_mcp_for_tools(...)`
  for setup + required-tool gating, rather than open-coding availability checks.
- **R20.2**: Reusable assertions in HTTP bridge tests **should** use
  `tests/common/assert_tool_success_with_output(...)` where output is required.
- **R20.3**: Tests that need tempdir + `.ahma` tools dir + MCP client **should** use
  `ahma_mcp::test_utils::client::McpClientFixture`.
- **R20.4**: Integration tests with custom bridge startup parameters **should** use
  `tests/common/server::spawn_server_guard_with_config(...)` instead of duplicating
  process startup/port/health polling code.
- **R20.5**: Timeout values in integration tests **must** use `TestTimeouts` categories or
  scaling helpers; numeric `Duration::from_secs(<literal>)` / `from_millis(<literal>)`
  should only be used in narrowly justified micro-timing helpers.

#### R21: Guardrail Enforcement

- **R21.1**: Guardrail scripts **must** reject deprecated HTTP bridge test helper usage
  (`ensure_server_available` outside `common/sse_test_helpers.rs`).
- **R21.2**: Guardrail scripts **must** reject newly added literal `Duration::from_secs(...)`
  / `Duration::from_millis(...)` patterns in timeout-sensitive handshake/bridge integration tests.
- **R21.3**: Guardrail scripts **should** verify that custom HTTP bridge integration tests
  use shared startup helpers from `tests/common/server.rs`.

### 11.2 Recurring Failure Mode Detection

This repo has a recurring failure mode: tests can pass while real-world usage is broken.

---

## 12. Feature Requirements by Module

### 12.1 ahma-mcp

| Feature | Status | Description |
|---------|--------|-------------|
| Adapter execution | PASS | Sync/async CLI tool execution |
| MCP ServerHandler | PASS | Complete MCP protocol implementation |
| Shell pool | PASS | Pre-warmed processes, per-directory pooling |
| Linux sandbox | PASS | Landlock enforcement |
| macOS sandbox | PASS | Seatbelt/sandbox-exec enforcement |
| Nested sandbox detection | PASS | Detect outer sandboxes |
| Operation monitor | PASS | Track async operations |
| Callback system | PASS | Push completion notifications |
| Config loading | PASS | MTDF JSON parsing |
| Schema validation | PASS | Validate at startup |
| Sequence tools | PASS | Multi-command workflows |
| Hot-reload | PASS | Watch tools directory |

### 12.2 ahma-http-bridge

| Feature | Status | Description |
|---------|--------|-------------|
| HTTP-to-stdio bridge | PASS | Proxy JSON-RPC to subprocess |
| SSE streaming | PASS | Server-sent events for notifications |
| Session isolation | PASS | Per-session sandbox scope |
| Auto-restart | PASS | Restart crashed subprocess |
| Health endpoint | PASS | `/health` monitoring |
| Session termination | PASS | DELETE with `Mcp-Session-Id` |

### 12.3 ahma-http-mcp-client

| Feature | Status | Description |
|---------|--------|-------------|
| HTTP transport | PASS | POST requests with Bearer auth |
| SSE receiving | PASS | Background task for server messages |
| OAuth 2.0 + PKCE | PASS | Browser-based auth flow |
| Token storage | PASS | Persist to temp directory |
| Token refresh | PLANNED | Auto-refresh expired tokens |

### 12.4 ahma-mcp --validate

| Feature | Status | Description |
|---------|--------|-------------|
| MTDF Validation | PASS | Validate tool configs against JSON schema via `ahma-mcp --validate` |
| Error reporting | PASS | Concise, actionable error messages |

---

## 13. CI Caching Strategy

To maintain high performance and avoid cache bloat, the following strategies are employed in GitHub Actions:

### 13.1 Daily Rotation
- **R13.1.1**: All caches **must** use a daily rotating key (e.g., `...-day${{ steps.day-number.outputs.day }}`) to ensure they contain only current files and do not grow indefinitely.
- **R13.1.2**: `restore-keys` **must** be used to fall back to the most recent previous cache (from earlier in the day or a previous day).

### 13.2 Distributed Caching (sccache)
- **R13.2.1**: **sccache** **must** be used as the compiler wrapper across all CI jobs.
- **R13.2.2**: The **GitHub Actions Backend** (`SCCACHE_GHA_ENABLED: "true"`) **must** be used for `sccache` to allow atomic uploads of object files directly to the GHA cache API.
- **R13.2.3**: `SCCACHE_DIRECT: "true"` **should** be enabled for Windows runners to optimize compiler invocation.
- **R13.2.4**: Each CI job **must** use unique `SCCACHE_GHA_CACHE_TO` keys to prevent concurrent write conflicts. Key format: `sccache-{OS}-{ARCH}-{JOB}-day{DAY}`.
- **R13.2.5**: Each CI job **must** use `SCCACHE_GHA_CACHE_FROM` with comma-separated fallbacks to enable cache sharing between related jobs on the same platform.
- **R13.2.6**: Debug-profile jobs on the same platform (clippy, nextest, android, coverage) **should** include each other in their `CACHE_FROM` lists since they produce compatible cache entries.
- **R13.2.7**: Release-profile jobs **must not** include debug caches in `CACHE_FROM` since `--release` flag produces incompatible cache entries.

### 13.3 Cargo Registry Caching
- **R13.3.1**: The Cargo registry (`~/.cargo/registry`) and git database (`~/.cargo/git`) **must** be cached using `actions/cache` or specialized actions, adhering to the Daily Rotation rule.

---

## 13. Build & Development

### 13.1 Prerequisites

```bash
# Rust 1.93+ required
rustup update stable

# Build
cargo build --release

# The binary will be at target/release/ahma-mcp
```

### 13.2 mcp.json Configuration

```json
{
  "servers": {
    "Ahma": {
      "type": "stdio",
      "cwd": "${workspaceFolder}",
      "command": "/path/to/ahma-mcp/target/release/ahma-mcp",
      "args": ["--tools-dir", ".ahma/tools"]
    }
  }
}
```

### 13.3 Quality Checks

> **CRITICAL for AI Assistants:** Run all checks and ensure they pass **before stopping work**.

```bash
cargo fmt                           # Format code
cargo clippy --all-targets          # Check for lints (must pass)
cargo build --release               # Verify build succeeds
cargo nextest run                   # Run all tests (must pass)
```

### 13.4 Test-First Development (TDD)

> **MANDATORY for all new features and bug fixes:**

**R13.4.1**: **ALL** functional requirements and bug fixes **MUST** follow test-first development:

1. **Write the test first** - Write a test that expresses the desired behavior or exposes the bug
2. **See it fail** - Run the test and verify it fails for the expected reason
3. **Implement the fix** - Write the minimal code to make the test pass
4. **See it pass** - Run the test and verify it passes
5. **Refactor** - Clean up the code while keeping tests green

**R13.4.2**: This workflow is **non-negotiable** and applies to:
- New features (e.g., auto-detection of `.ahma` directory)
- Bug fixes (any deviation from expected behavior)
- Performance improvements (when testable)
- Security enhancements (when testable)

**R13.4.3**: Tests are **part of the functional requirements**, not an afterthought.

**R13.4.4**: Code changes without corresponding tests **MUST NOT** be merged unless:
- The change is purely documentation
- The change is a trivial typo fix in comments
- Tests are genuinely impossible (must be justified in code review)

**R13.4.5**: Before considering any work complete, you **MUST** run these quality checks in order:
1. `cargo clippy` - Verify no warnings or errors
2. `cargo nextest run` (preferred) or `cargo test` - Verify all tests pass
3. Only after both pass can work be considered complete

This ensures:
- Code quality and idiomatic Rust patterns (clippy)
- No regressions in functionality (nextest)
- Early detection of issues before they are merged

**Failing to run these checks results in broken builds and wasted time.**

---

## 14. Maintenance Notes

> **AI Assistants:** When you modify code or discover issues:
>
> 1. Update the "Quick Status" table
> 2. Add to "Known Issues" if new bugs found
> 3. Update feature tables with status changes
> 4. **BEFORE stopping work: Run `cargo clippy` then `cargo nextest run` to verify quality`

**Last Updated**: 2026-01-18

**Status**: Living Document - Update with every architectural decision or significant change
