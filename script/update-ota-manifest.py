#!/usr/bin/env python3
# VIBEDEV: Atomically add/update OTA manifest entries (runs ON the release server).
#
# Reads a JSON array of entries from STDIN:
#   [{"product":"vibedev","os":"macos","arch":"aarch64","version":"1.6.3","url":"https://.../x.dmg"}, ...]
#
# Safety model:
#   - Only the given (product, os, arch) leaves are set (add or overwrite).
#   - EVERY other pre-existing leaf is asserted byte-identical; if any would change
#     (a bug), it ABORTS without writing — so a windows/linux entry can never be
#     clobbered by a macos deploy.
#   - Writes to a temp file, validates JSON, then os.replace() (atomic rename).
#   - Backs up to <manifest>.bak.<epoch> first.
#
# Usage:
#   echo '<entries-json>' | update-ota-manifest.py --manifest PATH [--channel stable]
#   update-ota-manifest.py --manifest PATH --restore PATH.bak.<epoch>   # rollback
import argparse
import fcntl
import json
import os
import shutil
import sys
import time


def leaves(channel_obj):
    """Flatten channel -> {(product, os, arch): entry_dict}."""
    out = {}
    for product, plats in channel_obj.items():
        if not isinstance(plats, dict):
            continue
        for osname, arches in plats.items():
            if not isinstance(arches, dict):
                continue
            for arch, val in arches.items():
                out[(product, osname, arch)] = val
    return out


def canon(v):
    return json.dumps(v, sort_keys=True, ensure_ascii=False)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--manifest", required=True)
    ap.add_argument("--channel", default="stable")
    ap.add_argument("--restore", help="copy this backup over the manifest and exit")
    args = ap.parse_args()

    if args.restore:
        with open(args.restore, encoding="utf-8") as f:
            json.load(f)  # validate the backup parses
        shutil.copy2(args.restore, args.manifest)
        print(f"restored {args.manifest} <- {args.restore}")
        return

    entries = json.load(sys.stdin)
    if not isinstance(entries, list) or not entries:
        sys.exit("error: expected a non-empty JSON array of entries on stdin")

    # Serialize concurrent updaters (e.g. the IDE and agent repos releasing at the
    # same time, both editing this one manifest) so no update is lost.
    lock = open(f"{args.manifest}.lock", "w")
    fcntl.flock(lock, fcntl.LOCK_EX)
    try:
        with open(args.manifest, encoding="utf-8") as f:
            manifest = json.load(f)

        channel = manifest.setdefault(args.channel, {})
        before = {k: canon(v) for k, v in leaves(channel).items()}

        applied = set()
        for e in entries:
            for k in ("product", "os", "arch", "version", "url"):
                if k not in e:
                    sys.exit(f"error: entry missing '{k}': {e}")
            key = (e["product"], e["os"], e["arch"])
            (channel.setdefault(e["product"], {})
                    .setdefault(e["os"], {}))[e["arch"]] = {
                "version": e["version"],
                "url": e["url"],
            }
            applied.add(key)

        # SAFETY: nothing outside `applied` may have changed.
        after = {k: canon(v) for k, v in leaves(channel).items()}
        for k, v in before.items():
            if k in applied:
                continue
            if after.get(k) != v:
                sys.exit(f"ABORT: pre-existing entry {'/'.join(k)} would change — refusing to write")

        bak = f"{args.manifest}.bak.{int(time.time())}"
        shutil.copy2(args.manifest, bak)

        tmp = f"{args.manifest}.tmp.{os.getpid()}"
        with open(tmp, "w", encoding="utf-8") as f:
            json.dump(manifest, f, indent=2, ensure_ascii=False)
            f.write("\n")
        with open(tmp, encoding="utf-8") as f:
            json.load(f)  # validate before the swap
        os.replace(tmp, args.manifest)  # atomic
    finally:
        fcntl.flock(lock, fcntl.LOCK_UN)
        lock.close()

    applied_str = ", ".join(sorted("/".join(k) for k in applied))
    print(f"OK  backup={bak}")
    print(f"applied: {applied_str}")


if __name__ == "__main__":
    main()
