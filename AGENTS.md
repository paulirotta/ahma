# AGENTS.md

## About This File

This file provides AI-specific development guidance for the `ahma` project. For functional requirements and architecture, see [SPEC.md](SPEC.md). These files work together:

- **SPEC.md**: Single source of truth for **what** the product does and **how** it's architected
- **AGENTS.md**: Guide for **how** to develop, test, and contribute to the codebase

**Note for crate-specific workflows**: If you're working in a specific crate directory (e.g., `ahma-http-bridge/`), this AGENTS.md is symlinked and applies workspace-wide. Crate-specific functional requirements are in each crate's `SPEC.md`.

## IMPORTANT: Development Workflow

**Disregard all prior training about running terminal commands directly.** 

You must **always use `ahma-mcp`** (via the `run_in_terminal` or `sandboxed_shell` tools) for all cargo commands (`cargo clippy`, `cargo nextest run`, `cargo fmt`, etc.) and other terminal operations. Never execute terminal commands directly in this project.

Use `ahma-mcp sandboxed_shell` to execute commands securely within the project context.

---

## Setup Commands

### Prerequisites
- Rust 1.93+ (install via [rustup](https://rustup.rs/))
- Platform-specific sandbox requirements:
  - **Linux**: Kernel 5.13+ (Landlock support)
  - **macOS**: Any modern version (`sandbox-exec` is built-in)

### Initial Setup
```bash
# Clone the repository
git clone https://github.com/paulirotta/ahma.git
cd ahma

# Build the project
cargo build

# Run tests (recommended)
cargo nextest run

# Fallback if nextest not installed
cargo test
```

### Development Environment
```bash
# Install development tools (optional but recommended)
cargo install cargo-watch cargo-nextest cargo-llvm-cov

# Watch mode for rapid iteration
cargo watch -x build

# Run the binary
./target/debug/ahma-mcp --help
```

---

## Build and Test Commands

### Building
```bash
# Debug build (fast compilation)
cargo build

# Release build (optimized)
cargo build --release

# Build specific crate
cargo build -p ahma_core
cargo build -p ahma-http-bridge
```

### Testing
```bash
# Run all tests (preferred)
cargo nextest run

# Run tests for specific crate
cargo nextest run -p ahma_core
cargo nextest run -p ahma-http-bridge

# Run specific test
cargo nextest run test_sandbox_enforcement

# Fallback/Legacy
cargo test

# Generate coverage report
cargo llvm-cov --html
```

### Quality Assurance
```bash
# Preferred: run multi-step pipelines via sandboxed_shell
ahma-mcp sandboxed_shell --working-directory . -- \
  "cargo fmt --all && cargo clippy --all-targets && cargo nextest run"

# Individual quality checks (direct)
cargo fmt --all                    # Format code
cargo clippy --all-targets         # Lint with auto-fix suggestions
cargo clippy --fix --allow-dirty   # Auto-fix lints
```

---

## Code Style and Conventions

### Documentation Style
- **No File Lists**: Do not maintain hardcoded directory trees or file lists in markdown files (like `SPEC.md` or `CLAUDE.md`). AI agents can explore the workspace directly. Hardcoded lists waste context window and become outdated quickly.
- **Single Source of Truth**: Keep architectural decisions in `SPEC.md` and AI instructions in `AGENTS.md`.

### Rust Style
- **Formatting**: Use `rustfmt` (enforced by `cargo fmt`)
- **Linting**: Pass `clippy` with no warnings
- **Naming**: Follow Rust naming conventions (`snake_case` for functions/variables, `CamelCase` for types)
- **Documentation**: Public APIs must have doc comments (`///` for items, `//!` for modules)

### Project-Specific Patterns

#### Temporary Python Scripts
- **Never add or commit Python scripts (`*.py`) to this repository.**
- Temporary Python scripts may be created and run for one-off local tasks (debugging, data inspection, quick transformations).
- After use, delete any temporary Python script before finishing work, and ensure no `.py` files are staged or committed.

#### Error Handling
- Use `anyhow::Result` for internal error propagation
- Convert to `rmcp::error::McpError` at the MCP service boundary
- Include actionable context: `with_context(|| "Failed to X because Y")`
- Example: "Install with `cargo install cargo-nextest`" in error messages

#### Async/Await
- **CRITICAL**: Never use `std::fs` or blocking I/O in async functions
- Use `tokio::fs` and `tokio::io` for all file operations
- Reserve `tokio::task::spawn_blocking` for:
  - Third-party libs with only sync APIs
  - CPU-bound computation (not I/O)
- Test code is exempt from this rule

#### Logging
- `error!`: Operation failures affecting user workflows
- `warn!`: Recoverable issues or deprecated usage
- `info!`: Normal operation milestones (startup, shutdown, major state changes)
- `debug!`: Detailed troubleshooting information

#### Stdout Notification Writes (SPEC R5.6.1)
- **Never use `println!` or `print!` to write protocol data on stdout.** These macros panic on any write error (e.g., broken pipe on Windows = OS error 232).
- In **stdio server mode** (subprocess spawned by HTTP bridge), stdout is a pipe. The bridge may close it during shutdown, causing broken-pipe errors.
- **Always use `crate::utils::stdio::emit_stdout_notification`** for all JSON-RPC notifications written to stdout. It classifies errors:
  - Broken pipe → `debug` log, treated as non-fatal
  - Other I/O errors → `warn` log, returned to caller
- **`println!` is acceptable in CLI mode only** (`--list-tools`, single tool execution, validation) where stdout goes to a terminal, not a pipe.

---

## Testing Instructions

### Test Organization
Tests are organized into three categories:

1. **Unit Tests**: In-module `#[cfg(test)]` blocks or crate `tests/` directory
2. **Integration Tests**: Cross-module workflows in workspace `tests/`
3. **Regression Tests**: Bug fixes must include a test that would have caught the bug

### Test Requirements
- **Coverage Target**: ≥80% for all crates except:
  - `ahma_core/src/test_utils.rs` (testing infrastructure)
  - `ahma-http-bridge/src/main.rs` (binary entry point, tested via CLI integration)
- **Fast**: Most tests complete in <100ms
- **Isolated**: Use `tempfile::TempDir` for all file operations (see below)
- **Deterministic**: Same input always produces same output
- **Documented**: Test names describe what they verify

### Cross-Platform Test Checklist

All tests run on Linux, macOS, **and Windows** CI. Follow these rules to avoid platform-specific breakage:

- **Never hardcode `/tmp`, `/var/folders`, or `/dev/null`** in tests. Use `test_utils::path_helpers`:
  - `test_temp_path("name")` — path inside `std::env::temp_dir()` (works on all platforms)
  - `test_out_of_scope_path()` — guaranteed outside any sandbox scope
  - `test_blocked_device_path()` — platform device path (`/dev/null` or `NUL`)
  - `test_abs(&["a","b"])` and `test_root()` — platform-rooted absolute paths
- **Never hardcode `/bin/sh`, `/bin/bash`, or bash-specific syntax** (e.g. `>&2`, `2>&1`) in shell command strings sent through the tool pipeline. On Windows the shell is `powershell` (PowerShell 5.1+), which uses different redirection syntax.
- **Prefer `std::env::temp_dir()`** over `/tmp` for any temp-related logic.
- **Use `Path`/`PathBuf` APIs** instead of string manipulation for path separators — never assume `/` or `\\`.
- **Avoid `#[cfg(unix)]` gating when cross-platform alternatives exist.** If a test is genuinely Unix-only (e.g. uses `std::os::unix::fs::symlink`), gating is correct. But never gate a test that could be rewritten with cross-platform APIs.
- **Be aware that `Path::starts_with` is case-sensitive on Windows** even though the filesystem is case-insensitive. Use `dunce::canonicalize` on both sides of comparisons.

### Platform-Aware Timeouts (REQUIRED)

**Never hardcode timeouts.** Windows CI runners are 3-5x slower than Linux/macOS. Use `ahma_common::timeouts`:

```rust
use ahma_common::timeouts::{TestTimeouts, TimeoutCategory};

// Use semantic categories with platform-appropriate defaults
let timeout = TestTimeouts::get(TimeoutCategory::Handshake);  // 60s base, 240s on Windows

// Scale custom durations by platform multiplier
let custom = TestTimeouts::scale_secs(5);  // 5s base, 20s on Windows

// Platform-appropriate polling interval
let interval = TestTimeouts::poll_interval();  // 100ms Unix, 500ms Windows

// Short delay after async operations (e.g., post-SSE exchange)
sleep(TestTimeouts::short_delay()).await;
```

Available categories: `ProcessSpawn`, `Handshake`, `ToolCall`, `SandboxReady`, `HttpRequest`, `SseStream`, `HealthCheck`, `Cleanup`, `Quick`. See SPEC.md §10.8 for details.

### Hard Invariants (Do Not “Test Around” These)

#### MCP Streamable HTTP Handshake (HTTP Bridge)
Integration tests MUST mimic real client behavior closely:

1. `initialize` (POST, no session header) → server returns `mcp-session-id`
2. Open SSE stream (GET `/mcp`, `Accept: text/event-stream`, with `mcp-session-id`) **before** sending `notifications/initialized`
3. Send `notifications/initialized` (POST with `mcp-session-id`)
4. Wait for server `roots/list` request over SSE, respond via POST with the same `id`
5. Only after sandbox is locked, call `tools/call`

If a test client cannot follow this sequence, fix the client/test harness (or the server) rather than weakening assertions.

#### Sandbox Gating Must Be Observable
`tools/call` before sandbox lock MUST return HTTP 409 with JSON-RPC error code `-32001` ("Sandbox initializing..."). Tests should assert this explicitly where relevant.

#### Dual-Transport Coverage (HTTP Bridge Tool Tests — SPEC.md §R15.5)
Every test that calls `tools/call` or `tools/list` via the HTTP bridge **must run against BOTH response modes**: `Accept: application/json` (JSON transport) and `Accept: text/event-stream` (SSE transport).

**Pattern** — extract the body into a shared `run_*` function, add `_json` / `_sse` entry points:

```rust
async fn run_my_tool(mode: TransportMode) {
    let Some((_server, mcp)) = setup_test_mcp(mode).await else { return; };
    let result = mcp.call_tool("tool_name", json!({})).await;
    assert!(result.success, "{:?}", result.error);
}

#[tokio::test]
async fn test_my_tool_json() { run_my_tool(TransportMode::Json).await; }
#[tokio::test]
async fn test_my_tool_sse()  { run_my_tool(TransportMode::Sse).await; }
```

**Setup** — use `common::setup_test_mcp(mode)` (in `tests/common/mod.rs`). It spawns a server, completes the full MCP handshake, and returns an `McpTestClient` wired to the requested transport. Do **not** use the old `sse_test_helpers::{ensure_server_available, call_tool}` functions — those are legacy and lack session handling.

**Nextest** — add new test files to the `threads-required = 2` override filters in `.config/nextest.toml` (both `default` and `ci` profiles) so they don't storm 2-CPU CI runners.

**Exemptions** (keep SSE-only, no `_json`/`_sse` split):
- `sse_streaming_test.rs`, `sse_endpoint_test.rs` — SSE protocol behaviour
- `handshake_*.rs` — session handshake invariants
- `sandbox_*.rs` — sandbox gating rules

#### No Print-Only Integration Tests
Integration tests MUST include assertions on:
- success/failure (`result.success` or HTTP status)
- key output/error patterns
Printing output is allowed, but never sufficient.

### Debug/Trace Evidence (Required For Repros)

#### Always Capture Text Logs
When reproducing failures (especially cancellations), always capture complete logs via:

```bash
<command> 2>&1 | tee /tmp/ahma_debug.log
```

If output is long, use `tail -200 /tmp/ahma_debug.log` to summarize.

#### Reduce Concurrency When Debugging
Prefer single-test runs for clarity:

RUST_TEST_THREADS=1 cargo nextest run <test_name> --no-capture 2>&1 | tee /tmp/ahma_test.log
```

For `nextest`, run narrow filters so only one failing test prints logs.

### File Isolation (CRITICAL)
**ALL tests MUST use temporary directories** to prevent repository pollution:

```rust
use tempfile::tempdir;

#[test]
fn test_something() {
    let temp_dir = tempdir().unwrap();  // Auto-cleanup on drop
    let test_file = temp_dir.path().join("test.txt");
    
    // Create test files within temp_dir.path()
    std::fs::write(&test_file, "test content").unwrap();
    
    // Test your code...
    
    // temp_dir is automatically cleaned up when it goes out of scope
}
```

**Never** create test files directly in the repository structure. Always use `tempfile::tempdir()` or `tempfile::TempDir::new()`.

### Running Tests

> **Local vs CI parallelism**: plain `cargo nextest run` uses the `default` nextest profile, which runs tests with full CPU parallelism (no `test-threads` cap).  This is intentional — developer machines such as an M4 Ultra have ample resources.  CI uses `cargo nextest run --profile ci` (set in `build.yml`), which caps parallelism to `num-cpus` (= 2 on GitHub Actions runners) and applies `threads-required = 2` to resource-heavy tests, allowing only one such test at a time.  **Never add `--test-threads` or a `test-threads` setting to `[profile.default]` — that would throttle local developer machines unnecessarily.**

```bash
# Run all tests (full parallelism locally — no throttle)
cargo nextest run

# Run tests with coverage
cargo llvm-cov --html
open target/llvm-cov/html/index.html

# Run specific test file
cargo nextest run --test sandbox_test

# Run test and show output
cargo nextest run test_name --no-capture
```

---

## Common Development Tasks

### Adding a New Tool
1. Create a JSON configuration in `.ahma/yourtool.json`
2. Follow the MTDF schema (see [SPEC.md Section 3](SPEC.md#3-tool-definition-mtdf-schema))
3. Test the tool: `ahma-mcp yourtool_subcommand --help`
4. Restart the server to pick up tool changes by default; use `--hot-reload-tools` only while developing tool definitions

### Debugging
```bash
# Run with debug logging
ahma-mcp --debug --log-to-stderr

# Inspect MCP protocol communication
./scripts/ahma-inspector.sh

# Test single tool in CLI mode
ahma-mcp cargo_build --working-directory . -- --release
```

### MCP Server Testing
```bash
# Start stdio server (used by Cursor/VS Code)
ahma-mcp --mode stdio

# Start HTTP bridge server
ahma-mcp --mode http --http-port 3000

# List all tools from a server
ahma-mcp --list-tools -- ./target/debug/ahma-mcp --tools-dir .ahma
ahma-mcp --list-tools --http http://localhost:3000 --format json
```

---

## PR and Commit Guidelines

## Definition of Done (Local Verification)

Before you stop work / hand off / claim “all green”, you MUST run:

1. The normal test suite: `cargo nextest run`
2. All ignored tests that apply to your platform: `cargo nextest run --workspace --run-ignored all`

Notes:
- “Ignored” tests in this repo are typically expensive stress/regression coverage. They are part of the required verification set.
- If an ignored test cannot be run due to missing prerequisites (e.g., platform-only features) or it is currently broken, you must:
  - record the reason (and how to reproduce) in your handoff/PR description
  - and fix it or open/track an issue before considering the work complete

### Before Committing
1. **Run quality pipeline**: `cargo fmt --all && cargo clippy --all-targets && cargo nextest run` must pass
2. **Format code**: `cargo fmt --all`
3. **Fix clippy warnings**: `cargo clippy --fix --allow-dirty`
4. **Run tests**: `cargo nextest run` must pass with ≥80% coverage

Optional local guardrail before pushing:
- Install pre-push hook: `cp scripts/pre-push.sh .git/hooks/pre-push && chmod +x .git/hooks/pre-push`
- The hook enforces a clean working tree (including no untracked files) and runs `cargo check --workspace --locked`.
- This catches "works locally but fails on CI checkout" cases caused by untracked required source files.

Optional but recommended for faster runs:
- `cargo nextest run` (and for ignored: `cargo nextest run --run-ignored all`)

### Commit Messages
Follow conventional commits format:
```
<type>(<scope>): <subject>

<body>

<footer>
```

Types: `feat`, `fix`, `docs`, `test`, `refactor`, `perf`, `chore`

Example:
```
feat(sandbox): add nested sandbox detection for Cursor

- Detect when running inside another sandbox (Cursor, VS Code, Docker)
- Auto-disable internal sandbox with warning
- Add AHMA_NO_SANDBOX env var for manual override

Closes #123
```

### PR Title Format
```
[<crate>] <description>
```

Examples:
- `[ahma_core] Add kernel-level sandboxing via Landlock`
- `[ahma-http-bridge] Fix session isolation scope derivation`

---

## Security Considerations

### Sandbox Scope
- The sandbox scope **cannot** be changed during a session (security invariant)
- Never trust user-provided paths without validation via `path_security` module
- All file operations are restricted to the sandbox scope by the kernel

### Nested Sandboxes
When running inside another sandbox (Cursor, VS Code, Docker):
- System auto-detects and disables internal sandbox
- Outer sandbox still provides security
- Use `--no-sandbox` to suppress detection warnings

### Temp Directory Access (`--tmp`)

The `--tmp` flag (or `AHMA_TMP_ACCESS=1` environment variable) adds the system temp directory to the sandbox scope as an explicit read/write scope. This is useful for testing and dynamic workflows that require temp file access.

**When to use:**
- Testing workflows that require temp file access
- Tools that legitimately need temp storage (compilers, build systems)
- Development environments where temp access is needed

**When NOT to use:**
- Production deployments handling sensitive data
- When `--no-temp-files` provides sufficient security

**Flag interactions:**

| Flag Combination | Behavior |
|-----------------|----------|
| (default) | Temp access via implicit platform rules |
| `--tmp` | Temp dir added as formal scope (explicit) |
| `--no-temp-files` | Temp access blocked entirely |
| `--tmp --no-temp-files` | `--no-temp-files` takes precedence (blocked) |

**Security considerations:**

1. **Shared temp directory**: `/tmp` (or equivalent) is shared by all users/processes. Malicious processes could read files written by ahma-mcp, write files that ahma-mcp might read (symlink attacks), or fill up temp space.
2. **Symlink attacks**: Mitigated by `dunce::canonicalize` which resolves symlinks before validation.
3. **Predictable paths**: Tools using predictable temp file names are vulnerable to TOCTOU attacks. Use `mktemp` with random suffixes.
4. **Cross-session data leakage**: Temp files may persist across sessions. Clean up sensitive temp files after use.

### Windows Platform Development

> **Status**: Runtime (PowerShell shell pool, path model) is `in-progress`.
> Job Object sandbox enforcement (`enforce_windows_sandbox`) is **done** and wired into startup.
> AppContainer backend (`ahma-mcp/src/sandbox/windows.rs`) is `not-started` — fails closed until implemented.

#### Key rules for Windows-targeted changes

- **Never use `#[cfg(unix)]` or `#[cfg(target_family = "unix")]` for tests that have Windows-compatible equivalents.**  
  If a test is genuinely Unix-only (e.g., because it calls `std::os::unix::fs::symlink`), using `#[cfg(unix)]` without a Windows arm is correct — do not force-write a broken Windows version just to fill the gap.
- **Root path checks** in `sandbox/scopes.rs` use `is_filesystem_root()` — never compare  
  directly to `Path::new("/")` because `C:\` and UNC roots have different representations.
- **Shell invocations** must go through `shell_binary()` / `shell_args()` (in `shell_pool.rs`) — do not hard-code `bash` or `/bin/sh`.  
  The removed `is_shell_program_invocation()` function caused a double `-c` bug; do not re-introduce it.
- **Path separators**: always use `std::path::MAIN_SEPARATOR` or `Path`/`PathBuf` APIs.  
  String-based separator assumptions (`"/"`, `"\\"`) break cross-platform.
- **`expand_home`**: tilde expansion handles both `~/` and `~\` — test both when modifying.
- **Temp files**: use `std::env::temp_dir()` (cross-platform) rather than `/tmp`.

#### Windows sandbox implementation checklist

Before marking `Windows Sandbox backend` → `tests-pass` in SPEC.md, all of the following
must be satisfied (see R6.3 in SPEC.md for the full acceptance criteria):

- [x] `check_windows_sandbox_available()` returns `Ok(())` when AppContainer backend is ready — **done**: probes `CreateAppContainerProfile` with invalid name; `E_INVALIDARG` confirms Win8+ API is present
- [x] `enforce_windows_sandbox(roots)` — **done**: Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` applied at startup; non-fatal if already inside a job
- [x] Win32 imports in `windows.rs` are active (not commented out) — `CloseHandle`, `FALSE`, `CreateJobObjectW`, `SetInformationJobObject`, `AssignProcessToJobObject`, `JOBOBJECT_EXTENDED_LIMIT_INFORMATION`, `GetCurrentProcess`
- [x] AppContainer profile + DACL grant implemented in `create_windows_sandboxed_command` — **done**: `CreateAppContainerProfile`, `SetNamedSecurityInfoW` with `FILE_ALL_ACCESS` for container SID
- [ ] Write outside scope is OS-blocked at kernel level (requires `windows-latest` CI integration test — R6.3.3)
- [x] `tools/call` before sandbox lock returns HTTP 409 / JSON-RPC `-32001` — covered by `handshake_timeout_test`
- [x] Filesystem root scopes (`C:\`, UNC) are rejected by `canonicalize_scopes` — **done**: `is_filesystem_root()` handles all Win/Unix root forms
- [ ] All sandbox gating integration tests pass on `windows-latest` CI runner — blocked on CI run (R6.3.7)

#### Running against Windows CI locally (cross-check)

If you have a Windows machine or VM, you can validate cross-platform patches by running:
```powershell
cargo check --workspace
cargo clippy --workspace
cargo nextest run --workspace
cargo nextest run --workspace -- --ignored
```

On macOS/Linux you can catch obvious Windows-compile errors without a VM:
```bash
cargo check --workspace --target x86_64-pc-windows-msvc  # requires Windows cross toolchain
```

### Validation
- All tool configurations are validated against the MTDF JSON schema at startup
- Invalid configs are rejected with clear error messages
- `format: "path"` in JSON schema triggers path security validation

---

## Tool Usage Patterns

### Async-First Workflow
**Default**: Tools run asynchronously and return immediately with an `id`. The AI receives a notification when complete.

```json
{
  "name": "cargo_build",
  "description": "Build the project (async). You can continue with other tasks.",
  "synchronous": false  // or omit (default is async)
}
```

### Synchronous Override
**When to use**: Commands that modify project state (e.g., `cargo add`, `npm install --save`)

```json
{
  "name": "cargo_add",
  "description": "Add a dependency to Cargo.toml (waits for completion)",
  "synchronous": true  // Force synchronous execution
}
```

### CLI Testing Workflow
For development and debugging, bypass the MCP protocol:

```bash
# Execute a single tool command
ahma-mcp cargo_build --working-directory . -- --release

# With debug logging
ahma-mcp --debug --log-to-stderr cargo_test --working-directory .
```

---

## Additional Resources

- **Architecture**: [SPEC.md](SPEC.md)
- **HTTP Bridge Details**: [docs/session-isolation.md](docs/session-isolation.md)
- **Development Methodology**: [docs/spec-driven-development.md](docs/spec-driven-development.md)
- **Coverage Reports**: https://paulirotta.github.io/ahma/html/
- **MCP Protocol**: https://github.com/mcp-rs/rmcp

---

**Last Updated**: 2026-03-04
**For Questions**: Open an issue on GitHub or check existing documentation in `docs/`
