#requires -Version 5
# Install (or remove) the VibeDev agent profile into a local Open Design
# install — https://github.com/nexu-io/open-design.
#
# Usage:
#   .\install-profile.ps1            # install (default action)
#   .\install-profile.ps1 -Uninstall # remove the profile, leave Open Design otherwise untouched
#   .\install-profile.ps1 -Quiet     # no console output (for Inno post-install hook)
#
# Behaviour:
#   - Detects `%USERPROFILE%\.open-design\profiles\` (created by Open Design
#     on first run). If absent, exits 0 silently — we never create directories
#     for a product the user hasn't installed.
#   - Copies vibedev.ts next to the script into that directory.
#   - Idempotent: re-running overwrites; uninstall removes only our file.
#
# Pure ASCII intentionally so PowerShell 5.1 (the system shell on Win10/11
# absent pwsh) reads it without BOM/codepage trouble. Same convention as
# build-zh.ps1.

[CmdletBinding()]
param(
    [switch]$Uninstall,
    [switch]$Quiet
)

$ErrorActionPreference = 'Stop'

function Write-Info {
    param([string]$Message)
    if (-not $Quiet) { Write-Host "[od-integration] $Message" }
}

# Open Design's profile drop-in directory. See open-design's
# apps/daemon/src/runtimes/local-profiles.ts for the canonical lookup path.
$OdProfilesDir = Join-Path $env:USERPROFILE '.open-design\profiles'
$TargetPath    = Join-Path $OdProfilesDir 'vibedev.ts'
$SourcePath    = Join-Path $PSScriptRoot 'vibedev.ts'

if (-not (Test-Path $OdProfilesDir)) {
    Write-Info "Open Design not detected at $OdProfilesDir; nothing to do."
    exit 0
}

if ($Uninstall) {
    if (Test-Path $TargetPath) {
        Remove-Item -Path $TargetPath -Force
        Write-Info "Removed $TargetPath"
    } else {
        Write-Info "Nothing to remove at $TargetPath"
    }
    exit 0
}

if (-not (Test-Path $SourcePath)) {
    throw "vibedev.ts not found next to this script: $SourcePath"
}

Copy-Item -Path $SourcePath -Destination $TargetPath -Force
Write-Info "Installed VibeDev profile -> $TargetPath"
Write-Info "Restart Open Design to pick it up."
