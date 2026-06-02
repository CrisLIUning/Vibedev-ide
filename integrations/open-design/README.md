# VibeDev × Open Design

Drop-in profile that registers VibeDev (the `ccb` / `claude-code-best` ACP
agent) as a selectable backend inside [Open Design][od]. Once installed,
Open Design's agent picker shows **VibeDev** next to Claude Code / Codex /
Cursor / Gemini / etc., and every design Skill that Open Design runs goes
through ccb → sub2 gateway → DeepSeek / Claude / GPT.

[od]: https://github.com/nexu-io/open-design

## Why this exists

- Open Design's daemon auto-detects coding-agent CLIs on `PATH` and uses
  them as the engine for its 132 design Skills + 150 Design Systems. It
  ships with 20 built-in adapter definitions under
  `apps/daemon/src/runtimes/defs/`.
- It also reads user-provided adapter definitions from
  `~/.open-design/profiles/*.ts` at startup (see `local-profiles.ts`),
  which is how we register VibeDev without forking the upstream repo.
- claude-code-best is a fork of Claude Code that preserves the entire CLI
  surface (`-p`, `--input-format stream-json`, `--output-format
  stream-json`, `--include-partial-messages`, `--add-dir`,
  `--permission-mode bypassPermissions`, `--mcp-config`, MCP-via-cwd).
  Open Design's `claude.ts` adapter therefore works verbatim against
  ccb — we only change the `id` / `name` / `bin` / `fallbackModels` /
  `env`.

## Install

### Auto (VibeDev installer)

The VibeDev installer's post-install hook runs `install-profile.ps1`
(Windows) or `install-profile.sh` (macOS / Linux) automatically. It checks
for `~/.open-design/profiles/` and, if present, drops `vibedev.ts` in. If
Open Design isn't installed, it exits silently — we never create
directories for products the user hasn't installed.

### Manual

#### Windows

```powershell
# From a checkout of the VibeDev repo:
cd integrations\open-design
.\install-profile.ps1
```

#### macOS / Linux

```bash
cd integrations/open-design
./install-profile.sh
```

#### Or just copy

```
cp vibedev.ts ~/.open-design/profiles/vibedev.ts
```

Then restart Open Design (Cmd/Ctrl+Q and relaunch). The picker will
include **VibeDev** alongside the built-in agents.

## Uninstall

```powershell
# Windows
.\install-profile.ps1 -Uninstall
```

```bash
# macOS / Linux
./install-profile.sh --uninstall
```

This removes `~/.open-design/profiles/vibedev.ts` and leaves the rest of
Open Design untouched.

## What the profile does

The complete adapter is `vibedev.ts` in this directory. Key fields:

| Field | Value | Notes |
| --- | --- | --- |
| `id` | `vibedev` | Independent namespace — never collides with built-in `claude` |
| `name` | `VibeDev` | Shown in Open Design's agent picker |
| `bin` | `ccb` | Primary lookup on PATH |
| `fallbackBins` | `['claude-code-best', 'ccb-bun']` | Other names ccb ships under |
| `streamFormat` | `claude-stream-json` | Reuses Open Design's Claude stream parser |
| `externalMcpInjection` | `claude-mcp-json` | Same MCP wiring as `claude.ts` |
| `fallbackModels` | DeepSeek + Claude + GPT-5.5 ids | Live ids accepted by sub2 |
| `env.CLAUDE_CODE_USE_OPENAI` | `1` | Routes ccb through the sub2 OpenAI-shaped gateway |
| `env.CLAUDE_CONFIG_DIR` | `~/.vibedev` | Isolates VibeDev's config from a parallel Claude Code install |
| `env.OPENAI_DEFAULT_*_MODEL` | DeepSeek aliases | Default model picks when Open Design sends `model: "default"` |

`buildArgs` is identical to `claude.ts` — every flag is preserved by ccb.
See inline comments in `vibedev.ts` for full rationale.

## Verifying it worked

After install + Open Design restart:

1. Open the agent picker (top-right in Open Design's main view, or via
   the Skills sidebar).
2. **VibeDev** should appear in the list alongside Claude Code, Codex,
   etc.
3. Selecting it and running any Skill (e.g. "Generate a landing page")
   spawns `ccb -p --input-format stream-json --output-format
   stream-json ...` and streams the result back into Open Design's
   sandboxed preview.
4. Open Design's settings → Agents page shows VibeDev's detected version
   (matches `ccb --version` output).

If VibeDev doesn't appear:

- Confirm `ccb --version` works on your `PATH`. If not, the VibeDev
  installer didn't add `<install-dir>/agent/` to `PATH` (or you're on a
  dev checkout — install globally with `bun install -g
  claude-code-best`).
- Confirm `~/.open-design/profiles/vibedev.ts` exists.
- Check Open Design's daemon log for parse errors (`~/.open-design/logs/`
  or via Open Design's Settings → Logs).

## Maintenance

- `fallbackModels` must stay in sync with what the sub2 gateway returns
  from `/v1/models`. Source of truth is
  `claude-code-best/src/services/api/openai/modelList.ts`. When a new
  model lands gateway-side, mirror it here.
- The `buildArgs` shape and `--permission-mode bypassPermissions` choice
  match upstream Open Design's `claude.ts`. If Open Design upgrades its
  Claude adapter (e.g. adds a flag, changes prompt format), regenerate
  this file from the new `claude.ts` rather than diverging.

## License

This integration glue is dual-licensed under Apache 2.0 (to match Open
Design) and GPL v3 (to match the rest of the VibeDev fork). Use whichever
fits your downstream needs.
