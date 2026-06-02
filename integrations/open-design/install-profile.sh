#!/usr/bin/env bash
# Install (or remove) the VibeDev agent profile into a local Open Design
# install — https://github.com/nexu-io/open-design.
#
# Usage:
#   ./install-profile.sh             # install
#   ./install-profile.sh --uninstall # remove
#   ./install-profile.sh --quiet     # no output (for use from package post-install hooks)
#
# Behaviour mirrors install-profile.ps1:
#   - Only acts if ~/.open-design/profiles/ already exists (i.e. user has
#     actually launched Open Design at least once). Never creates the
#     directory ourselves.
#   - Idempotent: re-running overwrites; --uninstall removes only our file.
#
# Used by VibeDev's macOS .dmg / Linux .deb / .AppImage post-install hooks
# and by users who want to hand-register on a stock install.

set -euo pipefail

ACTION="install"
QUIET=0
for arg in "$@"; do
    case "$arg" in
        --uninstall) ACTION="uninstall" ;;
        --quiet)     QUIET=1 ;;
        -h|--help)
            sed -n '2,/^set/p' "$0" | sed 's/^# \{0,1\}//'
            exit 0
            ;;
        *) echo "unknown arg: $arg" >&2; exit 2 ;;
    esac
done

log() { [ "$QUIET" -eq 0 ] && echo "[od-integration] $*"; }

# Resolve script dir (for the vibedev.ts source path).
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SOURCE_PATH="$SCRIPT_DIR/vibedev.ts"
OD_PROFILES_DIR="$HOME/.open-design/profiles"
TARGET_PATH="$OD_PROFILES_DIR/vibedev.ts"

if [ ! -d "$OD_PROFILES_DIR" ]; then
    log "Open Design not detected at $OD_PROFILES_DIR; nothing to do."
    exit 0
fi

if [ "$ACTION" = "uninstall" ]; then
    if [ -f "$TARGET_PATH" ]; then
        rm -f "$TARGET_PATH"
        log "Removed $TARGET_PATH"
    else
        log "Nothing to remove at $TARGET_PATH"
    fi
    exit 0
fi

if [ ! -f "$SOURCE_PATH" ]; then
    echo "vibedev.ts not found next to this script: $SOURCE_PATH" >&2
    exit 1
fi

cp -f "$SOURCE_PATH" "$TARGET_PATH"
log "Installed VibeDev profile -> $TARGET_PATH"
log "Restart Open Design to pick it up."
