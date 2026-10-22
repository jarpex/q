#!/usr/bin/env bash
set -euo pipefail

SBOM="$1"
OUTPUT="$2"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

VEX_FLAG=""
if [ -f vex/q.vex.json ]; then
    VEX_FLAG="--vex vex/q.vex.json"
fi

trivy sbom "$SBOM" --scanners vuln,license --format sarif --output "$OUTPUT" --severity HIGH,CRITICAL --disable-telemetry --quiet $VEX_FLAG

# Repair artifact locations so GitHub Code Scanning accepts the SARIF.
if command -v python3 >/dev/null 2>&1; then
    python3 "${SCRIPT_DIR}/fix-trivy-sarif.py" "$OUTPUT"
fi