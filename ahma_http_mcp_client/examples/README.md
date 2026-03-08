# AHMA HTTP MCP Client Examples

This directory contains example programs demonstrating the use of the AHMA HTTP MCP client.

## Available Examples

### stress_test

A comprehensive stress test for the AHMA HTTP Bridge server. This example spawns a server and runs multiple concurrent clients to test performance, reliability, and error handling.

**Features:**
- Spawns and monitors an HTTP server (with automatic error detection)
- Runs multiple concurrent clients (configurable: 1 sync + N async)
- Randomized command execution for realistic load patterns
- Real-time statistics and progress reporting
- Immediate shutdown on server errors
- Comprehensive final report with success rates and throughput

**Usage:**

```bash
# Run with default settings (port 7634, 60 seconds, 1 sync + 3 async clients)
cargo run --example stress_test

# Custom configuration
cargo run --example stress_test -- --port 8080 --duration 120 --async-clients 5

# Use a pre-built binary instead of cargo run
cargo run --example stress_test -- --binary ./target/release/ahma-mcp

# See all options
cargo run --example stress_test -- --help
```

**Command Pool:**

The stress test runs safe, read-only commands that exercise the server without modifying the repository:
- `echo` - Basic command execution
- `ls -la` - Directory listing
- `whoami` - User context
- `date` - System commands
- `pwd` - Working directory
- `uname -a` - System information
- `printenv PATH` - Environment access

**Output:**

The test provides real-time progress updates and a comprehensive final report:

```
╔═══════════════════════════════════════════════════════════╗
║       AHMA HTTP Bridge Stress Test                        ║
╚═══════════════════════════════════════════════════════════╝

Configuration:
  Port: 7634
  Duration: 60s
  Async clients: 3
  Sync clients: 1

Starting server...
OK Server is healthy

Starting 4 client(s)...
OK Sync client initialized
OK Async client 1 initialized
OK Async client 2 initialized
OK Async client 3 initialized

Running stress test...

[ 15s] Success: 450 | Errors: 0 | Rate: 30/s
[ 30s] Success: 920 | Errors: 0 | Rate: 30/s
[ 45s] Success: 1385 | Errors: 0 | Rate: 30/s
[ 60s] Success: 1842 | Errors: 0 | Rate: 30/s

OK Test duration completed

╔═══════════════════════════════════════════════════════════╗
║                     Test Results                          ║
╚═══════════════════════════════════════════════════════════╝
  Total requests: 1842
  Successful: 1842 (100.0%)
  Errors: 0
  Duration: 60.1s
  Average rate: 30.6 req/s

OK Test completed successfully
```

**Error Handling:**

The stress test monitors server stderr and will immediately stop if any error is detected:
- Server compilation or startup errors → Immediate abort
- Runtime errors in server logs → Immediate shutdown
- Client errors → Counted and reported, but test continues
- Final report includes error counts and server status

**Stress Test Usage:**

This offers several advantages over simpler stress test clients and scripts:

1. **Multi-threaded:** True concurrent execution using Tokio
2. **Immediate error detection:** Monitors server stderr in real-time
3. **Better statistics:** Per-second rate, success rate, comprehensive reporting
4. **Configurable:** Command-line arguments for all parameters
5. **More reliable:** Type-safe Rust implementation with proper error handling

### oauth_mcp_client

OAuth authentication example for MCP clients connecting to enterprise services.
