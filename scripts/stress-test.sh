#!/bin/bash
#
# DEPRECATED: This script has been replaced by a Rust example
#
# The new stress test is located at:
#   ahma_http_mcp_client/examples/stress_test.rs
#
# Usage:
#   cargo run --example stress_test -- --help
#   cargo run --example stress_test -- --port 7634 --duration 60
#
# The new implementation provides:
#   - Multi-threaded concurrent clients (1 sync + 3 async by default)
#   - Immediate error detection from server stderr
#   - Better performance and reliability
#   - Detailed statistics reporting
#   - Configurable port, duration, and client count
#
# For backwards compatibility, this script now invokes the Rust example:

exec cargo run --release --example stress_test -- "$@"

# Resolve project root
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_ROOT"

# Default values
QUICK_MODE=false
VERBOSE=false
SERVER_PID=""
HTTP_PORT=""

# Parse arguments
for arg in "$@"; do
    case $arg in
        --quick)
            QUICK_MODE=true
            ;;
        --verbose)
            VERBOSE=true
            ;;
        *)
            echo "Unknown option: $arg"
            echo "Usage: $0 [--quick] [--verbose]"
            exit 1
            ;;
    esac
done

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

# Cleanup function - kills server on exit
cleanup() {
    echo -e "\n${YELLOW}CLEAN Cleaning up...${NC}"
    
    if [[ -n "$SERVER_PID" ]] && kill -0 "$SERVER_PID" 2>/dev/null; then
        echo -e "   Stopping server (PID $SERVER_PID)..."
        kill "$SERVER_PID" 2>/dev/null || true
        wait "$SERVER_PID" 2>/dev/null || true
        echo -e "   ${GREEN}Server stopped${NC}"
    fi
    
    echo -e "${YELLOW}OK Cleanup complete${NC}"
}

# Set trap for cleanup on any exit (success, error, or Ctrl+C)
trap cleanup EXIT INT TERM

echo -e "${BLUE}╔══════════════════════════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║       Ahma HTTP Bridge Stress Test Suite                     ║${NC}"
echo -e "${BLUE}╚══════════════════════════════════════════════════════════════╝${NC}"
echo

# Step 1: Build the project
echo -e "${CYAN}Building project (release mode)...${NC}"
cargo build --release -p ahma-mcp --bin ahma-mcp 2>&1 | tail -5
echo -e "${GREEN}OK Build complete${NC}"
echo

# Step 2: Find a random available port
echo -e "${CYAN}Finding available port...${NC}"
# Use port 0 to let the OS assign a port, we'll extract it from server output
TEMP_PORT_FILE=$(mktemp)

# Step 3: Start the HTTP server
echo -e "${CYAN}Starting HTTP server...${NC}"
TOOLS_DIR="$PROJECT_ROOT/.ahma/tools"
SANDBOX_SCOPE="$PROJECT_ROOT"

# Start server in background, capturing output
if $VERBOSE; then
    # Verbose: show all server output
    "$PROJECT_ROOT/target/release/ahma-mcp" \
        --mode http \
        --http-port 0 \
        --sync \
        --tools-dir "$TOOLS_DIR" \
        --sandbox-scope "$SANDBOX_SCOPE" \
        --log-to-stderr \
        2>&1 | tee "$TEMP_PORT_FILE" &
else
    # Normal: only capture, don't show
    "$PROJECT_ROOT/target/release/ahma-mcp" \
        --mode http \
        --http-port 0 \
        --sync \
        --tools-dir "$TOOLS_DIR" \
        --sandbox-scope "$SANDBOX_SCOPE" \
        --log-to-stderr \
        > /dev/null 2> "$TEMP_PORT_FILE" &
fi

SERVER_PID=$!
echo -e "   Server PID: $SERVER_PID"

# Wait for server to report its port
echo -e "   Waiting for server to start..."
TIMEOUT=30
ELAPSED=0
while [[ $ELAPSED -lt $TIMEOUT ]]; do
    if grep -q "AHMA_BOUND_PORT=" "$TEMP_PORT_FILE" 2>/dev/null; then
        HTTP_PORT=$(grep "AHMA_BOUND_PORT=" "$TEMP_PORT_FILE" | head -1 | sed 's/AHMA_BOUND_PORT=//')
        break
    fi
    
    # Check if server died
    if ! kill -0 "$SERVER_PID" 2>/dev/null; then
        echo -e "${RED}FAIL Server process died unexpectedly${NC}"
        echo "Server output:"
        cat "$TEMP_PORT_FILE"
        exit 1
    fi
    
    sleep 0.2
    ELAPSED=$((ELAPSED + 1))
done

if [[ -z "$HTTP_PORT" ]]; then
    echo -e "${RED}FAIL Timeout waiting for server to report port${NC}"
    echo "Server output:"
    cat "$TEMP_PORT_FILE"
    exit 1
fi

rm -f "$TEMP_PORT_FILE"
echo -e "${GREEN}OK Server running on port $HTTP_PORT${NC}"
echo

# Step 4: Wait for health check
echo -e "${CYAN}Checking server health...${NC}"
SERVER_URL="http://127.0.0.1:$HTTP_PORT"
HEALTH_OK=false

for i in {1..20}; do
    if curl -s "$SERVER_URL/health" > /dev/null 2>&1; then
        HEALTH_OK=true
        break
    fi
    sleep 0.2
done

if ! $HEALTH_OK; then
    echo -e "${RED}FAIL Server health check failed${NC}"
    exit 1
fi

echo -e "${GREEN}OK Server is healthy${NC}"
echo

# Step 5: Run stress tests
echo -e "${BLUE}╔══════════════════════════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║                    Running Stress Tests                      ║${NC}"
echo -e "${BLUE}╚══════════════════════════════════════════════════════════════╝${NC}"
echo

# Export the server URL for tests
export AHMA_TEST_SSE_URL="$SERVER_URL"

# Determine which tests to run
if $QUICK_MODE; then
    echo -e "${YELLOW}📝 Quick mode: running subset of stress tests${NC}"
    TEST_FILTER="-E test(test_concurrent_tool_calls)"
else
    echo -e "${YELLOW}📝 Full mode: running all stress tests${NC}"
    TEST_FILTER="--run-ignored"
fi

echo
echo -e "${CYAN}═══ Session Stress Tests (session_stress_test.rs) ═══${NC}"
echo
cargo nextest run -p ahma-http-bridge --test session_stress_test 2>&1 || {
    echo -e "${YELLOW}WARNING️  Some session stress tests may have failed${NC}"
}

echo
echo -e "${CYAN}═══ Handshake State Machine Tests ═══${NC}"
echo
cargo nextest run -p ahma-http-bridge --test handshake_state_machine_test 2>&1 || {
    echo -e "${YELLOW}WARNING️  Some handshake tests may have failed${NC}"
}

echo
echo -e "${CYAN}═══ Concurrent Tool Call Tests (normally ignored) ═══${NC}"
echo
cargo nextest run -p ahma-http-bridge --test sse_tool_integration_test $TEST_FILTER 2>&1 || {
    echo -e "${YELLOW}WARNING️  Some concurrent tool tests may have failed${NC}"
}

echo
echo -e "${BLUE}╔══════════════════════════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║                    Stress Test Complete                      ║${NC}"
echo -e "${BLUE}╚══════════════════════════════════════════════════════════════╝${NC}"
echo

echo -e "${GREEN}OK Stress test suite finished${NC}"
echo -e "   Server ran on port: ${CYAN}$HTTP_PORT${NC}"
echo -e "   Server URL was: ${CYAN}$SERVER_URL${NC}"
