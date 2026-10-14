[CmdletBinding()]
param(
    [string]$Output = "sbom/clippy.sarif"
)

$ErrorActionPreference = "Stop"
$outputDirectory = Split-Path -Parent $Output
if ($outputDirectory) {
    New-Item -ItemType Directory -Force -Path $outputDirectory | Out-Null
}

$tempFile = [System.IO.Path]::GetTempFileName()
try {
    Write-Host "[sast] Running clippy with strict rules (-D warnings)..."

    & cargo clippy --all-targets --all-features --message-format=json-render-diagnostics -- -D warnings 1> $tempFile
    $clippyExit = $LASTEXITCODE

    Write-Host "[sast] Converting to SARIF..."
    if (-not (Get-Command clippy-sarif -ErrorAction SilentlyContinue)) {
        throw "[sast] ERROR: clippy-sarif not installed"
    }

    $sarif = Get-Content -Raw $tempFile | & clippy-sarif
    if ($LASTEXITCODE -ne 0) {
        throw "clippy-sarif failed with exit code $LASTEXITCODE"
    }
    [System.IO.File]::WriteAllText($Output, ($sarif -join [Environment]::NewLine), [System.Text.UTF8Encoding]::new($false))

    if (Get-Command jq -ErrorAction SilentlyContinue) {
        $dedupFile = "$Output.tmp"
        & jq '.runs[0].results |= unique_by(.ruleId, .locations[0].physicalLocation.artifactLocation.uri, .locations[0].physicalLocation.region.startLine, .locations[0].physicalLocation.region.startColumn)' $Output | Set-Content -Encoding utf8 $dedupFile
        if ($LASTEXITCODE -eq 0) {
            Move-Item -Force $dedupFile $Output
        }
    }

    $findings = 0
    if (Get-Command jq -ErrorAction SilentlyContinue) {
        $findings = (& jq '[.runs[0].results[]] | length' $Output).Trim()
    }
    Write-Host "[sast] SARIF written to $Output"
    Write-Host "[sast] Clippy findings: $findings"

    if ($clippyExit -ne 0) {
        Write-Error "[sast] ERROR: clippy failed with exit code $clippyExit"
        Write-Error "[sast] Fix all warnings before proceeding."
        exit $clippyExit
    }

    Write-Host "[sast] Clippy gate passed"
}
finally {
    if (Test-Path $tempFile) {
        Remove-Item -Force $tempFile
    }
    if (Test-Path "$Output.tmp") {
        Remove-Item -Force "$Output.tmp"
    }
}
