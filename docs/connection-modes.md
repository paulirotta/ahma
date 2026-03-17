# Connection Modes

`ahma-mcp` supports three connection modes for different integration scenarios.

## 1. STDIO Mode (Default)

The IDE spawns `ahma-mcp` as a subprocess and communicates via standard I/O. This is the recommended mode for development because:

- The IDE sets `cwd` to `${workspaceFolder}`, so the sandbox scope is automatic.
- Each workspace gets its own sandboxed server instance.
- No network exposure.

```bash
ahma-mcp --mode stdio
```

### mcp.json examples

**VS Code** (`~/.vscode/mcp.json` or `.vscode/mcp.json`):

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

**Cursor** (`~/.cursor/mcp.json`):

```json
{
    "mcpServers": {
        "Ahma": {
            "type": "stdio",
            "command": "ahma-mcp",
            "args": ["--tmp", "--livelog", "--simplify"]
        }
    }
}
```

**Claude Code** (`~/.claude/mcp.json`):

```json
{
    "mcpServers": {
        "Ahma": {
            "type": "stdio",
            "command": "ahma-mcp",
            "args": ["--tmp", "--livelog", "--simplify"]
        }
    }
}
```

**Antigravity** (uses bash wrapper to pass explicit scope — Antigravity doesn't send `roots/list`):

```json
{
  "mcpServers": {
    "Ahma": {
      "command": "bash",
      "args": ["-c", "ahma-mcp --simplify --rust --sandbox-scope $HOME/github"]
    }
  }
}
```

## 2. HTTP Bridge Mode

HTTP server that proxies MCP protocol to a stdio subprocess. Used for web clients, remote agents, debugging, or multi-client scenarios.

```bash
# Start on default port 3000 (sandbox scope from roots/list)
ahma-mcp --mode http

# Explicit sandbox scope (for clients that don't send roots/list)
ahma-mcp --mode http --sandbox-scope /path/to/your/project

# Via environment variable
export AHMA_SANDBOX_SCOPE=/path/to/your/project
ahma-mcp --mode http

# Custom port and host
ahma-mcp --mode http --http-port 8080 --http-host 127.0.0.1
```

**Security note**: HTTP mode is for local development only. Do not expose to untrusted networks.

| Feature | STDIO Mode | HTTP Mode |
|---------|-----------|----------|
| Sandbox scope | Set by IDE via `cwd` | Per session via `roots/list` |
| Per-project isolation | Automatic | Automatic (per session) |
| Configuration | `mcp.json` in IDE | CLI args or env vars |
| Use case | Standard IDE integration | Debugging, advanced setups |

## 3. HTTP Streaming (MCP Streamable HTTP)

Ahma implements [MCP Streamable HTTP](https://spec.modelcontextprotocol.io/) for multiplexed, reconnection-resilient communication.

**POST with JSON response:**

```bash
curl -X POST http://localhost:3000/mcp \
  -H "Content-Type: application/json" \
  -H "Accept: application/json" \
  -H "Mcp-Session-Id: <session-uuid>" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}'
```

**POST with SSE streaming:**

```bash
curl -X POST http://localhost:3000/mcp \
  -H "Content-Type: application/json" \
  -H "Accept: text/event-stream" \
  -H "Mcp-Session-Id: <session-uuid>" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}'
```

**Reconnect with event replay:**

```bash
curl -X GET http://localhost:3000/mcp \
  -H "Accept: text/event-stream" \
  -H "Mcp-Session-Id: <session-uuid>" \
  -H "Last-Event-Id: 42"
# Receives all events with ID > 42
```

| Feature | SSE | HTTP Streaming |
|---------|-----|----------------|
| Content negotiation | GET only | POST with `Accept: text/event-stream` |
| Event sequencing | None | SSE `id:` field for ordering and replay |
| Reconnection | Client guesses missed events | `Last-Event-Id` enables precise replay |
| Full-duplex | Separate streams | Single POST stream |
| Multiplexing | Limited | Native per-session |

## Session Isolation

In HTTP mode, each MCP session gets its own sandbox scope derived from the `roots/list` response. See [docs/session-isolation.md](session-isolation.md) for details.

## HTTP/3 (QUIC)

All HTTP clients in Ahma prefer HTTP/3 (QUIC) when the server advertises support via `Alt-Svc` headers, with transparent fallback to HTTP/2 and HTTP/1.1.
