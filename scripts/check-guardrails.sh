#!/bin/bash
# Unified guardrail checks for local commit/push workflows.
#
# Usage:
#   ./scripts/check-guardrails.sh --phase commit
#   ./scripts/check-guardrails.sh --phase push
#   ./scripts/check-guardrails.sh --phase commit --allow-dirty
#
# Recommended:
# - Before commit: run with --phase commit
# - Before push:   run with --phase push (requires clean working tree)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_ROOT"

PHASE="push"
ALLOW_DIRTY=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --phase)
      PHASE="${2:-}"
      shift 2
      ;;
    --allow-dirty)
      ALLOW_DIRTY=1
      shift
      ;;
    *)
      echo "Unknown argument: $1"
      exit 2
      ;;
  esac
done

if [[ "$PHASE" != "commit" && "$PHASE" != "push" ]]; then
  echo "Invalid phase: '$PHASE' (expected 'commit' or 'push')"
  exit 2
fi

echo "Running guardrail checks (phase: $PHASE)..."

if [[ "$ALLOW_DIRTY" -ne 1 ]]; then
  DIRTY_STATUS="$(git status --porcelain)"
  if [[ -n "$DIRTY_STATUS" ]]; then
    echo "FAIL Working tree is not clean."
    echo ""
    echo "$DIRTY_STATUS"
    echo ""
    echo "Commit/stash/remove local changes (including untracked files), or rerun with --allow-dirty."
    exit 1
  fi
  echo "OK Working tree is clean"
else
  echo "WARNING️  --allow-dirty enabled (clean tree check skipped)"
fi

echo "=== Guardrail: crate root preflight (src/lib.rs or src/main.rs) ==="
missing=0
while IFS= read -r manifest; do
  crate_dir="$(dirname "$manifest")"

  # Workspace root can have Cargo.toml without a package section.
  if ! grep -q "^\[package\]" "$manifest"; then
    continue
  fi

  if [[ ! -f "$crate_dir/src/lib.rs" && ! -f "$crate_dir/src/main.rs" ]]; then
    echo "Missing crate root in $crate_dir (expected src/lib.rs or src/main.rs)"
    missing=1
  fi
done < <(find . -name Cargo.toml -not -path "./target/*")

if [[ "$missing" -ne 0 ]]; then
  echo ""
  echo "FAIL Crate root preflight failed."
  echo "Likely cause: required files exist locally but are untracked or misnamed."
  exit 1
fi
echo "OK Crate root preflight passed"

echo "=== Guardrail: lint recurring test patterns ==="
./scripts/lint_test_paths.sh

echo "=== Guardrail: workspace cargo check ==="
cargo check --workspace --locked

echo "=== Guardrail: cargo smoke test scope (ahma-mcp package) ==="
cargo test -p ahma-mcp --test tool_tests tool_execution_integration_test::test_cargo_check_dry_run -- --nocapture

echo "=== Guardrail: nextest diagnostics config ==="
if ! grep -q 'success-output = "immediate"' .config/nextest.toml; then
  echo "FAIL .config/nextest.toml missing success-output = \"immediate\""
  exit 1
fi
if ! grep -q 'failure-output = "immediate"' .config/nextest.toml; then
  echo "FAIL .config/nextest.toml missing failure-output = \"immediate\""
  exit 1
fi
echo "OK Nextest diagnostics config looks good"

echo ""
echo "OK All guardrails passed for phase: $PHASE"
