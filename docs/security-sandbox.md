# Security Sandbox

Ahma enforces **kernel-level filesystem sandboxing** by default. The sandbox scope is set once at server startup and cannot be changed — the AI has full access within the scope but zero access outside it, regardless of how commands are constructed.

## Why Kernel-Level Sandboxing?

Trust-based security ("do you trust this tool?") doesn't protect against mistakes or manipulation at speed. Kernel-enforced boundaries do:

- **String filters can be bypassed** via path traversal, symlinks, or creative shell expansion. Kernel-level policies cannot.
- **Blast radius is bounded** — even if an AI agent tries `rm -rf ~`, the kernel rejects any write outside the workspace.
- **No runtime overhead** — the policy is applied once at sandbox lock and enforced by the OS.

## Sandbox Scope

The sandbox scope is the root directory boundary for all filesystem operations:

- **STDIO mode**: Defaults to the current working directory (`--cwd` set by the IDE). In `mcp.json`, set `"cwd": "${workspaceFolder}"` and the sandbox "just works".
- **HTTP mode**: Set once when the server starts. Configure via:
  1. `--sandbox-scope <path>` (highest priority)
  2. `AHMA_SANDBOX_SCOPE` environment variable
  3. Current working directory (default)

**Security invariant**: Once the sandbox scope is set, it cannot be changed for the lifetime of the server process. Any attempt to change it after lock terminates the session.

## Platform-Specific Enforcement

### Linux (Landlock)

On Linux, Ahma uses [Landlock](https://docs.kernel.org/userspace-api/landlock.html) — a kernel LSM that applies fine-grained filesystem access rules in-process with no daemon or capability escalation required.

**Requirements**: Linux kernel 5.13 or newer (released June 2021). The server refuses to start on older kernels unless sandbox is explicitly disabled.

```bash
uname -r                            # check kernel version
cat /sys/kernel/security/lsm        # verify landlock is active
```

**Older kernels / Raspberry Pi**: Landlock requires kernel ≥ 5.13. On older Pi OS kernels, run with:

```bash
export AHMA_NO_SANDBOX=1
ahma-mcp --mode stdio
```

or add `"--no-sandbox"` to `mcp.json` args.

### macOS (Seatbelt)

On macOS, Ahma uses Apple's built-in `sandbox-exec` with a generated Seatbelt profile (SBPL) that restricts write access to the sandbox scope. No additional installation required.

**Requirements**: Any modern macOS version. `sandbox-exec` is built into macOS.

### Windows (Job Objects + AppContainer)

On Windows, Ahma uses Job Object enforcement (`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`) at startup, with AppContainer profile DACL grants for per-scope access control. PowerShell (5.1+) is the shell. See [SPEC.md R6.3](../SPEC.md) for status.

## Nested Sandbox Environments

When running inside Cursor, VS Code, or Docker, the outer environment may prevent Ahma from applying its own sandbox. Ahma detects this and exits with instructions.

**Manual override** (when you know the outer environment is safe):

```bash
ahma-mcp --no-sandbox
# or
export AHMA_NO_SANDBOX=1
```

Common `mcp.json` for nested environments (VS Code with workspace scoping):

```json
{
    "servers": {
        "Ahma": {
            "type": "stdio",
            "command": "ahma-mcp",
            "args": ["--tmp", "--livelog", "--simplify"]
        }
    }
}
```

## Temp Directory Access (`--tmp`)

By default, the system temp directory is accessible only via platform-implicit rules. Use `--tmp` (or `AHMA_TMP_ACCESS=1`) to add it as an explicit read/write scope — useful for compilers and build tools.

| Flag combination | Behavior |
|-----------------|----------|
| (default) | Temp access via platform rules |
| `--tmp` | Temp dir added as explicit scope |
| `--no-temp-files` | Temp access blocked entirely |
| `--tmp --no-temp-files` | `--no-temp-files` wins (blocked) |

**Security considerations**: `/tmp` is shared by all users and processes. Use `mktemp` with random suffixes to avoid TOCTOU attacks. Clean up sensitive temp files after use.

## Live Log Monitoring (`--livelog`)

The `--livelog` flag grants additional read-only access to specific log files via symlinks in the `log/` directory at server startup — see [live-log-monitoring.md](live-log-monitoring.md) and [SPEC.md R9](../SPEC.md).
