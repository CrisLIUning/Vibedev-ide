[CmdletBinding()]
Param(
    [Parameter()][Alias('i')][switch]$Install,
    [Parameter()][Alias('h')][switch]$Help,
    [Parameter()][Alias('a')][string]$Architecture,
    [Parameter()][string]$Name
)

. "$PSScriptRoot/lib/workspace.ps1"

# https://stackoverflow.com/questions/57949031/powershell-script-stops-if-program-fails-like-bash-set-o-errexit
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true

$buildSuccess = $false

$OSArchitecture = switch ([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture) {
    "X64" { "x86_64" }
    "Arm64" { "aarch64" }
    default { throw "Unsupported architecture" }
}

$Architecture = if ($Architecture) {
    $Architecture
} else {
    $OSArchitecture
}

# VIBEDEV: default profile is `release` (lto="thin"), NOT release-fast. The
# round-1 / round-2 attempt at making release-fast the default backfired —
# switching profiles invalidates the entire cargo cache (target/<triple>/
# release/ vs target/<triple>/release-fast/ are separate trees), so the first
# release-fast build had to recompile ~1200 crates from scratch and finished
# ~14 min SLOWER than the prior LTO build that reused its cache. LTO's ~25
# min of single-threaded codegen is real, but the cache-reuse savings dwarf
# it on subsequent builds. release-fast also uses debug="full" which inflates
# PDBs past PowerShell's 2GB Compress-Archive limit (see the skipped
# ZipZedAndItsFriendsDebug call).
#
# Set $env:VIBEDEV_RELEASE_FAST=1 only when you've already wiped target/ or
# done a massive dependency-changing edit, and want the no-LTO fast path
# explicitly. Day-to-day, leave it unset — LTO + cache reuse wins.
$cargoProfile = if ($env:VIBEDEV_RELEASE_FAST) { 'release-fast' } else { 'release' }
# VIBEDEV: `[string[]]` cast is mandatory here. A bare `@('--release')` is a
# 1-element string array, and `cargo build @cargoProfileArgs ...` splats it —
# but PowerShell's @-splat of a 1-element array containing a single string
# *unwraps* the string into a char[]. cargo then sees ten args
# `-`,`-`,`r`,`e`,`l`,`e`,`a`,`s`,`e` instead of one `--release` and bails
# with "unexpected argument '-'". The 2-element profile-args path
# (release-fast: `@('--profile', 'release-fast')`) doesn't hit the quirk —
# which is why the round-5 LTO build was the first to surface it.
[string[]]$cargoProfileArgs = if ($env:VIBEDEV_RELEASE_FAST) { @('--profile', 'release-fast') } else { @('--release') }
$CargoOutDir = "./target/$Architecture-pc-windows-msvc/$cargoProfile"
Write-Host "VIBEDEV: cargo profile = $cargoProfile (set VIBEDEV_RELEASE_FAST=1 to bypass LTO)"

function Get-VSArch {
    param(
        [string]$Arch
    )

    switch ($Arch) {
        "x86_64" { "amd64" }
        "aarch64" { "arm64" }
    }
}

Push-Location
# VIBEDEV: CI runners have the Community edition; local dev machines often have
# BuildTools. Use whichever Launch-VsDevShell.ps1 exists instead of hard-coding
# Community (which made the bundle fail on a BuildTools-only box).
$vsDevShell = @(
    "C:\Program Files\Microsoft Visual Studio\2022\Community\Common7\Tools\Launch-VsDevShell.ps1",
    "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\Common7\Tools\Launch-VsDevShell.ps1",
    "C:\Program Files\Microsoft Visual Studio\2022\BuildTools\Common7\Tools\Launch-VsDevShell.ps1"
) | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $vsDevShell) { throw "Launch-VsDevShell.ps1 not found (looked for Community + BuildTools)" }
& $vsDevShell -Arch (Get-VSArch -Arch $Architecture) -HostArch (Get-VSArch -Arch $OSArchitecture)
Pop-Location

$target = "$Architecture-pc-windows-msvc"

if ($Help) {
    Write-Output "Usage: test.ps1 [-Install] [-Help]"
    Write-Output "Build the installer for Windows.\n"
    Write-Output "Options:"
    Write-Output "  -Architecture, -a Which architecture to build (x86_64 or aarch64)"
    Write-Output "  -Install, -i      Run the installer after building."
    Write-Output "  -Help, -h         Show this help message."
    exit 0
}

Push-Location -Path crates/zed
$channel = Get-Content "RELEASE_CHANNEL"
$env:ZED_RELEASE_CHANNEL = $channel
$env:RELEASE_CHANNEL = $channel
Pop-Location

function CheckEnvironmentVariables {
    if(-not $env:CI) {
        return
    }

    $requiredVars = @(
        'ZED_WORKSPACE', 'RELEASE_VERSION', 'ZED_RELEASE_CHANNEL',
        'AZURE_TENANT_ID', 'AZURE_CLIENT_ID', 'AZURE_CLIENT_SECRET',
        'ACCOUNT_NAME', 'CERT_PROFILE_NAME', 'ENDPOINT',
        'FILE_DIGEST', 'TIMESTAMP_DIGEST', 'TIMESTAMP_SERVER'
    )

    foreach ($var in $requiredVars) {
        if (-not (Test-Path "env:$var")) {
            Write-Error "$var is not set"
            exit 1
        }
    }
}

function PrepareForBundle {
    if (Test-Path "$innoDir") {
        Remove-Item -Path "$innoDir" -Recurse -Force
    }
    New-Item -Path "$innoDir" -ItemType Directory -Force
    Copy-Item -Path "$env:ZED_WORKSPACE\crates\zed\resources\windows\*" -Destination "$innoDir" -Recurse -Force
    New-Item -Path "$innoDir\make_appx" -ItemType Directory -Force
    New-Item -Path "$innoDir\appx" -ItemType Directory -Force
    New-Item -Path "$innoDir\bin" -ItemType Directory -Force
    New-Item -Path "$innoDir\tools" -ItemType Directory -Force

    rustup target add $target
}

function GenerateLicenses {
    # VIBEDEV: upstream generate-licenses.ps1 runs `cargo install cargo-about` +
    # `cargo about generate`, which cannot resolve through this machine's local
    # crates mirror (it returns HTTP 500 for cargo-about's transitive deps, so the
    # install fails with "no such command: about"). For our internal/dev installer
    # we emit licenses.md from the bundled THEME + ICON license text (always present,
    # no network) and a placeholder for the per-crate CODE licenses. The "Open Source
    # License Attribution" view still renders; full per-crate attribution is produced
    # by upstream CI builds where cargo-about can install normally.
    $outputFile = "$env:ZED_WORKSPACE\assets\licenses.md"
    New-Item -Path $outputFile -ItemType File -Value "" -Force | Out-Null
    @(
        "# ###### THEME LICENSES ######"
        Get-Content "$env:ZED_WORKSPACE\assets\themes\LICENSES"
        "# ###### ICON LICENSES ######"
        Get-Content "$env:ZED_WORKSPACE\assets\icons\LICENSES"
        "# ###### CODE LICENSES ######"
        "Full per-crate (Rust dependency) license attribution is generated by CI via cargo-about."
    ) | Add-Content -Path $outputFile
    Write-Host "VIBEDEV: generated licenses.md (theme + icon licenses; crate licenses skipped on this host)"
}

function BuildZedAndItsFriends {
    Write-Output "Building VibeDev and its friends, for channel: $channel"
    # VIBEDEV: package is still "zed" but [[bin]] is "vibedev" (crates/zed/Cargo.toml),
    # so `cargo build --package zed` emits vibedev.exe. Stage it as VibeDev.exe to
    # match $appExeName / the .iss Source line / cli exe-detection (../VibeDev.exe).
    cargo build @cargoProfileArgs --package zed --package cli --package auto_update_helper --target $target
    Copy-Item -Path ".\$CargoOutDir\vibedev.exe" -Destination "$innoDir\VibeDev.exe" -Force
    Copy-Item -Path ".\$CargoOutDir\cli.exe" -Destination "$innoDir\cli.exe" -Force
    Copy-Item -Path ".\$CargoOutDir\auto_update_helper.exe" -Destination "$innoDir\auto_update_helper.exe" -Force
    # Build explorer_command_injector.dll
    switch ($channel) {
        "stable" {
            cargo build @cargoProfileArgs --features stable --no-default-features --package explorer_command_injector --target $target
        }
        "preview" {
            cargo build @cargoProfileArgs --features preview --no-default-features --package explorer_command_injector --target $target
        }
        default {
            cargo build @cargoProfileArgs --package explorer_command_injector --target $target
        }
    }
    Copy-Item -Path ".\$CargoOutDir\explorer_command_injector.dll" -Destination "$innoDir\zed_explorer_command_injector.dll" -Force
}

function BuildRemoteServer {
    Write-Output "Building remote_server for $target"
    cargo build @cargoProfileArgs --package remote_server --target $target

    # Create zipped remote server binary
    $remoteServerSrc = (Resolve-Path ".\$CargoOutDir\remote_server.exe").Path

    if ($env:CI) {
        Write-Output "Code signing remote_server.exe"
        & "$innoDir\sign.ps1" $remoteServerSrc
    }

    # VIBEDEV: remote_server is NOT bundled next to VibeDev.exe anymore. It is
    # DOWNLOADED on SSH connect (and reused by future app auto-update) from
    # VibeDev's own server — see crates/remote ensure_server_binary +
    # crates/auto_update get_release_asset (VIBEDEV_RELEASES_BASE). We still
    # build + compress the zip artifact below; that is the asset the server
    # hosts and the download path fetches.
    $remoteServerDst = "$env:ZED_WORKSPACE\target\vibedev-remote-server-windows-$Architecture.zip"
    Write-Output "Compressing remote_server to $remoteServerDst"
    Compress-Archive -Path $remoteServerSrc -DestinationPath $remoteServerDst -Force

    Write-Output "Remote server compressed successfully"
}

function ZipZedAndItsFriendsDebug {
    $items = @(
        ".\$CargoOutDir\vibedev.pdb",
        ".\$CargoOutDir\cli.pdb",
        ".\$CargoOutDir\auto_update_helper.pdb",
        ".\$CargoOutDir\explorer_command_injector.pdb",
        ".\$CargoOutDir\remote_server.pdb"
    )

    Compress-Archive -Path $items -DestinationPath ".\$CargoOutDir\zed-$env:RELEASE_VERSION-$env:ZED_RELEASE_CHANNEL.dbg.zip" -Force
}


function UploadToSentry {
    if (-not (Get-Command "sentry-cli" -ErrorAction SilentlyContinue)) {
        Write-Output "sentry-cli not found. skipping sentry upload."
        Write-Output "install with: 'winget install -e --id=Sentry.sentry-cli'"
        return
    }
    if (-not (Test-Path "env:SENTRY_AUTH_TOKEN")) {
        Write-Output "missing SENTRY_AUTH_TOKEN. skipping sentry upload."
        return
    }
    Write-Output "Uploading zed debug symbols to sentry..."
    for ($i = 1; $i -le 3; $i++) {
        try {
            sentry-cli debug-files upload --include-sources --wait -p zed -o zed-dev $CargoOutDir
            break
        }
        catch {
            Write-Output "Sentry upload attempt $i failed: $_"
            if ($i -eq 3) {
                Write-Output "All sentry upload attempts failed"
                throw
            }
            Start-Sleep -Seconds 2
        }
    }
}

function MakeAppx {
    switch ($channel) {
        "stable" {
            $manifestFile = "$env:ZED_WORKSPACE\crates\explorer_command_injector\AppxManifest.xml"
        }
        "preview" {
            $manifestFile = "$env:ZED_WORKSPACE\crates\explorer_command_injector\AppxManifest-Preview.xml"
        }
        default {
            $manifestFile = "$env:ZED_WORKSPACE\crates\explorer_command_injector\AppxManifest-Nightly.xml"
        }
    }
    Copy-Item -Path "$manifestFile" -Destination "$innoDir\make_appx\AppxManifest.xml"
    # Add makeAppx.exe to Path
    $sdk = "C:\Program Files (x86)\Windows Kits\10\bin\10.0.26100.0\x64"
    $env:Path += ';' + $sdk
    makeAppx.exe pack /d "$innoDir\make_appx" /p "$innoDir\zed_explorer_command_injector.appx" /nv
}

function SignZedAndItsFriends {
    if (-not $env:CI) {
        return
    }

    $files = "$innoDir\VibeDev.exe,$innoDir\cli.exe,$innoDir\auto_update_helper.exe,$innoDir\zed_explorer_command_injector.dll,$innoDir\zed_explorer_command_injector.appx"
    & "$innoDir\sign.ps1" $files
}

function DownloadAMDGpuServices {
    # If you update the AGS SDK version, please also update the version in `crates/gpui/src/platform/windows/directx_renderer.rs`
    $url = "https://codeload.github.com/GPUOpen-LibrariesAndSDKs/AGS_SDK/zip/refs/tags/v6.3.0"
    $zipPath = ".\AGS_SDK_v6.3.0.zip"
    # Download the AGS SDK zip file
    Invoke-WebRequest -Uri $url -OutFile $zipPath
    # Extract the AGS SDK zip file
    Expand-Archive -Path $zipPath -DestinationPath "." -Force
}

function DownloadConpty {
    $url = "https://github.com/microsoft/terminal/releases/download/v1.23.13503.0/Microsoft.Windows.Console.ConPTY.1.23.251216003.nupkg"
    $zipPath = ".\Microsoft.Windows.Console.ConPTY.1.23.251216003.nupkg"
    Invoke-WebRequest -Uri $url -OutFile $zipPath
    Expand-Archive -Path $zipPath -DestinationPath ".\conpty" -Force
}

function BuildAgent {
    # VIBEDEV: bundle Bun runtime + claude-code-best ACP agent dist into the
    # installer so the ACP agent works without the user pre-installing Bun.
    # We rebuild dist/ from source (claude-code-best is a sibling repo under
    # $env:ZED_WORKSPACE\..\claude-code-best) and ship the entire dist/ tree
    # plus bun.exe to <install_dir>/agent/. settings.rs expands ${ZED_BIN_DIR}
    # in default.json so default agent_servers.VibeDev points at the bundled
    # runtime without hardcoded absolute paths.
    $agentSrc = (Resolve-Path "$env:ZED_WORKSPACE\..\claude-code-best" -ErrorAction Stop).Path
    Write-Output "VIBEDEV: building agent dist at $agentSrc"
    Push-Location $agentSrc
    try {
        & bun install --frozen-lockfile
        if ($LASTEXITCODE -ne 0) { throw "bun install failed (exit $LASTEXITCODE)" }
        & bun run build
        if ($LASTEXITCODE -ne 0) { throw "bun run build failed (exit $LASTEXITCODE)" }
    } finally {
        Pop-Location
    }

    $agentDst = Join-Path $innoDir 'agent'
    if (Test-Path $agentDst) { Remove-Item -Path $agentDst -Recurse -Force }
    New-Item -ItemType Directory -Path $agentDst -Force | Out-Null
    # VIBEDEV (single-binary): the compiled agent is self-contained (embeds the Bun
    # runtime), so dist/* already contains vibedev-agent.exe + VERSION + vendor/.
    # No separate bun.exe to ship anymore.
    Copy-Item -Path (Join-Path $agentSrc 'dist\*') -Destination $agentDst -Recurse -Force
    Write-Output "VIBEDEV: agent bundled to $agentDst (single-binary dist/)"
}

function CollectFiles {
    Move-Item -Path "$innoDir\zed_explorer_command_injector.appx" -Destination "$innoDir\appx\zed_explorer_command_injector.appx" -Force
    Move-Item -Path "$innoDir\zed_explorer_command_injector.dll" -Destination "$innoDir\appx\zed_explorer_command_injector.dll" -Force
    # VIBEDEV: terminal command is `vibedev` (was `zed`). zed.sh invokes vibedev.exe.
    Move-Item -Path "$innoDir\cli.exe" -Destination "$innoDir\bin\vibedev.exe" -Force
    Move-Item -Path "$innoDir\zed.sh" -Destination "$innoDir\bin\vibedev" -Force
    Move-Item -Path "$innoDir\auto_update_helper.exe" -Destination "$innoDir\tools\auto_update_helper.exe" -Force
    if($Architecture -eq "aarch64") {
        New-Item -Type Directory -Path "$innoDir\arm64" -Force
        Move-Item -Path ".\conpty\build\native\runtimes\arm64\OpenConsole.exe" -Destination "$innoDir\arm64\OpenConsole.exe" -Force
        Move-Item -Path ".\conpty\runtimes\win-arm64\native\conpty.dll" -Destination "$innoDir\conpty.dll" -Force
    }
    else {
        New-Item -Type Directory -Path "$innoDir\x64" -Force
        New-Item -Type Directory -Path "$innoDir\arm64" -Force
        Move-Item -Path ".\AGS_SDK-6.3.0\ags_lib\lib\amd_ags_x64.dll" -Destination "$innoDir\amd_ags_x64.dll" -Force
        Move-Item -Path ".\conpty\build\native\runtimes\x64\OpenConsole.exe" -Destination "$innoDir\x64\OpenConsole.exe" -Force
        Move-Item -Path ".\conpty\build\native\runtimes\arm64\OpenConsole.exe" -Destination "$innoDir\arm64\OpenConsole.exe" -Force
        Move-Item -Path ".\conpty\runtimes\win-x64\native\conpty.dll" -Destination "$innoDir\conpty.dll" -Force
    }
}

function BuildInstaller {
    $issFilePath = "$innoDir\zed.iss"
    switch ($channel) {
        "stable" {
            # VIBEDEV: fresh AppId GUID (NOT Zed's) so Windows treats VibeDev as its
            # own product in Add/Remove Programs, not an upgrade of a real Zed install.
            $appId = "{{B4610BD5-C3B1-4A83-8F4B-67C57AA9260F}"
            $appIconName = "app-icon"
            $appName = "VibeDev"
            $appDisplayName = "VibeDev"
            $appSetupName = "VibeDev-$Architecture"
            # AppMutex MUST equal "{app_identifier()}-Instance-Mutex" from
            # crates/release_channel/src/lib.rs (consumed by windows_only_instance.rs).
            $appMutex = "VibeDev-Stable-Instance-Mutex"
            $appExeName = "VibeDev"
            $regValueName = "VibeDev"
            # MUST match ReleaseChannel::app_id() (release_channel) for taskbar grouping.
            $appUserId = "ai.vibedev.VibeDev"
            $appShellNameShort = "&VibeDev"
            # VIBEDEV-TODO: appx identity still Zed (Win11 explorer context menu).
            # Rebrand needs new publisher + regenerated family hash; deferred.
            $appAppxFullName = "ZedIndustries.Zed_1.0.0.0_neutral__japxn1gcva8rg"
        }
        "preview" {
            # VIBEDEV: fresh AppId GUID (see stable block rationale).
            $appId = "{{7BD78B4F-8805-414A-BDFE-2EC5FB81F905}"
            $appIconName = "app-icon-preview"
            $appName = "VibeDev Preview"
            $appDisplayName = "VibeDev Preview"
            $appSetupName = "VibeDev-$Architecture"
            # AppMutex MUST equal "{app_identifier()}-Instance-Mutex" (release_channel).
            $appMutex = "VibeDev-Preview-Instance-Mutex"
            $appExeName = "VibeDev"
            $regValueName = "VibeDevPreview"
            $appUserId = "ai.vibedev.VibeDev-Preview"
            $appShellNameShort = "&VibeDev Preview"
            # VIBEDEV-TODO: appx identity still Zed (deferred, see stable block).
            $appAppxFullName = "ZedIndustries.Zed.Preview_1.0.0.0_neutral__japxn1gcva8rg"
        }
        "nightly" {
            # VIBEDEV: fresh AppId GUID (see stable block rationale).
            $appId = "{{39657544-62CA-4B51-A634-03DE3EF4708B}"
            $appIconName = "app-icon-nightly"
            $appName = "VibeDev Nightly"
            $appDisplayName = "VibeDev Nightly"
            $appSetupName = "VibeDev-$Architecture"
            # AppMutex MUST equal "{app_identifier()}-Instance-Mutex" (release_channel).
            $appMutex = "VibeDev-Nightly-Instance-Mutex"
            $appExeName = "VibeDev"
            $regValueName = "VibeDevNightly"
            $appUserId = "ai.vibedev.VibeDev-Nightly"
            $appShellNameShort = "&VibeDev Nightly"
            # VIBEDEV-TODO: appx identity still Zed (deferred, see stable block).
            $appAppxFullName = "ZedIndustries.Zed.Nightly_1.0.0.0_neutral__japxn1gcva8rg"
        }
        "dev" {
            # VIBEDEV: fresh AppId GUID (see stable block rationale).
            $appId = "{{8A8B6E82-CDC8-49E0-A3A6-E617FA84DD55}"
            $appIconName = "app-icon-dev"
            # VIBEDEV: user-facing display names drop the "Dev" suffix — we ship
            # only one channel to end users, calling it "VibeDev Dev" in Start
            # Menu / Add-Remove Programs / installer wizard / install dir is
            # confusing. Technical identifiers ($appMutex / $regValueName /
            # $appUserId / $appId) keep their "Dev" suffix so this channel
            # remains distinct from the stable-channel AppId/mutex namespaces
            # in case we ever ship both side-by-side. The Rust app_identifier()
            # in release_channel/src/lib.rs uses RELEASE_CHANNEL=dev → still
            # "VibeDev-Dev-Instance-Mutex", consistent with $appMutex below.
            $appName = "VibeDev"
            $appDisplayName = "VibeDev"
            $appSetupName = "VibeDev-$Architecture"
            # AppMutex MUST equal "{app_identifier()}-Instance-Mutex" (release_channel).
            $appMutex = "VibeDev-Dev-Instance-Mutex"
            $appExeName = "VibeDev"
            $regValueName = "VibeDevDev"
            $appUserId = "ai.vibedev.VibeDev-Dev"
            $appShellNameShort = "&VibeDev"
            # VIBEDEV-TODO: appx identity still Zed (deferred, see stable block).
            $appAppxFullName = "ZedIndustries.Zed.Dev_1.0.0.0_neutral__japxn1gcva8rg"
        }
        default {
            Write-Error "can't bundle installer for $channel."
            exit 1
        }
    }

    # Windows runner 2022 default has iscc in PATH, https://github.com/actions/runner-images/blob/main/images/windows/Windows2022-Readme.md
    # Currently, we are using Windows 2022 runner.
    # Windows runner 2025 doesn't have iscc in PATH for now, https://github.com/actions/runner-images/issues/11228
    $innoSetupPath = "C:\Program Files (x86)\Inno Setup 6\ISCC.exe"

    $definitions = @{
        "AppId"          = $appId
        "AppIconName"    = $appIconName
        "OutputDir"      = "$env:ZED_WORKSPACE\target"
        "AppSetupName"   = $appSetupName
        "AppName"        = $appName
        "AppDisplayName" = $appDisplayName
        "RegValueName"   = $regValueName
        "AppMutex"       = $appMutex
        "AppExeName"     = $appExeName
        "ResourcesDir"   = "$innoDir"
        "ShellNameShort" = $appShellNameShort
        "AppUserId"      = $appUserId
        "Version"        = "$env:RELEASE_VERSION"
        "SourceDir"      = "$env:ZED_WORKSPACE"
        "AppxFullName"   = $appAppxFullName
    }

    $defs = @()
    foreach ($key in $definitions.Keys) {
        $defs += "/d$key=`"$($definitions[$key])`""
    }

    $innoArgs = @($issFilePath) + $defs
    if($env:CI) {
        $signTool = "powershell.exe -ExecutionPolicy Bypass -File $innoDir\sign.ps1 `$f"
        $innoArgs += "/sDefaultsign=`"$signTool`""
    }

    # Execute Inno Setup
    Write-Host "🚀 Running Inno Setup: $innoSetupPath $innoArgs"
    $process = Start-Process -FilePath $innoSetupPath -ArgumentList $innoArgs -NoNewWindow -Wait -PassThru

    if ($process.ExitCode -eq 0) {
        Write-Host "✅ Inno Setup successfully compiled the installer"
        Write-Output "SETUP_PATH=target/$appSetupName.exe" >> $env:GITHUB_ENV
        $script:buildSuccess = $true
    }
    else {
        Write-Host "❌ Inno Setup failed: $($process.ExitCode)"
        $script:buildSuccess = $false
    }
}

ParseZedWorkspace
$innoDir = "$env:ZED_WORKSPACE\inno\$Architecture"
$debugArchive = "$CargoOutDir\zed-$env:RELEASE_VERSION-$env:ZED_RELEASE_CHANNEL.dbg.zip"
$debugStoreKey = "$env:ZED_RELEASE_CHANNEL/zed-$env:RELEASE_VERSION-$env:ZED_RELEASE_CHANNEL.dbg.zip"

CheckEnvironmentVariables
PrepareForBundle
GenerateLicenses
BuildZedAndItsFriends
BuildRemoteServer
MakeAppx
SignZedAndItsFriends
# VIBEDEV: ZipZedAndItsFriendsDebug skipped. It builds a .dbg.zip out of the
# five PDBs (vibedev.pdb + cli + auto_update_helper + explorer_command_injector
# + remote_server) for Sentry symbol upload. Two reasons to skip it: (a)
# Sentry upload (UploadToSentry below) only runs in CI, never on local
# builds; (b) the release-fast profile uses debug="full", so vibedev.pdb
# alone is multi-GB and PowerShell 5.1's Compress-Archive hits its 2GB
# Stream-too-long limit, aborting the entire bundle. If we ever need the
# debug bundle (Sentry-enabled CI), re-enable + switch to 7z compression.
# ZipZedAndItsFriendsDebug
DownloadAMDGpuServices
DownloadConpty
BuildAgent
CollectFiles
BuildInstaller

if($env:CI) {
    UploadToSentry
}

if ($buildSuccess) {
    Write-Output "Build successful"
    if ($Install) {
        Write-Output "Installing Zed..."
        Start-Process -FilePath "$env:ZED_WORKSPACE/target/ZedEditorUserSetup-x64-$env:RELEASE_VERSION.exe"
    }
    exit 0
}
else {
    Write-Output "Build failed"
    exit 1
}
