#!/usr/bin/env bash
# VibeDev - produce a Simplified-Chinese build by replacing English string
# literals at build time, then restoring the English source. See README.md.
#
# This is the POSIX-shell mirror of build-zh.ps1 (for macOS / Linux release &
# QA builds). The Chinese lives in the JSON data files, read as UTF-8 by
# zedl10n; this script never embeds non-ASCII text.
#
# Prereqs:
#   pip install git+https://github.com/x6nux/zed-globalization.git   (zedl10n CLI)
#   a normal "cargo build -p zed" toolchain on PATH.
#
# Usage (from anywhere):
#   ./vibedev-i18n/build-zh.sh             # apply -> build -> restore English
#   ./vibedev-i18n/build-zh.sh --apply-only  # apply only; build/inspect, then: git restore .
set -euo pipefail

i18n="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"   # .../zed/vibedev-i18n
zed="$(dirname "$i18n")"                                # .../zed   (repo root - must be named "zed",
                                                        #            matching the "zed/<file>" keys)
root="$(dirname "$zed")"                                # .../v2    (source-root: contains the zed/ dir)

apply_only=0
if [ "${1:-}" = "--apply-only" ]; then
  apply_only=1
fi

if ! command -v zedl10n >/dev/null 2>&1; then
  echo "zedl10n not found. Install: pip install git+https://github.com/x6nux/zed-globalization.git" >&2
  exit 1
fi

# Refuse to run on a dirty tree: this edits tracked source in place, then runs
# 'git restore .' which would also revert any uncommitted work.
if [ -n "$(git -C "$zed" status --short --untracked-files=no)" ]; then
  echo "tracked files have uncommitted changes - commit or stash first (this edits tracked source in place, then runs 'git restore .' which would also revert your changes)." >&2
  exit 1
fi

# VIBEDEV: order matters - overrides FIRST, then zh-CN. zedl10n replaces an
# English-source-literal with a Chinese-target-literal in .rs files. If we ran
# zh-CN first, source strings like "Welcome to Zed AI" would already be
# rewritten, the overrides pass would find nothing, and "Zed" would leak through
# the localized UI. Running overrides first means the English keys are still in
# source, our brand-corrected translations land, and the zh-CN pass that follows
# simply skips entries whose source string is already gone (harmless no-op).
echo "[build-zh] applying VibeDev overrides (must run BEFORE zh-CN) ..."
zedl10n replace --input "$i18n/vibedev-overrides.json" --source-root "$root" --do-not-translate "$i18n/do_not_translate.json"
echo "[build-zh] applying base zh-CN translations ..."
zedl10n replace --input "$i18n/zh-CN.json"             --source-root "$root" --do-not-translate "$i18n/do_not_translate.json"

# VIBEDEV: populate the runtime translation table (command-palette labels + ACP
# mode names) that zedl10n cannot reach -- they are computed at runtime, not
# source literals. Committed empty ({}); copied here so the Chinese build embeds
# it, and the "git restore ." below reverts it to {} for the English source tree.
# NOTE (macOS release trap): if this copy is skipped, ALL runtime translations
# ship in English. Always run THIS script (not a bare cargo build) for zh builds.
echo "[build-zh] populating vibedev_i18n runtime table ..."
cp -f "$i18n/runtime-zh.json" "$zed/crates/vibedev_i18n/src/runtime-translations.json"

if [ "$apply_only" -eq 1 ]; then
  echo "[build-zh] applied (source is now Chinese). Build manually, then restore with:"
  echo "           git -C \"$zed\" restore ."
  exit 0
fi

# Always restore English source, even if the build fails.
restore_english() {
  echo "[build-zh] restoring English source ..."
  git -C "$zed" restore .
}
trap restore_english EXIT

echo "[build-zh] building (cargo build -p zed) ..."
( cd "$zed" && cargo build -p zed )

echo "[build-zh] done - Chinese build at target/debug/zed; source restored to English."
