#!/usr/bin/env bash
# publish-winget.sh — Update winget manifests and submit to winget-pkgs.
#
# Prerequisites:
#   wingetcreate (https://aka.ms/wingetcreate) — install with:
#     winget install Microsoft.WingetCreate
#   GitHub CLI (gh) with write access to paulirotta/ahma
#
# Usage:
#   ./scripts/publish-winget.sh [VERSION]
#
# If VERSION is omitted, the version is read from the workspace Cargo.toml.
#
# The script:
#   1. Determines the package version.
#   2. Fetches the SHA256 for the Windows .zip from the GitHub release.
#   3. Instantiates the manifests under winget/manifests/ with the real values.
#   4. Validates them with wingetcreate validate.
#   5. Submits a PR to microsoft/winget-pkgs via wingetcreate submit.

set -euo pipefail

# ---------------------------------------------------------------------------
# 1. Determine version
# ---------------------------------------------------------------------------
if [[ "${1:-}" != "" ]]; then
    VERSION="$1"
else
    VERSION=$(grep -m1 '^version' Cargo.toml | sed 's/version = "\(.*\)"/\1/')
fi
echo "Publishing winget manifest for version: $VERSION"

PACKAGE_ID="paulirotta.Ahma"
RELEASE_TAG="latest"
ZIP_ASSET="ahma-release-windows-x86_64.zip"
DOWNLOAD_URL="https://github.com/paulirotta/ahma/releases/download/${RELEASE_TAG}/${ZIP_ASSET}"

# ---------------------------------------------------------------------------
# 2. Fetch SHA256 from the GitHub release's SHA256SUMS file
# ---------------------------------------------------------------------------
echo "Fetching SHA256 from GitHub release..."
SUMS_URL="https://github.com/paulirotta/ahma/releases/download/${RELEASE_TAG}/SHA256SUMS"
SHA256=$(curl -fsSL "$SUMS_URL" | grep "$ZIP_ASSET" | awk '{print $1}' | tr '[:lower:]' '[:upper:]')

if [[ -z "$SHA256" ]]; then
    echo "ERROR: Could not find SHA256 for $ZIP_ASSET in $SUMS_URL" >&2
    exit 1
fi
echo "SHA256: $SHA256"

# ---------------------------------------------------------------------------
# 3. Instantiate manifests into a versioned directory
# ---------------------------------------------------------------------------
MANIFEST_DIR="winget/manifests"
OUT_DIR="${MANIFEST_DIR}/${VERSION}"
mkdir -p "$OUT_DIR"

for f in "${MANIFEST_DIR}"/*.yaml; do
    basename_f="$(basename "$f")"
    # Update PackageVersion and SHA256 placeholders
    sed \
        -e "s/PackageVersion: .*/PackageVersion: ${VERSION}/" \
        -e "s/<SHA256_PLACEHOLDER>/${SHA256}/" \
        -e "s|/releases/download/latest/|/releases/download/latest/|g" \
        "$f" > "${OUT_DIR}/${basename_f}"
done
echo "Written versioned manifests to ${OUT_DIR}/"

# ---------------------------------------------------------------------------
# 4. Validate with wingetcreate
# ---------------------------------------------------------------------------
if command -v wingetcreate &>/dev/null; then
    echo "Validating manifests..."
    wingetcreate validate "$OUT_DIR"
else
    echo "wingetcreate not found — skipping local validation."
    echo "Install with: winget install Microsoft.WingetCreate"
fi

# ---------------------------------------------------------------------------
# 5. Submit PR to winget-pkgs (requires WINGET_TOKEN env var)
# ---------------------------------------------------------------------------
if [[ "${WINGET_TOKEN:-}" == "" ]]; then
    echo ""
    echo "WINGET_TOKEN not set — skipping automatic PR submission."
    echo "To submit manually:"
    echo "  wingetcreate submit --token <github-token> \\"
    echo "    --prtitle '[${PACKAGE_ID}] version ${VERSION}' \\"
    echo "    ${OUT_DIR}"
else
    echo "Submitting PR to microsoft/winget-pkgs..."
    wingetcreate submit \
        --token "$WINGET_TOKEN" \
        --prtitle "[${PACKAGE_ID}] version ${VERSION}" \
        "$OUT_DIR"
    echo "PR submitted successfully."
fi
