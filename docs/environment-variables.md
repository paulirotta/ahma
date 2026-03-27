# Ahma Environment Variables

All non-essential runtime options in `ahma-mcp` are controlled via environment variables.
The CLI itself handles subcommand selection and frequently changed options (tool bundles,
HTTP host/port). Everything else — sandbox policy, logging, execution tuning — lives here.

## Boolean flags

Set to `1`, `true`, `yes`, or `on` to enable. Any other value (or absence) means disabled.

## Path lists

On Unix, multiple paths are colon-separated (e.g. `AHMA_SANDBOX_SCOPE=/a:/b`).  
On Windows, use semicolons (`;`) as separators.

---

## Tool Management

| Variable | CLI equivalent | Default | Description |
|---|---|---|---|
| `AHMA_TOOLS_DIR` | `--tools-dir` | auto-detect `.ahma/` | Path to the directory containing JSON tool definitions. Takes precedence over the `--tools-dir` CLI flag when both are set. |
| `AHMA_TIMEOUT` | `--timeout` | `360` | Default tool execution timeout in seconds. Individual tools can override this via the `timeout_seconds` field in their JSON definition. |
| `AHMA_SYNC` | `--sync` | off | Force all tools to run synchronously. By default tools are async-first: if a result arrives within 5 seconds it is returned inline; otherwise an operation ID is returned and the result is pushed as a notification. |
| `AHMA_HOT_RELOAD` | — | off | Watch the tools directory for JSON changes and reload tool definitions at runtime. **Security warning**: enabling this allows future writes to the tools directory to add or replace tools mid-session. Enable only while authoring tool definitions. |
| `AHMA_SKIP_PROBES` | — | off | Skip tool availability probes at startup. Probes detect whether required executables (e.g. `cargo`, `git`) are installed and hide tools whose prerequisites are missing. Skip to reduce startup latency when you know all tools are available. |
| `AHMA_PROGRESSIVE_DISCLOSURE_OFF` | — | off | Disable progressive disclosure (expose all tools to the client immediately). By default, less-frequently-used tools are hidden until the client requests them, preserving the AI's context window. |

```bash
# Use a shared tools directory
AHMA_TOOLS_DIR=/shared/ahma-tools ahma-mcp serve stdio

# Extend the default timeout for slow builds (env var or CLI flag)
AHMA_TIMEOUT=600 ahma-mcp serve stdio
ahma-mcp serve stdio --timeout 600

# Force synchronous execution (env var or CLI flag)
AHMA_SYNC=1 ahma-mcp serve stdio
ahma-mcp serve stdio --sync
```

---

## Sandbox & Security

See [docs/security-sandbox.md](security-sandbox.md) for full platform details, nested sandbox
detection, and example `mcp.json` configurations.

| Variable | CLI equivalent | Default | Description |
|---|---|---|---|
| `AHMA_DISABLE_SANDBOX` | `--no-sandbox` | off | Disable the kernel sandbox entirely. **UNSAFE** — the AI can read and write anywhere on the filesystem. Use only in environments that provide their own containment (Docker, CI containers) or on hardware where the kernel sandbox is unsupported (e.g. Raspberry Pi with kernel < 5.13). |
| `AHMA_SANDBOX_SCOPE` | — | current working directory | Colon-separated list of absolute paths that define the sandbox boundary. The AI can read and write only within these directories. If not set, the sandbox scope is the directory from which `ahma-mcp` was launched. |
| `AHMA_SANDBOX_DEFER` | — | off | Defer sandbox lock until the MCP client sends a `roots/list` response. Use when the client supplies workspace roots at connection time and you want those roots to become the sandbox scope automatically. |
| `AHMA_WORKING_DIRS` | — | — | Colon-separated fallback working directories used when `AHMA_SANDBOX_DEFER=1` is set but the client does not provide roots. Has no effect when `AHMA_SANDBOX_DEFER` is off. |
| `AHMA_TMP_ACCESS` | `--tmp` | off | Add the system temp directory (`/tmp` or equivalent) to the sandbox scope. Useful for workflows that need scratch space (compilers, build systems). See [security-sandbox.md](security-sandbox.md) for security trade-offs. |
| `AHMA_DISABLE_TEMP` | — | off | Block all access to the system temp directory. Takes precedence over `AHMA_TMP_ACCESS`. |

```bash
# Sandbox scoped to two project directories
AHMA_SANDBOX_SCOPE=/projects/backend:/projects/frontend ahma-mcp serve stdio

# Defer sandbox scope to whatever the IDE declares as the workspace root
AHMA_SANDBOX_DEFER=1 ahma-mcp serve stdio

# Defer with a fallback if the client doesn't provide roots
AHMA_SANDBOX_DEFER=1 AHMA_WORKING_DIRS=/home/user/projects ahma-mcp serve stdio

# Disable sandbox in a Docker container that provides its own isolation (env var or CLI flag)
AHMA_DISABLE_SANDBOX=1 ahma-mcp serve stdio
ahma-mcp serve stdio --no-sandbox

# Allow build tools to write to the temp directory (env var or CLI flag)
AHMA_TMP_ACCESS=1 ahma-mcp serve stdio
ahma-mcp serve stdio --tmp
```

---

## Logging

| Variable | CLI equivalent | Default | Description |
|---|---|---|---|
| `RUST_LOG` | — | `info` | Standard Rust log filter. Controls verbosity for all crates. Common values: `debug`, `info`, `warn`, `error`. Crate-specific filters (e.g. `ahma_mcp=debug,rmcp=warn`) are also supported. |
| `AHMA_LOG_TARGET` | — | file (rolling) | Set to `stderr` to route all log output to stderr instead of the default rotating log file under `./log/`. Useful for Docker, CI, or any environment where stdout/stderr is captured. |
| `AHMA_LOG_MONITOR` | `--log-monitor` | off | Enable live log monitoring. Ahma tails the configured log stream through an LLM to detect issues in real time and push alerts as MCP progress notifications. See [docs/live-log-monitoring.md](live-log-monitoring.md) for setup. |
| `AHMA_MONITOR_RATE_LIMIT` | `--monitor-rate-limit` | `60` | Minimum seconds between successive log-monitor alerts. Prevents alert storms when a persistent issue triggers repeated pattern matches. |

```bash
# Debug logging to stderr (ideal for development)
RUST_LOG=debug AHMA_LOG_TARGET=stderr ahma-mcp serve stdio

# Enable live log monitoring with reduced rate limiting (env var or CLI flags)
AHMA_LOG_MONITOR=1 AHMA_MONITOR_RATE_LIMIT=30 ahma-mcp serve stdio
ahma-mcp serve stdio --log-monitor --monitor-rate-limit 30
```

---

## HTTP Transport

These variables apply only when running `ahma-mcp serve http`. Most have equivalent CLI flags
on the `serve http` subcommand; the environment variable and CLI flag can be used together
(either enables the feature).

See [docs/connection-modes.md](connection-modes.md) for full HTTP bridge setup, `mcp.json`
examples, and streaming transport details.

| Variable | CLI equivalent | Default | Description |
|---|---|---|---|
| `AHMA_DISABLE_QUIC` | `--disable-quic` | off | Disable HTTP/3 over QUIC. The bridge defaults to serving HTTP/2 (TCP) and HTTP/3 (QUIC) concurrently and advertising QUIC via the `Alt-Svc` header. Set this when UDP is blocked or QUIC causes connectivity issues. |
| `AHMA_DISABLE_HTTP1_1` | `--disable-http1-1` | off | Require HTTP/2 or better; reject HTTP/1.1 connections. |
| `AHMA_HANDSHAKE_TIMEOUT` | — | `45` | MCP handshake timeout in seconds. The server closes a session that does not complete the MCP initialize/notifications/initialized exchange within this window. |

```bash
# HTTP bridge on port 8080, TCP only, strict HTTP/2+
AHMA_DISABLE_QUIC=1 AHMA_DISABLE_HTTP1_1=1 ahma-mcp serve http --port 8080

# Extend handshake timeout for slow clients
AHMA_HANDSHAKE_TIMEOUT=120 ahma-mcp serve http
```

---

## Install Script

The following variable is only read by the install script (`scripts/install.sh`) and has no
effect on the `ahma-mcp` binary itself.

| Variable | Description |
|---|---|
| `AHMA_PREFER_MUSL` | Set to `1` to force the musl-linked Linux binary (auto-detected on Alpine / musl systems). |

---

## Quick reference

```
AHMA_TOOLS_DIR             Path to .ahma/ JSON tool definitions
AHMA_TIMEOUT               Tool execution timeout (seconds, default 360)
AHMA_SYNC                  Force synchronous execution (1=yes)
AHMA_HOT_RELOAD            Reload tools on file change (1=yes)
AHMA_SKIP_PROBES           Skip tool availability probes (1=yes)
AHMA_PROGRESSIVE_DISCLOSURE_OFF  Expose all tools immediately (1=yes)

AHMA_DISABLE_SANDBOX       Disable kernel sandbox — UNSAFE (1=yes)
AHMA_SANDBOX_SCOPE         Colon-separated sandbox scope dirs
AHMA_SANDBOX_DEFER         Defer sandbox until client provides roots (1=yes)
AHMA_WORKING_DIRS          Fallback dirs for deferred sandbox
AHMA_TMP_ACCESS            Add temp dir to sandbox scope (1=yes)
AHMA_DISABLE_TEMP          Block all temp dir access (1=yes)

RUST_LOG                   Log verbosity (debug | info | warn | error)
AHMA_LOG_TARGET            Log destination (stderr | file)
AHMA_LOG_MONITOR           Enable live log monitoring (1=yes)
AHMA_MONITOR_RATE_LIMIT    Min seconds between log alerts (default 60)

AHMA_DISABLE_QUIC          Disable HTTP/3 QUIC (1=yes; also --disable-quic)
AHMA_DISABLE_HTTP1_1       Require HTTP/2+ (1=yes; also --disable-http1-1)
AHMA_HANDSHAKE_TIMEOUT     MCP handshake timeout seconds (default 45)
```
