# VibeDev i18n (build-time Simplified Chinese)

VibeDev keeps its **committed source 100% English** so the fork can keep
rebasing onto upstream Zed. Chinese localization is applied as a **build-time
source transform** (the [zed-globalization](https://github.com/x6nux/zed-globalization)
model): English string literals are replaced with Chinese just before
compiling, then reverted. There is **no runtime i18n framework** and no
permanent edits to the source.

## Files

- `zh-CN.json` — base translations for upstream Zed strings (from
  zed-globalization, `v1.4.2-pre` snapshot). Shape: `{ "zed/<file>": { "English": "中文" } }`.
- `vibedev-overrides.json` — VibeDev-specific strings (welcome / menus). The
  "VibeDev" product name is intentionally left untranslated.
- `do_not_translate.json` — per-file + global allowlist of strings that must
  **not** be translated (code identifiers, format keys, MIME types, …).
- `build-zh.ps1` — apply translations → `cargo build -p zed` → restore English.
- `NOTICE` — third-party attribution (GPL-3.0).

## Producing a Chinese build

Prereqs:

- `pip install git+https://github.com/x6nux/zed-globalization.git` (provides the
  `zedl10n` CLI).
- An MSVC build environment on PATH (run from a `vcvars64` shell with CMake) —
  the same environment a normal `cargo build -p zed` needs on Windows.

```powershell
# from an MSVC-env shell:
./vibedev-i18n/build-zh.ps1             # apply -> build -> restore English source
./vibedev-i18n/build-zh.ps1 -ApplyOnly  # apply only; build/inspect, then: git restore .
```

Dev builds stay English (plain `cargo build -p zed`); the Chinese build is for
release / QA. The script refuses to run on a dirty tree (it edits source in
place, then restores) and always restores English source in a `finally` block.

## Updating after a Zed rebase

Re-fetch the matching-version `zh-CN.json` from zed-globalization's `i18n`
branch. Translations are keyed by the English string, so strings unchanged
across versions still match; new or changed upstream strings simply fall back
to English until translated.
