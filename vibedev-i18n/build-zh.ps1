#requires -Version 5
# VibeDev - produce a Simplified-Chinese build by replacing English string
# literals at build time, then restoring the English source. See README.md.
#
# This script is intentionally pure ASCII: Windows PowerShell 5.1 reads a .ps1
# without a BOM as the system ANSI codepage, so any non-ASCII char here would
# break parsing. (The Chinese lives in the JSON data files, read as UTF-8 by
# zedl10n, never executed by PowerShell.)
#
# Prereqs:
#   pip install git+https://github.com/x6nux/zed-globalization.git   (zedl10n CLI)
#   an MSVC build env on PATH (run from a vcvars64 shell + CMake), like a normal
#   "cargo build -p zed".
[CmdletBinding()]
param([switch]$ApplyOnly)
$ErrorActionPreference = "Stop"

$i18n = $PSScriptRoot                # ...\zed\vibedev-i18n
$zed  = Split-Path -Parent $i18n     # ...\zed   (repo root - must be named "zed",
                                     #            matching the "zed/<file>" keys)
$root = Split-Path -Parent $zed      # ...\v2    (source-root: contains the zed/ dir)

if (-not (Get-Command zedl10n -ErrorAction SilentlyContinue)) {
    throw "zedl10n not found. Install: pip install git+https://github.com/x6nux/zed-globalization.git"
}
if (git -C $zed status --short --untracked-files=no) {
    throw "tracked files have uncommitted changes - commit or stash first (this edits tracked source in place, then runs 'git restore .' which would also revert your changes)."
}

# VIBEDEV: order matters - overrides FIRST, then zh-CN. zedl10n replaces an
# English-source-literal with a Chinese-target-literal in .rs files. If we ran
# zh-CN first, source strings like "Welcome to Zed AI" would already be
# rewritten to "Welcome to Zed AI"-in-Chinese-with-literal-Zed. The overrides
# pass would then search for the English key in source, find nothing (already
# Chinese), and silently no-op - leaving "Zed" leaking through the localized
# UI. Running overrides first means the English keys are still in source, our
# brand-corrected translations land, and the zh-CN pass that follows simply
# skips entries whose source string is already gone (do-no-op, harmless).
Write-Host "[build-zh] applying VibeDev overrides (must run BEFORE zh-CN) ..."
zedl10n replace --input "$i18n\vibedev-overrides.json" --source-root $root --do-not-translate "$i18n\do_not_translate.json"
Write-Host "[build-zh] applying base zh-CN translations ..."
zedl10n replace --input "$i18n\zh-CN.json"             --source-root $root --do-not-translate "$i18n\do_not_translate.json"

if ($ApplyOnly) {
    Write-Host "[build-zh] applied (source is now Chinese). Build manually, then restore with:"
    Write-Host ("           git -C " + '"' + $zed + '"' + " restore .")
    return
}

try {
    Push-Location $zed
    Write-Host "[build-zh] building (cargo build -p zed) ..."
    cargo build -p zed
} finally {
    Pop-Location
    Write-Host "[build-zh] restoring English source ..."
    git -C $zed restore .
}
Write-Host "[build-zh] done - Chinese build at target\debug\zed.exe; source restored to English."
