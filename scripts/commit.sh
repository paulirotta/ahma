#!/bin/bash
# Test and commit current branch changes to git.
set -euo pipefail

# Resolve script directory and project root (assumes script lives in <project>/scripts/)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
TOOLS_DIR="$PROJECT_ROOT/.ahma/tools"

# Change to project root regardless of current working directory
cd "$PROJECT_ROOT"

echo "=== Running cargo check ==="
cargo check
if [ $? -ne 0 ]; then
    echo "cargo check failed"
    exit 1
fi

echo "=== Running cargo fmt ==="
cargo fmt
if [ $? -ne 0 ]; then
    echo "cargo fmt failed"
    exit 1
fi

echo "=== Running cargo clippy --fix --allow-dirty ==="
cargo clippy --fix --allow-dirty -- -D warnings
if [ $? -ne 0 ]; then
    echo "cargo clippy failed"
    exit 1
fi

echo "=== Running cargo clippy --fix --allow-dirty --tests ==="
cargo clippy --fix --allow-dirty --tests -- -D warnings
if [ $? -ne 0 ]; then
    echo "cargo clippy failed on tests"
    exit 1
fi

echo "=== Running cargo nextest run ==="
cargo nextest run
if [ $? -ne 0 ]; then
    echo "cargo nextest run failed"
    exit 1
fi

echo "=== Generating JSON tool schema ==="
cargo run --bin generate-tool-schema
if [ $? -ne 0 ]; then
    echo "generate-tool-schema failed"
    exit 1
fi

echo "=== Validating tool configurations ==="
cargo run --bin ahma-mcp -- --validate "$TOOLS_DIR"
if [ $? -ne 0 ]; then
    echo "ahma-mcp --validate failed"
    exit 1
fi

echo "=== Building release ==="
cargo build --release
if [ $? -ne 0 ]; then
    echo "cargo build --release failed"
    exit 1
fi

echo "=== All checks passed ==="

# Check if there are changes to commit
if git diff --quiet && git diff --staged --quiet; then
    echo "No changes to commit."
    exit 0
fi

echo "=== Staging all changes ==="
git add -A

echo "=== Generating commit message ==="
# Generate a commit message based on the staged changes
CHANGED_FILES=$(git diff --staged --name-only | head -20)
STATS=$(git diff --staged --stat | tail -1)

# Create a summary of changes
COMMIT_MSG="chore: automated commit

Changed files:
$CHANGED_FILES

$STATS"

echo "Proposed commit message:"
echo "------------------------"
echo "$COMMIT_MSG"
echo "------------------------"

read -p "Use this message? (y/n/e for edit): " CHOICE
case "$CHOICE" in
    y|Y)
        git commit -m "$COMMIT_MSG"
        ;;
    e|E)
        git commit -e -m "$COMMIT_MSG"
        ;;
    *)
        echo "Commit aborted."
        exit 1
        ;;
esac

echo "=== Commit successful ==="
