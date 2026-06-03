#!/usr/bin/env bash
# VIBEDEV release deploy: upload artifacts to the download station, atomically
# update the OTA manifest, and verify. NEVER hardcode host/credentials here —
# everything sensitive comes from the environment (CI secrets or a local
# untracked env file).
#
# Required env:
#   DEPLOY_HOST     release server host/IP
#   DEPLOY_SSH_KEY  path to the deploy user's private key
#   DOWNLOADS_DIR   server dir served at PUBLIC_PREFIX
#   MANIFEST_PATH   server path to the OTA manifest.json
#   PUBLIC_PREFIX   public URL base for downloads (e.g. https://host/downloads)
#   OTA_BASE        OTA endpoint base (e.g. https://host/vibedev/releases)
# Optional env:
#   DEPLOY_USER (default: deploy)   CHANNEL (default: stable)
#
# Usage:
#   release-deploy.sh --ide-version 1.6.3 [--agent-version 2.6.8] \
#       --artifacts ./dist --platform macos-aarch64 [--dry-run]
#
#   --platform : macos-aarch64 | linux-x86_64 | windows-x86_64
#   Only products whose artifact file exists in --artifacts are deployed.
set -euo pipefail

: "${DEPLOY_HOST:?set DEPLOY_HOST}"
: "${DEPLOY_SSH_KEY:?set DEPLOY_SSH_KEY (path to private key)}"
: "${DOWNLOADS_DIR:?set DOWNLOADS_DIR}"
: "${MANIFEST_PATH:?set MANIFEST_PATH}"
: "${PUBLIC_PREFIX:?set PUBLIC_PREFIX}"
: "${OTA_BASE:?set OTA_BASE}"
DEPLOY_USER="${DEPLOY_USER:-deploy}"
CHANNEL="${CHANNEL:-stable}"

IDE_VERSION=""; AGENT_VERSION=""; ARTIFACTS=""; PLATFORM=""; DRYRUN=0
while [ $# -gt 0 ]; do
  case "$1" in
    --ide-version)   IDE_VERSION="$2"; shift 2;;
    --agent-version) AGENT_VERSION="$2"; shift 2;;
    --artifacts)     ARTIFACTS="$2"; shift 2;;
    --platform)      PLATFORM="$2"; shift 2;;
    --dry-run)       DRYRUN=1; shift;;
    -h|--help)       grep '^#' "$0" | sed 's/^# \{0,1\}//'; exit 0;;
    *) echo "unknown arg: $1" >&2; exit 2;;
  esac
done
[ -n "$IDE_VERSION" ] && [ -n "$ARTIFACTS" ] && [ -n "$PLATFORM" ] || {
  echo "usage: release-deploy.sh --ide-version X [--agent-version Y] --artifacts DIR --platform PLAT [--dry-run]" >&2
  exit 2; }
[ -d "$ARTIFACTS" ] || { echo "artifacts dir not found: $ARTIFACTS" >&2; exit 2; }
case "$PLATFORM" in macos-aarch64|linux-x86_64|windows-x86_64) ;; *)
  echo "unsupported --platform: $PLATFORM" >&2; exit 2;; esac

OS="${PLATFORM%-*}"; ARCH="${PLATFORM##*-}"
IDE_VN="${IDE_VERSION//./}"; AGENT_VN="${AGENT_VERSION//./}"

SSH_OPTS=(-i "$DEPLOY_SSH_KEY" -o IdentitiesOnly=yes -o StrictHostKeyChecking=accept-new -o ConnectTimeout=20)
ssh_do() { ssh "${SSH_OPTS[@]}" "$DEPLOY_USER@$DEPLOY_HOST" "$@"; }
scp_to() { scp "${SSH_OPTS[@]}" "$1" "$DEPLOY_USER@$DEPLOY_HOST:$2"; }

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
WORK="$(mktemp -d)"; trap 'rm -rf "$WORK"' EXIT
ENTRIES_TSV="$WORK/entries.tsv"; : > "$ENTRIES_TSV"

# rows: localfile <TAB> primary_dl_name <TAB> alias_csv <TAB> product <TAB> version
plan() {
  case "$PLATFORM" in
    macos-aarch64)
      printf '%s|%s|%s|%s|%s\n' "VibeDev-aarch64.dmg" "VibeDev-apple-silicon.dmg" "VibeDev-${IDE_VERSION}-apple-silicon.dmg" "vibedev" "$IDE_VERSION"
      printf '%s|%s|%s|%s|%s\n' "vibedev-remote-server-macos-aarch64.gz" "vibedev-remote-server-macos-aarch64-${IDE_VN}.gz" "" "vibedev-remote-server" "$IDE_VERSION"
      [ -n "$AGENT_VERSION" ] && printf '%s|%s|%s|%s|%s\n' "vibedev-agent-macos-aarch64.tar.gz" "vibedev-agent-macos-aarch64-${AGENT_VN}.tar.gz" "" "vibedev-agent" "$AGENT_VERSION"
      ;;
    linux-x86_64)
      printf '%s|%s|%s|%s|%s\n' "vibedev-remote-server-linux-x86_64.gz" "vibedev-remote-server-linux-x86_64-${IDE_VN}.gz" "" "vibedev-remote-server" "$IDE_VERSION"
      [ -n "$AGENT_VERSION" ] && printf '%s|%s|%s|%s|%s\n' "vibedev-agent-linux-x86_64.tar.gz" "vibedev-agent-linux-x86_64-${AGENT_VN}.tar.gz" "" "vibedev-agent" "$AGENT_VERSION"
      # NOTE: Linux IDE ('vibedev') is not in the OTA manifest today — see CICD-RELEASE-PLAN.md §8.
      ;;
    windows-x86_64)
      # Download names confirmed on server: VibeDevSetup-<ver>.exe (manifest target)
      # + VibeDevSetup.exe (version-less "latest" alias for the landing page).
      # The LOCAL build-artifact name from bundle-windows.ps1 is still TBD — plan §8.
      printf '%s|%s|%s|%s|%s\n' "VibeDevSetup-${IDE_VERSION}.exe" "VibeDevSetup-${IDE_VERSION}.exe" "VibeDevSetup.exe" "vibedev" "$IDE_VERSION"
      printf '%s|%s|%s|%s|%s\n' "vibedev-remote-server-windows-x86_64.zip" "vibedev-remote-server-windows-x86_64-${IDE_VN}.zip" "" "vibedev-remote-server" "$IDE_VERSION"
      [ -n "$AGENT_VERSION" ] && printf '%s|%s|%s|%s|%s\n' "vibedev-agent-windows-x86_64.tar.gz" "vibedev-agent-windows-x86_64-${AGENT_VN}.tar.gz" "" "vibedev-agent" "$AGENT_VERSION"
      ;;
  esac
}

echo "== VibeDev release-deploy =="
echo "   platform=$PLATFORM  ide=$IDE_VERSION  agent=${AGENT_VERSION:-<none>}  channel=$CHANNEL  dry-run=$DRYRUN"
echo "   target=$DEPLOY_USER@$DEPLOY_HOST:$DOWNLOADS_DIR"

found=0
while IFS='|' read -r localf primary aliases product version <&3; do
  [ -z "${localf:-}" ] && continue
  src="$ARTIFACTS/$localf"
  if [ ! -f "$src" ]; then echo "  - skip $product: no '$localf' in $ARTIFACTS"; continue; fi
  found=$((found+1))
  size=$(wc -c < "$src" | tr -d ' ')
  echo "  + $product  $localf ($size B)  ->  $primary${aliases:+  (alias: $aliases)}"
  if [ "$DRYRUN" = "0" ]; then
    # Upload to a temp name then atomic-rename into place: the new file is owned
    # by the deploy user (so chmod works even when overwriting a www-owned file),
    # and downloads never see a half-written or missing file.
    tmp="$DOWNLOADS_DIR/.deploytmp.$$"
    scp_to "$src" "$tmp"
    ssh_do "chmod 644 '$tmp' && mv -f '$tmp' '$DOWNLOADS_DIR/$primary'"
    if [ -n "$aliases" ]; then
      IFS=',' read -ra AL <<< "$aliases"
      for a in "${AL[@]}"; do
        [ -n "$a" ] || continue
        ssh_do "cp -f '$DOWNLOADS_DIR/$primary' '$tmp' && chmod 644 '$tmp' && mv -f '$tmp' '$DOWNLOADS_DIR/$a'"
      done
    fi
  fi
  printf '%s|%s|%s|%s|%s\n' "$product" "$OS" "$ARCH" "$version" "$PUBLIC_PREFIX/$primary" >> "$ENTRIES_TSV"
done 3< <(plan)

[ "$found" -gt 0 ] || { echo "no artifacts matched in $ARTIFACTS for $PLATFORM" >&2; exit 1; }

ENTRIES_JSON="$(python3 -c '
import json,sys
out=[]
for line in open(sys.argv[1], encoding="utf-8"):
    line=line.rstrip("\n")
    if not line: continue
    p,o,a,v,u=line.split("|")
    out.append({"product":p,"os":o,"arch":a,"version":v,"url":u})
print(json.dumps(out))' "$ENTRIES_TSV")"
echo "== manifest entries =="
echo "$ENTRIES_JSON" | python3 -m json.tool

if [ "$DRYRUN" = "1" ]; then
  echo "(dry-run) nothing uploaded, manifest untouched."
  exit 0
fi

echo "== atomic manifest update =="
scp_to "$SCRIPT_DIR/update-ota-manifest.py" "/tmp/update-ota-manifest.py"
echo "$ENTRIES_JSON" | ssh_do "python3 /tmp/update-ota-manifest.py --manifest '$MANIFEST_PATH' --channel '$CHANNEL'; rm -f /tmp/update-ota-manifest.py"

echo "== verify =="
rc=0
while IFS='|' read -r product os arch version url; do
  [ -z "${product:-}" ] && continue
  ep="$OTA_BASE/$CHANNEL/$version/asset?os=$os&arch=$arch&asset=$product"
  resp="$(curl -fsS "$ep" 2>/dev/null || true)"
  if printf '%s' "$resp" | grep -qF "$url"; then
    echo "  OK  OTA $product/$os/$arch@$version resolves correctly"
  else
    echo "  !!  OTA $product/$os/$arch@$version BAD: $resp"; rc=1
  fi
  head="$(curl -fsSI "$url" 2>/dev/null | grep -iE '^HTTP|content-length' | tr '\n' ' ' || true)"
  echo "      DL $url -> ${head:-NO RESPONSE}"
done < "$ENTRIES_TSV"
[ "$rc" = "0" ] && echo "== done (verified) ==" || { echo "== DONE WITH VERIFY FAILURES =="; exit 1; }
