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

echo "=== Guardrail: SKILL.md self-containment (no relative file links) ==="
# Skills are installed standalone to ~/.agents/skills/ — external relative links break.
# All cross-references must use absolute GitHub URLs, not relative paths like ](docs/foo.md).
SKILL_LINK_VIOLATIONS=$(grep -rn '](docs/' skills .agents/skills 2>/dev/null | grep -v 'https://' || true)
if [[ -n "$SKILL_LINK_VIOLATIONS" ]]; then
  echo ""
  echo "FAIL SKILL.md files contain relative docs/ links that break when installed standalone:"
  echo "$SKILL_LINK_VIOLATIONS"
  echo ""
  echo "Replace relative paths with absolute GitHub URLs:"
  echo "  ](docs/foo.md)  ->  ](https://github.com/paulirotta/ahma/blob/main/docs/foo.md)"
  exit 1
fi
echo "OK No relative docs/ links in SKILL.md files"

echo "=== Guardrail: skill version consistency with Cargo.toml ==="
# AHMA_VERSION in install.sh and version: in skill files must match [workspace.package] version.
CARGO_VER=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')
INSTALL_VER=$(grep '^AHMA_VERSION=' scripts/install.sh | head -1 | sed 's/AHMA_VERSION="\(.*\)"/\1/')
AHMA_SKILL_VER=$(grep '^version:' skills/ahma/SKILL.md | head -1 | awk '{print $2}')
SIMPLIFY_SKILL_VER=$(grep '^version:' skills/ahma-simplify/SKILL.md | head -1 | awk '{print $2}')

SKILL_VER_FAIL=0
if [ "$INSTALL_VER" != "$CARGO_VER" ]; then
  echo "FAIL install.sh AHMA_VERSION=\"${INSTALL_VER}\" != Cargo.toml version \"${CARGO_VER}\""
  SKILL_VER_FAIL=1
fi
if [ "$AHMA_SKILL_VER" != "$CARGO_VER" ]; then
  echo "FAIL skills/ahma/SKILL.md version: ${AHMA_SKILL_VER} != Cargo.toml version ${CARGO_VER}"
  SKILL_VER_FAIL=1
fi
if [ "$SIMPLIFY_SKILL_VER" != "$CARGO_VER" ]; then
  echo "FAIL skills/ahma-simplify/SKILL.md version: ${SIMPLIFY_SKILL_VER} != Cargo.toml version ${CARGO_VER}"
  SKILL_VER_FAIL=1
fi
if [ "$SKILL_VER_FAIL" -ne 0 ]; then
  echo ""
  echo "  Bump AHMA_VERSION in scripts/install.sh and version: in skills/*/SKILL.md"
  echo "  to match [workspace.package] version = \"${CARGO_VER}\" in Cargo.toml."
  exit 1
fi
echo "OK Skill versions consistent (v${CARGO_VER})"

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
cargo test -p ahma_mcp --test tool_tests tool_execution_integration_test::test_cargo_check_dry_run -- --nocapture

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
