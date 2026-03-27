# Connection Modes

`ahma-mcp` supports:
1. **STDIO Mode** (default): IDE spawns `ahma-mcp` as a subprocess and communicates via standard I/O. Recommended for development.
2. **HTTP Mode**: Start `ahma-mcp --mode http` for HTTP/3 (QUIC) support.

## 1. STDIO Mode (Default)

The IDE spawns `ahma-mcp` as a subprocess and communicates via standard I/O. This is the recommended mode for development because:

- The IDE sets `cwd` to `${workspaceFolder}`, so the sandbox scope is automatic.
- Each workspace gets its own sandboxed server instance.
- No network exposure.

```bash
ahma-mcp --mode stdio
```

### mcp.json examples

**VS Code** (user profile `mcp.json` or `.vscode/mcp.json`):

> The user-level `mcp.json` lives in your VS Code profile folder:
> macOS `~/Library/Application Support/Code/User/mcp.json`,
> Linux `~/.config/Code/User/mcp.json`,
> Windows `%APPDATA%\Code\User\mcp.json`.
> Or run `MCP: Open User Configuration` from the Command Palette.

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

Alternatively, in a terminal run `ahma-mcp --mode http` for visibility of all actions, and use:

```json
{
    "servers": {
        "Ahma-http": {
            "type": "http",
            "url": "http://localhost:3000/mcp"
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

**Claude Code** (`~/.claude.json`):

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

## 2. HTTP Mode (EXPERIMENTAL)

**IMPORTANT SECURITY NOTE**: HTTP mode is for local development only. Do not expose to untrusted networks. OAuth pass through is not yet fully implemented, so all tools are available to any client that can connect. Use firewall rules or `--http-host` to restrict access.

First start the server in a terminal with your preferred flags, defaulting to port 3000:

```bash
ahma-mcp --mode http --tmp --livelog --simplify
```

The HTTP server requires **HTTP/2 or HTTP/3**. HTTP/1.1 connections are explicitly rejected.

- **HTTP/2** (h2c — cleartext, no TLS required): the default transport for all HTTP clients.
- **HTTP/3** (QUIC): clients that support HTTP/3 and Alt-Svc negotiation may upgrade automatically. Server-side QUIC is not yet implemented; clients fall back to HTTP/2.

Default endpoint: `http://localhost:3000/mcp`

- `POST /mcp` with `Accept: application/json`: JSON-RPC, preferred for speed and low overhead.
- `POST /mcp` with `Accept: text/event-stream`: Streamable HTTP (SSE fallback for some networks).
- `GET /mcp` with `Accept: text/event-stream`: SSE stream for server-to-client events.

> **Client configuration**: Configure your HTTP client with HTTP/2 prior-knowledge (`--http2-prior-knowledge` in curl, `http2_prior_knowledge()` in reqwest) because the server does not negotiate via ALPN (no TLS).

Then configure your IDE to connect to for example `http://localhost:3000/mcp`:

```json
{
    "servers": {
        "Ahma-http": {
            "type": "http",
            "url": "http://localhost:3000/mcp"
        }
    }
}
```

HTTP server that proxies MCP protocol to a stdio subprocess. Used for web clients, remote agents, debugging, or multi-client scenarios.

#  Used for increased visibility, web clients, remote agents, debugging, or multi-client scenarios. While sandbox scope is automatic in stdio mode, HTTP mode requires your MCP client to support `roots/list` responses. VSCode and Cursor do this automatically. For clients that don't, you can set a fixed sandbox scope (e.g. `--sandbox-scope /path/to/your/project`).

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

Ahma HTTP clients built on `reqwest` prefer HTTP/3 (QUIC) when the remote server advertises support via `Alt-Svc`, with transparent fallback to HTTP/2 and HTTP/1.1.

For this local HTTP bridge endpoint, clients should expect HTTP/2 or HTTP/1.1.
