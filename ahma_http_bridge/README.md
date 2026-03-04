# Ahma HTTP Bridge

A simple HTTP-to-stdio bridge for MCP (Model Context Protocol) servers.

## Overview

This crate provides an HTTP server that acts as a bridge between HTTP clients and stdio-based MCP servers. It spawns an MCP server as a subprocess and proxies JSON-RPC messages between HTTP requests and the server's stdin/stdout.

**Status: Complete and Production-Ready** OK

## Features

- **Simple Integration**: Works with any stdio-based MCP server
- **Automatic Process Management**: Spawns and manages the MCP server subprocess
- **Auto-Restart**: Automatically restarts the server if it crashes
- **CORS Support**: Allows cross-origin requests for web clients
- **Health Check**: Provides a `/health` endpoint for monitoring

## Usage

```bash
cargo build --relese
```

### Cursor MCP

```json
{
    "mcpServers": {
        "ahma-stdio-debug-vscode": {
            "cwd": "${workspaceFolder}",
            "command": "/path/to/ahma/target/release/ahma_mcp",
            "args": [
                "--tools-dir",
                "/path/to/ahma/.ahma",
                "--log-to-stderr"
            ],
            "env": {
                "RUST_LOG": "debug"
            }
        }
    }
}
```

### LM Studio MCP

```json
{
  "mcpServers": {
    "ahma": {
      "cwd": "/path/to/working/directory",
      "command": "/path/to/ahma/target/release/ahma_mcp",
      "args": [
        "--sync",
        "--tools-dir",
        "/path/to/ahma/.ahma",
        "--log-to-stderr"
      ],
      "env": {
        "RUST_LOG": "debug"
      }
    }
  }
}
```

#### VSCode MCP

```json
{
    "servers": {
        "ahma": {
            "cwd": "/path/to/working/directory",
            "command": "/path/to/ahma/target/release/ahma_mcp",
            "args": [
                "--sync",
                "--tools-dir",
                "/path/to/ahma/.ahma",
                "--log-to-stderr"
            ],
        }
    }
}
```

### As a Library

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = BridgeConfig {
        bind_addr: "127.0.0.1:3000".parse().unwrap(),
        server_command: "ahma_mcp".to_string(),
        server_args: vec!["--tools-dir".to_string(), "./tools".to_string()],
    
### Command Line

The bridge is integrated into the `ahma_shell` binary:

```bash
# Start HTTP bridge on default port (3000)
# Clients must provide roots/list unless explicit fallback scope is configured
ahma_mcp --mode http

# Explicit fallback scope for clients without roots/list support
ahma_mcp --mode http --sandbox-scope /path/to/project

# Start on custom port
ahma_mcp --mode http --http-port 8080

# Start with specific tools directory
ahma_mcp --mode http --tools-dir ./my-tools
```

## Port Configuration

| Environment | Port | Notes |
| ----------- | ---- | ----- |
| **Production** | 3000 (default) | Configurable via `--http-port` |
| **Integration Tests** | 5721 (reserved) | Hardcoded constant, do not change |

### Why Port 5721 for Tests?

Integration tests use a dedicated port (5721) to ensure:

1. **Isolation**: Tests never accidentally connect to a production server on port 3000
2. **Reproducibility**: All tests use the same port for consistent behavior
3. **Debugging**: Port collisions immediately reveal concurrent test issues

If port 5721 is already in use when running tests, they will fail loudly. This indicates either:

- Another test process is running concurrently
- A previous test crashed and left an orphaned process

To fix: Kill any existing ahma_mcp processes using port 5721.

## Endpoints

### POST /mcp

Send JSON-RPC messages to the MCP server.

**Request:**

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "tools/list",
  "params": {}
}
```

**Response:**

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "tools": [...]
  }
}
```

### GET /health

Health check endpoint.

**Response:**

```
OK
```

### GET /mcp

Server-Sent Events stream for real-time notifications (per MCP Streamable HTTP transport spec).

- Streams JSON-RPC notifications emitted by the MCP server
- Includes keep-alive comments to maintain the connection
- Same endpoint as POST /mcp, differentiated by HTTP method

## How It Works

1. HTTP bridge starts and binds to the specified address
2. On first request, it spawns the MCP server as a subprocess
3. HTTP requests are converted to JSON-RPC and sent to server's stdin
4. Server responses from stdout are returned as HTTP responses
5. If the server crashes, it's automatically restarted on the next request

## Architecture

```
┌─────────────┐
│ HTTP Client │
└──────┬──────┘
       │ HTTP POST /mcp
       │ JSON-RPC Request
       ▼
┌──────────────────┐
│  HTTP Bridge     │
│  (This Crate)    │
└──────┬───────────┘
       │ stdin (JSON-RPC)
       │
       ▼
┌──────────────────┐
│  MCP Server      │
│  (stdio mode)    │
└──────┬───────────┘
       │ stdout (JSON-RPC)
       │
       ▼
┌──────────────────┐
│  HTTP Bridge     │
└──────┬───────────┘
       │ HTTP Response
       │ JSON-RPC Result
       ▼
┌─────────────┐
│ HTTP Client │
└─────────────┘
```

## Testing

A test script is provided:

```bash
./test_http_bridge.sh
```

This script:

1. Starts the HTTP bridge
2. Tests the health endpoint
3. Sends MCP initialize request
4. Sends tools/list request
5. Verifies responses
6. Cleans up

## Benefits

### For Users

- **Easy HTTP Access**: No need to understand stdio transport
- **Web Integration**: Can be called from web browsers and HTTP clients
- **Testing**: Easy to test with curl or Postman
- **Debugging**: Can inspect requests/responses with HTTP tools

### For Developers

- **Clean Separation**: Bridge is a separate crate
- **Reusable**: Can be used as a library in other projects
- **Maintainable**: Simple, focused implementation (~250 lines)
- **Extensible**: Easy to add features like authentication, rate limiting, etc.

## Limitations

- **No Server-Initiated Messages**: HTTP is request-response, so server-initiated notifications are not supported
- **Single Request at a Time**: Requests are serialized through the subprocess
- **No Streaming**: Large responses are buffered completely

For production use with server-initiated messages, consider using WebSocket.

## Security Model

The HTTP bridge implements a sandboxing security model to restrict AI-generated commands to the client's workspace.

### What's Protected OK

1. **File Write Access**: Write operations are restricted to the client's workspace (sandbox scope)
2. **Per-Session Isolation**: Each client session gets its own sandbox scope derived from workspace roots
3. **Immutable Sandbox**: Once locked, the sandbox scope cannot be changed (prevents escalation attacks)
4. **Path Traversal Protection**: Attempts to escape via `../` or symlinks are blocked

### How Sandboxing Works

- **macOS**: Uses `sandbox-exec` with Seatbelt profiles per command
- **Linux**: Uses Landlock (kernel 5.13+) for process-level restrictions

### High-Security Mode: `--no-temp-files`

For environments requiring stricter security, the `--no-temp-files` flag blocks writes to temp directories:

```bash
ahma_http_bridge --no-temp-files
```

This prevents data exfiltration via `/tmp` or `/var/folders` but breaks tools that require temp file access.

### Known Security Limitations WARNING️

These are accepted trade-offs for practical operation:

| Limitation | Risk | Mitigation |
| ------------ | ------ | ------------ |
| **Read access unrestricted** | AI can read any file (including `~/.ssh/id_rsa`) | Required for shells to function; outer sandbox recommended |
| **Network unrestricted** | Data exfiltration via network | Future: optional `--restrict-network` |
| **Temp dirs writable** | Persistence outside sandbox | Use `--no-temp-files` for high-security |
| **No authentication** | Localhost access is trusted | Only bind to 127.0.0.1 |

### Trust Model

- **Local Development Only**: The HTTP bridge is designed for local use on developer machines
- **Trusted Client**: Clients on localhost are trusted (no authentication)
- **Outer Sandbox**: When running inside Cursor/VS Code, the IDE's sandbox provides additional protection

### Red Team Testing

Security tests verify that sandbox escape attempts fail:

- Path traversal (`../../../etc/passwd`)
- Symlink escapes (link inside workspace pointing outside)
- Absolute path escapes (`/etc/passwd`)
- Command injection via path

See `tests/sandbox_security_test.rs` and `ahma_mcp/tests/sandbox_security_red_team_test.rs`.
