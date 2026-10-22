#!/usr/bin/env bash
set -euo pipefail

SBOM="$1"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

VERSION=$(grep -E '^version\s*=' "$SCRIPT_DIR/../Cargo.toml" | head -n 1 | sed 's/.*"\(.*\)".*/\1/')

if [ -z "$VERSION" ]; then
    echo "ERROR: Could not extract version from Cargo.toml" >&2
    exit 1
fi

# Drop components that are not in the actual build graph (phantom optional
# dependencies emitted by cargo-cyclonedx) before normalising.
if command -v python3 >/dev/null 2>&1; then
    python3 "${SCRIPT_DIR}/prune-sbom.py" "$SBOM" || true
fi

jq --arg version "$VERSION" -f "${SCRIPT_DIR}/clean-sbom.jq" "$SBOM" > "${SBOM}.tmp" && mv "${SBOM}.tmp" "$SBOM"