# VibeDev × Zed — Fork Patch Ledger

> **Why this file:** VibeDev forks Zed by **adding new crates** (own files → never
> conflict) plus a **minimal set of append-only edits** to a handful of upstream
> files. This ledger lists every upstream touch so each Zed release can be rebased
> mechanically. **Locate any touch with `git grep -n vibedev <file>`** — the line
> numbers below are indicative (verified 2026-05-27) and drift across rebases, so
> grep, don't trust the numbers.

## 1. New crates (own files — zero upstream conflict)

| Crate | Responsibility |
|---|---|
| `crates/vibedev_account/` | Data + HTTP glue to the local sidecar (account/providers/login); spawns + supervises the `bun` sidecar (`launcher.rs`). No Zed-trait impls. |
| `crates/vibedev_ui/` | `VibedevAccountPanel` (impl Zed `Panel`), `VibedevCostStatusItem` (impl `StatusItemView`), and `init(cx)`. Depends on `vibedev_account`. |

These never conflict on rebase. The ongoing cost is **tracking Zed trait changes**
(`Panel` / `StatusItemView` signatures) — that's internal to these crates, it does
not touch upstream files.

## 2. Upstream touch points (append-only)

| # | File | ~Lines (2026-05-27) | Change |
|---|---|---|---|
| 1 | `Cargo.toml` (workspace root) | 220–221 | `[workspace].members += "crates/vibedev_account", "crates/vibedev_ui"` |
| 2 | `Cargo.toml` (workspace root) | 481–482 | `[workspace.dependencies] += vibedev_account = { path = … }`, `vibedev_ui = { path = … }` |
| 3 | `crates/zed/Cargo.toml` | 216 | `vibedev_ui.workspace = true` (pulls `vibedev_account` transitively) |
| 4 | `crates/zed/src/main.rs` | 745 | `vibedev_ui::init(cx);` (right after `project_panel::init(cx);`) |
| 5 | `crates/zed/src/zed.rs` | 568–570, 603 | Cost status item: construct `vibedev_cost` + `status_bar.add_right_item(vibedev_cost, …)` |
| 6 | `crates/zed/src/zed.rs` | 738–741, 765–766 | **`initialize_panels()`**: `VibedevAccountPanel::load`, then `add_panel_when_ready(vibedev_account_panel, …)` inside the `futures::join!` (Providers panel removed — superseded by in-chat config) |
| 7 | `crates/zed_actions/src/lib.rs` | 366–376 | `pub mod vibedev { actions!(vibedev, [ToggleAccountFocus]) }` |
| 8 | `assets/settings/default.json` | ~2664–2710 | `agent_servers.VibeDev` block (ACP backend, `type:"custom"`), wrapped in `// VIBEDEV:` comments. `command` + `args` use the `${ZED_BIN_DIR}` placeholder expanded by `settings::default_settings()`. `env` carries only non-secret config (`CLAUDE_CODE_USE_OPENAI` + `OPENAI_DEFAULT_*` model alias map). **`CLAUDE_CONFIG_DIR` is intentionally NOT set here** — the backend's `runAcpAgent()` defaults it to `~/.vibedev` via `ensureVibedevConfigDir()` (claude-code-best a1c02ca), so this fork stays user-name-free and works for every checkout out of the box. |
| 9 | `assets/settings/default.json` | ~517, 937, 1486–1491, 1659 | De-Zed defaults: `title_bar.show_sign_in:false`, `collaboration_panel.button:false`, `auto_update:false`, `edit_predictions.provider:"none"` (kills the Zeta "Sign In & Start Using" edit-prediction onboarding). **`telemetry.{diagnostics,metrics}` restored to `true`** once endpoints were rerouted to our own sub2api collector (see row #27) — those defaults were `false` originally to avoid phoning home to zed.dev. |
| 10 | `crates/agent_ui/src/agent_panel.rs` | ~1169, ~5104, ~5176, ~5197 | Default agent = `Agent::Custom { id: AgentId::new("VibeDev") }` (non-collab); **VibeDev replaces the "Zed Agent" primary picker entry** (~5104 — launches the custom ACP agent via `new_external_agent_thread`, not `Agent::NativeAgent`); VibeDev filtered out of the `external_agents()` list (~5176); "External Agents" header **kept** for other/future agents (~5197) |
| 11 | `crates/release_channel/src/lib.rs` | ~192 | `display_name()` "Zed*" → "VibeDev*" (window title + About dialog) |
| 12 | `crates/workspace/src/welcome.rs` | ~447, ~479 | Welcome headline "Welcome (back) to Zed" → "…VibeDev"; Zed tagline ("The editor for what's next") removed |
| 13 | `crates/onboarding/src/onboarding.rs` | ~353 | Onboarding headline → "Welcome to VibeDev"; Zed tagline removed |
| 14 | `crates/onboarding/src/basics_page.rs` (+ `crates/onboarding/Cargo.toml`) | ~587, ~678 | "Agent Setup": `render_zed_agent_button` → `render_vibedev_agent_button` (triggers sub2api login via `vibedev_account::start_login`, not Zed's `client.sign_in`); featured-agent install cards kept; `+ vibedev_account` dep |
| 15 | `crates/zed/src/zed/app_menus.rs` | ~63–104 | App menu name + About/Hide/Quit "Zed" → "VibeDev" |
| 16 | `crates/command_palette/src/command_palette.rs` | ~114 | Display-only: remap `zed::` action namespace → `vibedev::` in the palette (action ids/keymaps untouched) |
| 17 | `assets/images/zed_logo.svg` | (whole file) | Zed glyph → VibeDev waveform (mono, theme-tinted); `VectorName::ZedLogo` path unchanged → welcome + onboarding logos both swap |
| 18 | `crates/release_channel/src/lib.rs` (+ `crates/zed/Cargo.toml` `[bundle-*]`) | ~215 | `app_id()` + macOS bundle `identifier` "dev.zed.Zed-*" → "ai.vibedev.VibeDev-*" (AppUserModelID / WM_CLASS / macOS bundle id — fixes the cached Windows taskbar/jump-list name). This is the *grouping* id; data dir is `paths::APP_NAME` (row #22) and the mutex/pipe id is `app_identifier()` (row #23). |
| 19 | `crates/windows_resources/src/windows_resources.rs` | ~47–53 | Version-resource ProductName/FileDescription "Zed*" → "VibeDev*" (taskbar / exe properties); + `cargo:rerun-if-changed` for the icon so icon swaps re-embed |
| 20 | `crates/zed/resources/windows/app-icon-dev.ico`, `crates/zed/resources/app-icon-dev.png` (+ `@2x`) | (whole files) | Zed dev app icon → VibeDev waveform (generated from v1 SVG via sharp + png-to-ico) |
| 21 | `crates/vibedev_ui/src/vibedev_ui.rs` (`init`) + `global_skills.rs` | — | Hides Zed-only palette actions (`EmailZed`/`OpenZedRepo`/`RegisterZedScheme`/`OpenZedPredictOnboarding`) via `CommandPaletteFilter`; adds deps `feedback`/`install_cli`/`command_palette_hooks`/`agent_skills` (our crate — noted for rebase awareness). On workspace open, `global_skills::refresh_global_skill_index` scans `agent_skills::global_skills_dir()` (= `~/.vibedev/skills`) and publishes into the `SkillIndex` global so the `#agent.skills` settings page lists them — the NativeAgent path that normally fills `SkillIndex` doesn't run under our ACP agent. |
| 22 | `crates/paths/src/paths.rs`; `crates/zed/Cargo.toml`; `crates/zed/src/main.rs`; `crates/cli/src/main.rs` | `paths.rs` APP_NAME; `Cargo.toml` 9/57–60; `main.rs` ~9; `cli/main.rs` ~1212 | **Binary rename + data-dir isolation.** `paths::APP_NAME` "Zed"→"VibeDev" (→ `%APPDATA%\VibeDev`, `~/.vibedev`-adjacent); `crates/zed/Cargo.toml` `default-run`+`[[bin]] name` "zed"→"vibedev" (package stays "zed" so `cargo build -p zed` works → emits `vibedev.exe`); `main.rs` compile-assert `APP_NAME_LOWERCASE == CARGO_BIN_NAME` forces the two to agree; `cli/main.rs` exe-detection probes `../VibeDev.exe` / `./vibedev.exe`. |
| 23 | `crates/release_channel/src/lib.rs` (`app_identifier()`, Windows) | ~31–38 | Single-instance **mutex + named-pipe** id "Zed-Editor-*" → "VibeDev-*" so VibeDev never shares a mutex/pipe with a real Zed install (else launching VibeDev while Zed runs forwards args into Zed). Couples to installer AppMutex (row #24). |
| 24 | `script/bundle-windows.ps1`; `crates/zed/resources/windows/zed.iss`; `crates/zed/resources/windows/zed.sh` | ps1 4 channel blocks + staging copy; iss `[Files]` + URI handler; sh whole | **Windows installer rebrand.** ps1: cargo output `vibedev.exe`/`.pdb`, stage as `VibeDev.exe`, CLI command `zed`→`vibedev`; per-channel `$appName`/`$appExeName`/`$appSetupName`/`$regValueName` → VibeDev, **fresh `$appId` GUIDs** (own product, not a Zed upgrade), `$appUserId` = row-#18 `app_id()`, `$appMutex` = row-#23 `app_identifier()`+"-Instance-Mutex". iss: staging `Source` `Zed.exe`→`VibeDev.exe`, `zed://` handler exe path rebranded (scheme name kept — internal). **Deferred:** `$appAppxFullName` (Win11 explorer appx identity — needs new publisher + family-hash). |
| 25 | `crates/edit_prediction/src/open_ai_compatible.rs` (+ `crates/vibedev_account/Cargo.toml` dep + `crates/vibedev_account/src/launcher.rs`) | `open_ai_compatible.rs` end-of-file `vibedev_configure_fim` + `OPEN_AI_COMPATIBLE_TOKEN_ENV_VAR_NAME` const; `launcher.rs` `configure_fim` | **Inline-autocomplete (FIM) → loopback sidecar.** `vibedev_configure_fim(cx, port, token)` points the `open_ai_compatible_api` edit-prediction provider at the sidecar's `POST http://127.0.0.1:{port}/v1/completions` (model `deepseek-coder`, format `DeepseekCoder`, 256 max tokens) and carries the per-launch bearer token. **Why the helper lives in `edit_prediction`:** the `GlobalOpenAiCompatibleApiKey` key-state global is private to that crate, and the env-var (`ZED_OPEN_AI_COMPATIBLE_EDIT_PREDICTION_API_KEY`) is read once into a `LazyLock` that `EditPredictionButton::new` materializes at startup — so a late `set_var` loses the race; we must OVERWRITE the global `ApiKeyState` with a fresh `EnvVar { value: Some(token) }`. The dynamic `api_url` (runtime port) is written to user settings via `SettingsStore::update_settings_file`. `launcher.rs` calls it on the foreground `AsyncApp` after each successful handshake (`vibedev_account` gains an `edit_prediction` dep). `default.json` `edit_predictions.provider` stays `"none"` (row #9) — the runtime write flips it on once the sidecar is ready. |
| 26 | `crates/edit_prediction_ui/src/edit_prediction_button.rs` (`get_available_providers`) | ~1444 | **De-Zed the edit-prediction provider menu.** Stop listing "Zed AI" (upstream pushed `EditPredictionProvider::Zed` unconditionally) and GitHub Copilot (upstream pushed it when GitHub-authed). Leaves the opt-in third-party providers (Codestral / Ollama / Mercury — only listed when the user configures a key / local runtime) + our `OpenAiCompatibleApi` (the VibeDev FIM, row #25). Also makes the Zed-cloud sign-in (zed.dev / GitHub OAuth) + the Copilot GitHub device-login **unreachable** from the menu (both gate on their provider being current). Those code paths are left intact (dead/gated) for rebase cleanliness. |
| 27 | `crates/client/src/telemetry.rs` | ~92–116, ~595–614 | **Route minidumps + telemetry events to our sub2api collector.** Two `LazyLock<Option<String>>` endpoint constants default to `aitoken.bigopen.cn/crashes/upload` + `aitoken.bigopen.cn/telemetry/events` (env vars `ZED_MINIDUMP_ENDPOINT` / `VIBEDEV_TELEMETRY_ENDPOINT` still override for staging). `build_telemetry_request` checks `TELEMETRY_EVENTS_ENDPOINT` first, falls back to `build_zed_api_url` only if explicitly unset. Server side: `handler/vibedev/telemetry.go` in `sub2api-vibedev` fork — `POST /telemetry/events` (JSONL) + `POST /crashes/upload` (Sentry minidump multipart). Couples to row #9 (telemetry defaults flipped back to ON). |
| 28 | `crates/agent_skills/agent_skills.rs` (`global_skills_dir`) | ~748 | Global skills dir `~/.agents/skills` → `~/.vibedev/skills` so the ccb ACP backend (reads CLAUDE_CONFIG_DIR/skills) sees frontend-installed skills. project-scope `.agents/skills` unchanged. Rebase: re-apply the one-line path swap. |

> Real conflict points on rebase: **#6** (`initialize_panels()` `futures::join!` —
> re-add the two `add_panel_when_ready(vibedev_*_panel, …)` lines at the end) and
> **#10** (the `agent_panel.rs` default-agent `.or_else` fallback + the
> VibeDev-replaces-"Zed Agent" picker entry, the `external_agents()` filter, and
> the kept "External Agents" header). Both are UI-logic edits; re-apply by
> grepping `VIBEDEV` in those files. **#25** also touches upstream
> `open_ai_compatible.rs` (append-only end-of-file fn) — re-apply by grepping
> `vibedev_configure_fim`; the rest of #25 is in the VibeDev-owned `vibedev_account`
> crate.

## 3. Rebase procedure (per Zed release)

```
git remote add upstream https://github.com/zed-industries/zed   # once
git fetch upstream --tags
git rebase <target-zed-tag>
```
1. **New crates** (`crates/vibedev_*`): never conflict.
2. **Conflicts only in §2 files.** Re-apply each row; then `git grep -n vibedev`
   across §2 files to confirm all 11 touches are present.
3. **#6** is the usual conflict — re-add the two panel lines at the end of the
   `futures::join!`.
4. **Build:** `cargo build -p zed`.
   - Needs MSVC `vcvars64.bat` env + CMake on PATH (Windows). **Do NOT set `RUSTFLAGS`.**
   - **Debug build reads `assets/settings/default.json` from disk at runtime** →
     a default.json change needs a **restart, not a recompile**.
5. **Verify:** VibeDev account + providers panels appear in the dock, the cost
   status item shows, and `agent_servers.VibeDev` is present in default.json
   (a fresh session spawns `bun … --acp` and responds).

## 4. Traps (do not relearn the hard way)

- **Never edit "VibeDev" via Zed's agent-settings UI.** It writes
  `agent_servers.VibeDev = { type: "registry" }` into the user's
  `settings.json`, which **overrides and breaks** the baked `type: "custom"` ACP
  registration. Keep the custom def in `default.json` only; if the agent
  "is not registered", check `~/AppData/Roaming/Zed/settings.json` for a stray
  `type:registry` override and delete it.
- **Kill the running `vibedev.exe` before any rebuild.** The link step overwrites
  `target/.../vibedev.exe`; a running instance holds an exclusive lock → `cargo
  build` fails with exit 101 "failed to remove vibedev.exe". (Renamed from `zed.exe`
  in row #22.)
- **Kill the stale `bun.exe` sidecar before relaunching Zed** after a backend
  `dist` rebuild — otherwise the new launch reuses the stale-dist sidecar.
- **Two `bun` processes on startup** = sidecar single-instance startup race
  (tracked: Phase 3 / audit §3.5). Not fatal; the agent (a separate per-session
  `bun … --acp` spawn) is unaffected.
- **Secrets never live here.** The gateway `api_key` + base URL are resolved at
  runtime from `~/.vibedev/auth.json` (written by the sidecar login flow), so
  this public fork ships no secret. `default.json` only carries non-secret env
  (`CLAUDE_CODE_USE_OPENAI`, `OPENAI_DEFAULT_*`) — `CLAUDE_CONFIG_DIR` is
  deliberately absent (the backend bottoms it to `~/.vibedev` in
  `ensureVibedevConfigDir`, so no user-name leak).

## 5. Productization TODO (Phase 5)

- `agent_servers.VibeDev.args` currently uses an absolute **dev** path to
  `dist/cli-bun.js`. Shipping must swap it for the bundled install location (and
  decide whether `bun` is bundled or PATH-resolved).
