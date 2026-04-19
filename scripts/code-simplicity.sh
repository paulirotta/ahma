#!/bin/bash

# Code Simplicity Metrics wrapper script
#
# This script runs 'ahma-mcp simplify' to analyze code simplicity metrics
# for any directory (not just the ahma-mcp repository).
#
# Usage:
#   ./scripts/code-simplicity.sh [TARGET_DIR] [ADDITIONAL_ARGS...]
#
# Arguments:
#   TARGET_DIR        - Directory to analyze (optional, defaults to current directory)
#   ADDITIONAL_ARGS   - Additional arguments passed to ahma-mcp simplify
#
# Examples:
#   ./scripts/code-simplicity.sh                    # Analyze current directory
#   ./scripts/code-simplicity.sh /path/to/project   # Analyze specific directory
#   ./scripts/code-simplicity.sh . --limit 50       # Analyze current dir, show top 50 issues
#
# Note: This script can be called from any directory. It will find the ahma
# repository root (where Cargo.toml is located) to run cargo, but the analysis
# target can be any directory on your system.

# Find the ahma repository root based on script location
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
AHMA_REPO_ROOT="$( cd "$SCRIPT_DIR/.." && pwd )"

# Verify we found the correct repository root
if [ ! -f "$AHMA_REPO_ROOT/Cargo.toml" ]; then
    echo "Error: Could not find ahma repository root (Cargo.toml not found)" >&2
    echo "Expected location: $AHMA_REPO_ROOT/Cargo.toml" >&2
    exit 1
fi

# Capture the original working directory for output files
ORIGINAL_CWD="$PWD"

# Target directory: first argument or current working directory
TARGET_DIR="${1:-$PWD}"

# Validate target directory exists
if [ ! -d "$TARGET_DIR" ]; then
    echo "Error: Target directory does not exist: $TARGET_DIR" >&2
    exit 1
fi

# Canonicalize target directory to absolute path
TARGET_DIR="$(cd "$TARGET_DIR" && pwd)" || {
    echo "Error: Could not access target directory: $TARGET_DIR" >&2
    exit 1
}

# Shift to pass remaining arguments to ahma-mcp simplify
shift || true

echo "Analyzing: $TARGET_DIR"
echo "Running from ahma repo: $AHMA_REPO_ROOT"
echo ""

# AI-first default focus: score maintainability of production code paths first.
# Set CODE_SIMPLICITY_INCLUDE_NON_PROD=1 to include tests/examples/benches.
EXTRA_EXCLUDES=()
if [ "${CODE_SIMPLICITY_INCLUDE_NON_PROD:-0}" != "1" ]; then
    EXTRA_EXCLUDES=(
        --exclude "**/examples/**"
        --exclude "**/tests/**"
        --exclude "**/benches/**"
    )
    echo "Focus mode: excluding non-production paths (examples/, tests/, benches/)."
    echo "Set CODE_SIMPLICITY_INCLUDE_NON_PROD=1 to include them."
    echo ""
fi

# Change to ahma repo root to run cargo, but analyze the target directory
cd "$AHMA_REPO_ROOT" || exit 1

# Run the code simplicity aggregator on the target directory
# Use --output-path to write files to the original working directory
cargo run -p ahma_mcp -- simplify "$TARGET_DIR" --html --open --output-path "$ORIGINAL_CWD" "${EXTRA_EXCLUDES[@]}" "$@"
