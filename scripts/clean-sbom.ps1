[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Sbom
)

$ErrorActionPreference = "Stop"
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$metadata = cargo metadata --no-deps --format-version 1 | ConvertFrom-Json
$version = $metadata.packages[0].version

if ([string]::IsNullOrWhiteSpace($version)) {
    throw "ERROR: Could not extract version from Cargo.toml"
}

$temp = "$Sbom.tmp"
try {
    $cleaned = & jq --arg version $version -f (Join-Path $scriptDir "clean-sbom.jq") $Sbom
    if ($LASTEXITCODE -ne 0) {
        throw "jq failed with exit code $LASTEXITCODE"
    }
    [System.IO.File]::WriteAllText($temp, ($cleaned -join [Environment]::NewLine), [System.Text.UTF8Encoding]::new($false))
    Move-Item -Force $temp $Sbom
}
finally {
    if (Test-Path $temp) {
        Remove-Item -Force $temp
    }
}
