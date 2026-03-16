#!/bin/bash
# Guardrail lint script for recurring test anti-patterns.
# Focus: duplicated path resolution, legacy helper drift, timeout regressions.

set -euo pipefail

echo "🔍 Checking for improper CARGO_TARGET_DIR usage in tests..."

# Find all Rust test files
VIOLATIONS=0

# Search for CARGO_TARGET_DIR outside of test_utils::cli
while IFS= read -r file; do
    # Skip the allowed file
    if [[ "$file" == *"ahma_mcp/src/test_utils.rs" ]]; then
        continue
    fi
    
    # Skip files that just remove the env var (that's OK)
    if grep -q 'std::env::var("CARGO_TARGET_DIR")' "$file" && ! grep -q 'env_remove("CARGO_TARGET_DIR")' "$file"; then
        echo "FAIL VIOLATION: $file"
        echo "   Found manual CARGO_TARGET_DIR access"
        echo "   Use ahma_mcp::test_utils::cli::get_binary_path() instead"
        echo ""
        VIOLATIONS=$((VIOLATIONS + 1))
    fi
done < <(find . -path "*/tests/*.rs" -o -name "*_test.rs" -o -name "test_*.rs" | grep -v target)

echo "🔍 Checking for deprecated SSE test helper usage..."
while IFS= read -r file; do
    # Defining the helper is allowed only in its canonical module.
    if [[ "$file" == *"ahma_http_bridge/tests/common/sse_test_helpers.rs" ]]; then
        continue
    fi

    if rg -q 'ensure_server_available\(' "$file"; then
        echo "FAIL VIOLATION: $file"
        echo "   Found deprecated ensure_server_available() usage"
        echo "   Use tests/common/setup_test_mcp(...) or setup_test_mcp_for_tools(...)"
        echo ""
        VIOLATIONS=$((VIOLATIONS + 1))
    fi
done < <(find . -path "*/tests/*.rs" -o -name "*_test.rs" -o -name "test_*.rs" | grep -v target)

echo "🔍 Checking timeout literals in handshake-critical integration tests..."
for file in \
    "./ahma_http_bridge/tests/handshake_timeout_test.rs" \
    "./ahma_http_bridge/tests/http_bridge_integration_test.rs"
do
    if [[ -f "$file" ]] && rg -q 'Duration::from_(secs|millis)\([0-9]+\)' "$file"; then
        echo "FAIL VIOLATION: $file"
        echo "   Found literal Duration timeout value"
        echo "   Use TestTimeouts::get/scale_* categories instead"
        echo ""
        VIOLATIONS=$((VIOLATIONS + 1))
    fi
done

echo "🔍 Checking shared custom server spawn usage in HTTP bridge integration test..."
HTTP_BRIDGE_TEST="./ahma_http_bridge/tests/http_bridge_integration_test.rs"
if [[ -f "$HTTP_BRIDGE_TEST" ]] && ! rg -q 'spawn_server_guard_with_config' "$HTTP_BRIDGE_TEST"; then
    echo "FAIL VIOLATION: $HTTP_BRIDGE_TEST"
    echo "   Missing shared custom server startup helper usage"
    echo "   Use tests/common/server::spawn_server_guard_with_config(...)"
    echo ""
    VIOLATIONS=$((VIOLATIONS + 1))
fi

if [ $VIOLATIONS -eq 0 ]; then
    echo "OK No violations found"
    exit 0
else
    echo ""
    echo "FAIL Found $VIOLATIONS violation(s)"
    echo ""
    echo "Fix: Replace manual CARGO_TARGET_DIR logic with:"
    echo "  - ahma_mcp::test_utils::cli::get_binary_path(package, binary)"
    echo "  - ahma_mcp::test_utils::cli::build_binary_cached(package, binary)"
    exit 1
fi
