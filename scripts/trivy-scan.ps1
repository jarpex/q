[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Sbom,
    [Parameter(Mandatory = $true)]
    [string]$Output
)

$ErrorActionPreference = "Stop"
$arguments = @(
    "sbom",
    $Sbom,
    "--scanners",
    "vuln,license",
    "--format",
    "sarif",
    "--output",
    $Output,
    "--severity",
    "HIGH,CRITICAL",
    "--disable-telemetry",
    "--quiet"
)

if (Test-Path "vex/q.vex.json") {
    $arguments += @("--vex", "vex/q.vex.json")
}

& trivy @arguments
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$fixScript = Join-Path $scriptDir "fix-trivy-sarif.py"
if (Test-Path $fixScript) {
    & py $fixScript $Output
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }
}
