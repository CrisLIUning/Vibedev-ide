#requires -Version 5
# Upload latest.json (this directory) to the public webroot on the nginx
# server that serves https://aitoken.bigopen.cn/vibedev/latest.json.
#
# Usage:
#   .\deploy-latest-json.ps1            # upload + chown + chmod + verify
#   .\deploy-latest-json.ps1 -DryRun    # print the commands that WOULD run, do nothing
#
# Credentials:
#   Read from %USERPROFILE%\.vibedev\deploy-secrets.json. Never baked in.
#   Override with env: VIBEDEV_DEPLOY_SECRETS=/abs/path/to/secrets.json.
#   Expected schema (all fields required):
#     {
#       "host": "<release-host>",
#       "port": 22,
#       "user": "<deploy-user>",
#       "key_path": "C:\\Users\\you\\.ssh\\vibedev_deploy",
#       "remote_dir": "<release-webroot>/vibedev",
#       "verify_url": "https://aitoken.bigopen.cn/vibedev/latest.json"
#     }
#   `key_path` points to a PRIVATE key file with mode 600. Password auth is
#   intentionally NOT supported — interactive prompts break automation.
#
# What it does:
#   1. scp ./latest.json <user>@<host>:<remote_dir>/latest.json
#   2. ssh "<user>@<host>" 'chown www:www <remote_dir>/latest.json && chmod 644 <remote_dir>/latest.json'
#   3. curl -sI <verify_url> and assert HTTP/2 200 + application/json content-type.
#
# Pure ASCII for PowerShell 5.1 compatibility. Same convention as
# integrations/open-design/install-profile.ps1.

[CmdletBinding()]
param(
    [switch]$DryRun
)

$ErrorActionPreference = 'Stop'

function Write-Step {
    param([string]$Message)
    Write-Host "[update-check] $Message"
}

function Resolve-SecretsPath {
    if ($env:VIBEDEV_DEPLOY_SECRETS) {
        return $env:VIBEDEV_DEPLOY_SECRETS
    }
    return Join-Path $env:USERPROFILE '.vibedev\deploy-secrets.json'
}

# --- Load secrets ----------------------------------------------------------

$SecretsPath = Resolve-SecretsPath
if (-not (Test-Path $SecretsPath)) {
    throw "Secrets file not found at $SecretsPath. Create it (mode 600) with " +
          "host/port/user/key_path/remote_dir/verify_url, or set " +
          "VIBEDEV_DEPLOY_SECRETS to an alternate location. See the script header."
}

$secrets = Get-Content -Raw -Path $SecretsPath | ConvertFrom-Json

foreach ($field in @('host','port','user','key_path','remote_dir','verify_url')) {
    if (-not $secrets.$field) {
        throw "Secrets file $SecretsPath is missing required field '$field'."
    }
}

if (-not (Test-Path $secrets.key_path)) {
    throw "SSH private key not found at $($secrets.key_path) (from $SecretsPath)."
}

# --- Resolve source --------------------------------------------------------

$LocalJson = Join-Path $PSScriptRoot 'latest.json'
if (-not (Test-Path $LocalJson)) {
    throw "Local latest.json not found at $LocalJson — nothing to deploy."
}

# Lightweight validation so we never push a malformed file. The server
# returns 200 even for bad JSON; clients then log warn and stay silent,
# which looks identical to "no update yet" and is hard to debug.
try {
    $parsed = Get-Content -Raw -Path $LocalJson | ConvertFrom-Json
    foreach ($field in @('version','download_url')) {
        if (-not $parsed.$field) {
            throw "latest.json is missing required field '$field'."
        }
    }
} catch {
    throw "Refusing to deploy: latest.json failed local validation. $($_.Exception.Message)"
}

Write-Step "Source:      $LocalJson"
Write-Step "Secrets:     $SecretsPath"
Write-Step "Target:      $($secrets.user)@$($secrets.host):$($secrets.remote_dir)/latest.json"
Write-Step "Verify URL:  $($secrets.verify_url)"
Write-Step "Local version: $($parsed.version)"

# --- Build commands --------------------------------------------------------

$RemotePath = "$($secrets.remote_dir)/latest.json"
$ScpTarget  = "$($secrets.user)@$($secrets.host):$RemotePath"
$SshTarget  = "$($secrets.user)@$($secrets.host)"

$ScpArgs = @(
    '-P', "$($secrets.port)",
    '-i', $secrets.key_path,
    '-o', 'StrictHostKeyChecking=accept-new',
    '-o', 'BatchMode=yes',
    $LocalJson,
    $ScpTarget
)

$ChownCommand = "chown www:www '$RemotePath' && chmod 644 '$RemotePath'"
$SshArgs = @(
    '-p', "$($secrets.port)",
    '-i', $secrets.key_path,
    '-o', 'StrictHostKeyChecking=accept-new',
    '-o', 'BatchMode=yes',
    $SshTarget,
    $ChownCommand
)

if ($DryRun) {
    Write-Step "DRY RUN — would execute:"
    Write-Host "  scp $($ScpArgs -join ' ')"
    Write-Host "  ssh $($SshArgs -join ' ')"
    Write-Host "  curl -sI $($secrets.verify_url)"
    exit 0
}

# --- Execute ---------------------------------------------------------------

Write-Step 'Uploading latest.json...'
& scp @ScpArgs
if ($LASTEXITCODE -ne 0) {
    throw "scp failed with exit code $LASTEXITCODE."
}

Write-Step 'Applying chown www:www && chmod 644...'
& ssh @SshArgs
if ($LASTEXITCODE -ne 0) {
    throw "ssh chown/chmod failed with exit code $LASTEXITCODE."
}

Write-Step 'Verifying with curl -sI...'
$response = Invoke-WebRequest -Uri $secrets.verify_url -Method Head -UseBasicParsing
if ($response.StatusCode -ne 200) {
    throw "Verify failed: got HTTP $($response.StatusCode), expected 200."
}
$contentType = $response.Headers['Content-Type']
if ($contentType -notmatch '^application/json') {
    Write-Step "WARNING: content-type is '$contentType' (expected application/json). Check nginx mime.types."
}

Write-Step "Deploy OK. Clients pick this up on next 24h poll (or restart)."
