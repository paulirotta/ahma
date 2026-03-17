# Live Log Monitoring

Ahma's livelog feature turns any long-running streaming command into an LLM-powered monitoring tool. Instead of flooding the AI with raw log output, Ahma accumulates lines into time/size-bounded chunks and asks a local or cloud LLM to detect issues described in plain English. When an issue is found, a concise alert is pushed as an MCP progress notification. Between alerts, a configurable cooldown window prevents alert storms.

## How It Works

```
source_command  →  chunk accumulator  →  LLM  →  ProgressUpdate::LogAlert
  (adb logcat)      (50 lines / 30s)    detect      (pushed to MCP client)
```

1. `tools/call` on a livelog tool returns an `operation_id` immediately (async).
2. A background pipeline spawns the `source_command` inside Ahma's kernel sandbox.
3. Lines are buffered until `chunk_max_lines` is reached or `chunk_max_seconds` elapses.
4. The chunk is sent to the LLM with your `detection_prompt`.
5. If the LLM detects an issue (any response other than `"CLEAN"`), a `LogAlert` notification is pushed to the MCP client — but only if the `cooldown_seconds` window has elapsed since the last alert.
6. Use `cancel <operation_id>` to stop monitoring.

## Android Logcat Monitoring

### Prerequisites

- **ADB** installed and on `PATH` (`brew install android-platform-tools` or via Android Studio)
- Device or emulator connected (`adb devices` should show it)
- **Ollama** running locally: `brew install ollama && ollama serve`
- `llama3.2` model pulled: `ollama pull llama3.2`
- `--livelog` flag added to your Ahma `mcp.json` args (enables the livelog symlink sandbox feature)

### Setup

1. Copy the example tool definition into your project's `.ahma/` directory:

```bash
mkdir -p .ahma
cp /path/to/ahma/.ahma/android_logcat.json .ahma/
```

Or create `.ahma/android_logcat.json` with the content below.

2. Add `--livelog` to your `mcp.json` (VS Code example):

```json
{
    "servers": {
        "Ahma": {
            "type": "stdio",
            "command": "ahma-mcp",
            "args": ["--tmp", "--livelog"]
        }
    }
}
```

3. Reload your MCP server configuration (restart VS Code or run "MCP: Restart Server").

### Example Tool Definition

```json
{
    "name": "android_logcat",
    "description": "Monitor Android application logs via ADB logcat, using an LLM to detect crashes, exceptions, and ANR errors in real-time. Returns immediately with an operation ID; push alerts are delivered as MCP progress notifications whenever the LLM detects an issue. Use `cancel` with the operation ID to stop monitoring.",
    "command": "adb",
    "tool_type": "livelog",
    "enabled": true,
    "livelog": {
        "source_command": "adb",
        "source_args": ["-d", "logcat", "-v", "threadtime"],
        "detection_prompt": "Look for crashes (FATAL EXCEPTION, NullPointerException, IllegalStateException), Application Not Responding (ANR) errors, native crashes (SIGSEGV, SIGABRT), or any log line at level E (ERROR) or F (FATAL) that indicates a real problem rather than a known-harmless library warning.",
        "llm_provider": {
            "base_url": "http://localhost:11434/v1",
            "model": "llama3.2"
        },
        "chunk_max_lines": 50,
        "chunk_max_seconds": 30,
        "cooldown_seconds": 60,
        "llm_timeout_seconds": 30
    },
    "hints": {
        "custom": {
            "usage": "Call this tool to start live monitoring of Android logs. The tool returns an operation ID immediately. You will receive progress notifications when the LLM detects crashes or errors.",
            "prerequisites": "ADB must be installed and on PATH. Device/emulator connected (`adb devices`). Ollama running locally on port 11434 with `llama3.2` available (`ollama pull llama3.2`).",
            "stopping": "Use `cancel <operation_id>` to stop monitoring gracefully."
        }
    }
}
```

### Starting Monitoring

In your MCP client (VS Code, Cursor, etc.), call the `android_logcat` tool:

```
Use the android_logcat tool to start monitoring my device logs for crashes.
```

The tool returns immediately with an operation ID, for example:

```
Started live log monitoring. Operation ID: op_abc123
You will be notified of any detected issues.
```

### Stopping Monitoring

```
Cancel operation op_abc123
```

Or use the built-in `cancel` tool with the operation ID.

### What You'll See

When an issue is detected, you receive a progress notification like:

```
[android_logcat] LLM alert: FATAL EXCEPTION in com.example.app — NullPointerException at
MainActivity.onCreate(MainActivity.kt:42). Triggered by 3 lines at 14:32:06.
```

---

## Other Use Cases

### Remote Server Logs (SSH + tail)

```json
{
    "name": "server_error_monitor",
    "description": "Monitor remote server error logs for critical issues.",
    "command": "ssh",
    "tool_type": "livelog",
    "enabled": true,
    "livelog": {
        "source_command": "ssh",
        "source_args": ["user@myserver.example.com", "tail", "-f", "/var/log/app/error.log"],
        "detection_prompt": "Look for ERROR or FATAL log entries, stack traces, out-of-memory messages, or database connection failures. Ignore INFO and WARN level messages.",
        "llm_provider": {
            "base_url": "http://localhost:11434/v1",
            "model": "llama3.2"
        },
        "chunk_max_seconds": 15,
        "cooldown_seconds": 30
    }
}
```

### Docker Container Logs

```json
{
    "name": "docker_logs",
    "description": "Monitor a Docker container for errors.",
    "command": "docker",
    "tool_type": "livelog",
    "enabled": true,
    "livelog": {
        "source_command": "docker",
        "source_args": ["logs", "-f", "my-container"],
        "detection_prompt": "Look for unhandled exceptions, panic messages, connection refused errors, or out-of-memory kills.",
        "llm_provider": {
            "base_url": "http://localhost:11434/v1",
            "model": "llama3.2"
        }
    }
}
```

### Local File (tail -f)

```json
{
    "name": "app_log_monitor",
    "description": "Monitor a local application log file.",
    "command": "tail",
    "tool_type": "livelog",
    "enabled": true,
    "livelog": {
        "source_command": "tail",
        "source_args": ["-f", "logs/app.log"],
        "detection_prompt": "Look for ERROR level log entries, uncaught exceptions, or timeout messages.",
        "llm_provider": {
            "base_url": "http://localhost:11434/v1",
            "model": "llama3.2"
        }
    }
}
```

---

## Using a Cloud LLM Provider

If you prefer a cloud API (e.g. OpenAI) instead of Ollama, add an `api_key` and point `base_url` to the provider:

```json
"llm_provider": {
    "base_url": "https://api.openai.com/v1",
    "model": "gpt-4o-mini",
    "api_key": "sk-..."
}
```

> **Privacy note**: Using a cloud provider sends your log content to that provider. For sensitive production logs, prefer a local model (Ollama, LM Studio, etc.) or ensure your cloud provider has appropriate data handling agreements.

---

## Configuration Reference

See [SPEC.md Section 5.5](../SPEC.md) for the full `LivelogConfig` and `LlmProviderConfig` field reference.

The MTDF JSON schema (which includes `LivelogConfig`) is at [docs/mtdf-schema.json](mtdf-schema.json).
