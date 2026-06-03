//! Spawns and supervises the VibeDev backend sidecar.
//!
//! Per Plan 5's launcher contract:
//! - command: `<install_dir>/agent/vibedev-agent.exe --vibedev-sidecar`
//! - readiness: `~/.vibedev/endpoint.json` appears (<=5s) AND
//!   `GET http://127.0.0.1:<port>/vibedev/health` returns 200
//! - shutdown: kill on owner exit (the sidecar's own single-instance lock +
//!   refcount handle the multi-window case backend-side)
//!
//! Supervision restarts the child on crash with capped exponential backoff so a
//! crash loop can't spin the CPU or spam logs (Plan 5 degradation table).

use anyhow::Result;
use gpui::{App, AppContext as _, AsyncApp, BackgroundExecutor, Task};
use http_client::{AsyncBody, HttpClient, Request};
use std::future::Future;
use std::process::ExitStatus;
use std::sync::atomic::{AtomicU32, Ordering};
use std::{path::PathBuf, sync::Arc, time::Duration};

use crate::read_endpoint;

/// Dev path to the built agent binary. Productization replaces this with the
/// bundled install location; for now it's the checked-out backend dist build.
// Relative to the dev cwd (the zed repo root when run via `cargo run`); the
// sibling `claude-code-best` checkout holds the dev agent build. No absolute /
// user-specific path so this is clean for the public repo.
#[cfg(target_os = "windows")]
const DEV_AGENT_BIN: &str = r"..\claude-code-best\dist\vibedev-agent.exe";
#[cfg(not(target_os = "windows"))]
const DEV_AGENT_BIN: &str = "../claude-code-best/dist/vibedev-agent";

/// Max time to wait for the handshake file + health check after a spawn.
// VIBEDEV (macOS): the agent is a ~218 MB `bun --compile` binary; its FIRST
// launch on Apple Silicon is slow (~25-30s) while the kernel/AMFI scans the
// freshly-written ad-hoc-signed binary. 5s is right once warm and on Win/Linux,
// but too short for that cold start (the account panel flashes "backend not
// ready" on first run). `await_ready` polls and returns the instant the sidecar
// is up, so a larger ceiling only costs time on a genuine miss.
#[cfg(target_os = "macos")]
pub const SIDECAR_HEALTH_TIMEOUT: Duration = Duration::from_secs(45);
#[cfg(not(target_os = "macos"))]
pub const SIDECAR_HEALTH_TIMEOUT: Duration = Duration::from_secs(5);

const HEALTH_POLL_INTERVAL: Duration = Duration::from_millis(150);
const RESTART_BACKOFF_MIN: Duration = Duration::from_millis(500);
const RESTART_BACKOFF_MAX: Duration = Duration::from_secs(30);

/// Handle to the supervised sidecar. Dropping it cancels supervision; the
/// `kill_on_drop(true)` on the child process tears the sidecar down with us.
pub struct VibedevSidecar {
    _supervisor: Task<()>,
    // VIBEDEV (agent-OTA): the "apply now" channel. `request_restart` pushes a
    // unit here and the supervision loop races it against the child-exit wait
    // (see `supervise`), killing + respawning the sidecar so it re-launches from
    // the freshly hot-swapped `agent/` dir. Unbounded so the non-blocking send
    // never back-pressures; an enqueued-but-not-yet-observed signal is drained
    // on the next cycle so we don't double-restart.
    restart_tx: smol::channel::Sender<()>,
}

impl VibedevSidecar {
    /// Spawn + supervise the sidecar. Call once from `vibedev_ui::init`.
    ///
    /// Resolves the agent binary via `agent_program()` (env override
    /// `VIBEDEV_AGENT_BIN`, else bundled `<install_dir>/agent/vibedev-agent[.exe]`,
    /// else the dev dist build).
    pub fn spawn(cx: &mut App) -> Self {
        let executor = cx.background_executor().clone();
        let http = cx.http_client();
        // VIBEDEV: a FOREGROUND task (not `background_spawn`) so the supervisor can
        // hold an `AsyncApp` across awaits and hop onto the app to wire
        // edit-prediction (FIM) at the loopback sidecar once the handshake yields
        // the per-launch port + bearer token. `AsyncApp` is `!Send`, so it cannot
        // cross into a background task. The supervision work is all await points
        // (process spawn/wait, health polling), which yield cooperatively, so the
        // UI thread is not blocked.
        // VIBEDEV: shared holder for the live sidecar child PID so the app-quit
        // hook can tear it down. Windows does NOT kill child processes when the
        // parent exits, and `kill_on_drop(true)` only fires on a graceful Drop —
        // which Zed's `process::exit()` shutdown skips — so without this the bun
        // sidecar orphans on every IDE close (and keeps holding the loopback port
        // via its single-instance lock). See memory reference_zed_orphan_bun_windows.
        let child_pid = Arc::new(AtomicU32::new(0));
        // VIBEDEV (agent-OTA): unbounded so `request_restart`'s `try_send` never
        // blocks or fails on a full buffer. The Receiver moves into the
        // supervisor; the Sender is kept on the handle for `request_restart`.
        let (restart_tx, restart_rx) = smol::channel::unbounded::<()>();
        let supervisor = {
            let child_pid = child_pid.clone();
            cx.spawn(async move |cx| {
                supervise(executor, http, child_pid, restart_rx, cx).await;
            })
        };
        // Kill the sidecar's process tree on app quit (the normal window-close
        // path). A crash / Task-Manager kill of the IDE won't run on_app_quit;
        // covering that too would need a Windows Job Object — left as a follow-up.
        cx.on_app_quit(move |_cx| {
            let pid = child_pid.load(Ordering::SeqCst);
            async move {
                if pid != 0 {
                    kill_process_tree(pid);
                }
                // VIBEDEV (V2-PLAN-8 P3): also reap the `ssh -R` reverse-tunnel
                // children ensure_tunnel spawned. Same Windows orphan class as the
                // sidecar above (no process-tree reaping; process::exit skips Drop),
                // but those children live in remote_tunnel, not under this PID.
                crate::remote_tunnel::teardown_tunnels();
            }
        })
        .detach();
        Self {
            _supervisor: supervisor,
            restart_tx,
        }
    }

    /// VIBEDEV (agent-OTA): request that the running sidecar be restarted so it
    /// re-launches from the (just hot-swapped) bundled `agent/` files. This is
    /// the "apply now" trigger the agent-OTA flow calls after staging a new
    /// agent build on disk: the supervision loop observes this signal, kills the
    /// live child's process tree, and respawns from `agent_program()` (which
    /// re-reads `agent/`), with no restart backoff.
    ///
    /// Best-effort and non-blocking: a closed channel (app shutting down) or a
    /// send that races an already-in-flight restart is a harmless no-op — a
    /// restart is either coming or moot.
    pub fn request_restart(&self) {
        let _ = self.restart_tx.try_send(());
    }

    /// Relay a login-callback payload to the running sidecar. Exposed so a
    /// `vibedev://auth-callback` handler (or a manual/dev trigger) can forward
    /// the opaque base64 payload without needing direct access to the http
    /// client wiring. Returns the verified email if the sidecar reports one.
    ///
    /// TODO(Plan 4): wire the OS-level `vibedev://` scheme so this is driven
    /// automatically. On Windows `gpui::App::register_url_scheme` is currently
    /// unimplemented, so scheme registration is deferred to productization;
    /// until then this is the callable entry point for the relay.
    pub fn relay_login_callback(
        cx: &mut App,
        payload: String,
    ) -> Task<Result<Option<String>>> {
        let http = cx.http_client();
        cx.background_spawn(async move { crate::relay_login_callback(http, payload).await })
    }
}

/// VIBEDEV: re-run the inline-assistant chat-provider configuration on demand,
/// WITHOUT the supervision loop's port/model-count guard.
///
/// The supervisor parks on `child.status().await` while the sidecar stays up, so
/// it can't notice a 0 → N model change that happens mid-session (the common
/// case: the user signs in after launch). The account panel calls this from its
/// sign-in / refresh-models success paths to (re)register + (re)select the chat
/// model once the account's `available_models` are known. A not-signed-in or
/// transport failure is logged and left as a no-op (the launcher's guarded path
/// will still pick it up on the next sidecar restart).
pub async fn reconfigure_inline_assistant(http: Arc<dyn HttpClient>, cx: &AsyncApp) {
    let endpoint = match read_endpoint() {
        Ok(endpoint) => endpoint,
        Err(error) => {
            log::warn!(
                "vibedev inline assistant reconfigure skipped; endpoint unreadable: {error:#}"
            );
            return;
        }
    };
    let mut models = match crate::fetch_providers(http).await {
        Ok(providers) => providers.available_models,
        Err(error) => {
            log::info!(
                "vibedev inline assistant reconfigure: providers fetch failed ({error:#}); \
                 falling back to the cached model list"
            );
            Vec::new()
        }
    };
    // VIBEDEV: the sidecar's HTTP endpoints don't expose the account's model list
    // (they return an empty/absent available_models), so fetch_providers yields 0
    // models even though VibeDev already cached them. Reuse that cached list (the
    // same one the chat model picker uses) from ~/.vibedev/auth.json.
    if models.is_empty() {
        models = crate::read_cached_available_model_ids();
    }
    let port = endpoint.port;
    let token = endpoint.token;
    let model_count = models.len();
    let _ = cx.update(move |cx| {
        language_models::vibedev_configure_inline_assistant(cx, port, token, models);
    });
    log::info!(
        "vibedev inline assistant reconfigured after login: port {port} ({model_count} model(s))"
    );
}

/// VIBEDEV: the directory the running executable lives in — the install dir
/// where `VibeDev.exe` sits and the bundled `agent/` (plus a staged
/// `agent.new/`) sit next to it. `current_exe().parent()` is the same
/// resolution `agent_program()` / the agent-OTA swap all
/// need; factored here so there is one source of truth.
///
/// VIBEDEV (agent-OTA): `pub(crate)` so `remote_agent::bundled_agent_version`
/// can resolve `<install_dir>/agent/VERSION` (the version the remote is kept in
/// lockstep with) from the same single source of truth. `None` if the exe path
/// can't be resolved (extremely rare — e.g. the binary was deleted out from
/// under a running process).
pub(crate) fn install_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.to_path_buf()))
}

/// VIBEDEV (single-binary): resolve the compiled agent binary to spawn. The
/// agent is now a single self-contained executable (`bun build --compile`,
/// embeds the Bun runtime) — there is no separate `bun.exe` + `cli-bun.js`
/// entry script anymore.
///
/// Resolution: `VIBEDEV_AGENT_BIN` env override (dev/test) → bundled
/// `<install_dir>/agent/vibedev-agent[.exe]` (shipped by bundle-windows.ps1's
/// BuildAgent + the matching macOS/Linux bundlers; install_dir() resolves to
/// where VibeDev.exe lives, agent/ sits next to it) → dev-checkout dist build.
/// The dev fallback is wrong on every machine but this dev's, which is why the
/// bundled path is preferred whenever it exists.
fn agent_program() -> PathBuf {
    if let Some(path) = std::env::var_os("VIBEDEV_AGENT_BIN") {
        return PathBuf::from(path);
    }
    let bin_name = if cfg!(target_os = "windows") {
        "vibedev-agent.exe"
    } else {
        "vibedev-agent"
    };
    if let Some(dir) = install_dir() {
        let bundled = dir.join("agent").join(bin_name);
        if bundled.exists() {
            return bundled;
        }
    }
    PathBuf::from(DEV_AGENT_BIN)
}

/// VIBEDEV (agent-OTA): the "no-click / next-launch" swap path. Called once at
/// the very TOP of `supervise()`, BEFORE the sidecar entry is resolved or the
/// child is spawned, so no `agent/` file is locked by our own bun and the swap
/// is a pure rename (instant, atomic on the same volume) with zero interruption.
/// This mirrors the installer helper's swap-before-launch (see
/// `auto_update_helper/src/updater.rs` JOBS: move current → backup, move new →
/// current, delete backup) but needs no Restart Manager — our sidecar isn't up
/// yet, so the only handle that could hold `agent/` is *another* running
/// instance's bun, which is exactly the locked-rename fail-safe below.
///
/// Contract (the load-bearing safety invariant): this function NEVER panics and
/// NEVER leaves the install without a usable `agent/` on ANY path. On any error
/// it logs and leaves the existing `agent/` intact. `install_dir` is taken as a
/// param (not resolved via `current_exe()` inside) so it's unit-testable against
/// a tempdir.
///
/// Algorithm:
/// 1. Crash recovery: if `agent/` is gone but `agent.old/` survives (a prior
///    swap died between the two renames), restore `agent.old/` → `agent/` first.
/// 2. If no `agent.new/` is staged, there's nothing to do.
/// 3. Version gate: swap only if `agent.new/VERSION` parses as semver AND is
///    strictly newer than `agent/VERSION` (a missing/unparseable current VERSION
///    counts as "older", so the staged build wins).
/// 4. Swap with per-step rollback: move `agent/` → `agent.old/` (a locked-file
///    failure here is the key fail-safe — keep `agent/`, skip), move
///    `agent.new/` → `agent/` (rolling back the backup on failure), then delete
///    `agent.old/` best-effort.
///
/// MANUAL CHECK (on a packaged Windows build, since the unit tests above only
/// exercise the rename logic against a tempdir, not a real install + spawn):
///   1. Install/launch VibeDev once; confirm the backend comes up (panel shows
///      ready) and note `<install_dir>/agent/VERSION`.
///   2. Fully quit VibeDev (close all windows so no bun holds `agent/`).
///   3. Copy `<install_dir>/agent` → `<install_dir>/agent.new` and bump
///      `<install_dir>/agent.new/VERSION` to a higher semver (e.g. 1.0.0 →
///      1.0.1). Drop a sentinel file in `agent.new/` to eyeball the swap.
///   4. Relaunch. Expected: the log shows
///      `vibedev agent-OTA: applied staged agent update on launch (now 1.0.1)`,
///      `<install_dir>/agent/VERSION` now reads 1.0.1, the sentinel is present
///      under `agent/`, and both `agent.new/` and `agent.old/` are gone. The
///      sidecar comes up normally from the new dist.
///   5. Locked-file fail-safe: with a SECOND VibeDev instance still running
///      (holding `agent/`), repeat the stage + relaunch a third window; the new
///      window logs the "could not move agent/ -> agent.old/ ... keeping
///      current agent/" warning and still launches against the old dist (no
///      brick). The swap then succeeds on the next clean launch.
async fn apply_staged_agent_update(install_dir: &std::path::Path) {
    let staged = install_dir.join("agent.new");
    let agent = install_dir.join("agent");
    let backup = install_dir.join("agent.old");

    // 1. Crash recovery (safety): a previous swap that died after moving
    // `agent/` → `agent.old/` but before moving `agent.new/` → `agent/` would
    // leave us with NO `agent/` and a `agent.old/`. Restore it before doing
    // anything else so we never proceed (or return) without a usable `agent/`.
    if !agent.exists() && backup.exists() {
        match smol::fs::rename(&backup, &agent).await {
            Ok(()) => log::info!(
                "vibedev agent-OTA: recovered interrupted swap (restored agent.old/ -> agent/)"
            ),
            Err(error) => {
                // Couldn't restore; do NOT compound the damage by then moving a
                // staged dir into place over a half-state. Bail — the installer
                // helper's full-reinstall path can still repair this.
                log::warn!(
                    "vibedev agent-OTA: failed to recover interrupted swap (agent.old/ -> agent/): {error:#}; leaving install untouched"
                );
                return;
            }
        }
    }

    // 2. Nothing staged → nothing to do (the common path on every normal launch).
    if !staged.exists() {
        return;
    }

    // 3. Version gate. Read both VERSION files (async; a 48MB dir read would
    // block the UI thread, but a small VERSION file is cheap — still async for
    // uniformity). Swap only if the staged version parses AND it's strictly
    // newer than the current one. A missing/unparseable CURRENT version is
    // treated as "older" so a known-good staged build still wins; a
    // missing/unparseable STAGED version aborts (we won't swap in an unversioned
    // stage over a working agent).
    let staged_ver = match read_version(&staged.join("VERSION")).await {
        Some(ver) => ver,
        None => {
            log::warn!(
                "vibedev agent-OTA: staged agent.new/VERSION missing or unparseable; skipping swap"
            );
            return;
        }
    };
    let current_ver = read_version(&agent.join("VERSION")).await;
    let is_newer = match &current_ver {
        Some(current) => staged_ver > *current,
        // No usable current version → treat the staged build as newer.
        None => true,
    };
    if !is_newer {
        log::info!(
            "vibedev agent-OTA: staged agent not newer ({} <= {}), skipping",
            staged_ver,
            current_ver
                .as_ref()
                .map(|v| v.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        );
        // Leave the stale stage in place: deleting on a version-comparison
        // "not newer" is safe in principle, but the spec prefers not risking a
        // delete on any parse hiccup, so we don't touch it here.
        return;
    }

    // 3.5. Clear a stale agent.old/ left by an earlier swap whose cleanup (4c)
    // didn't complete. By here we've passed crash-recovery (step 1), so `agent/`
    // is the live source of truth and a surviving `agent.old/` is just debris —
    // but if we leave it, 4a's `rename(agent -> agent.old)` fails with
    // DirectoryNotEmpty and OTA stays wedged on EVERY future launch (the user
    // silently never updates). A failure to clear it is itself non-fatal: keep
    // `agent/`, skip this launch, retry next time.
    if backup.exists() {
        if let Err(error) = smol::fs::remove_dir_all(&backup).await {
            log::warn!(
                "vibedev agent-OTA: could not clear a stale agent.old/ before swap: {error:#}; keeping current agent/, will retry next launch"
            );
            return;
        }
        log::info!(
            "vibedev agent-OTA: cleared a stale agent.old/ left by an earlier interrupted swap"
        );
    }

    // 4. Swap with rollback.
    //
    // 4a. Move the current agent/ out of the way. If this fails — e.g. ANOTHER
    // running instance's bun still holds a handle under agent/ — this is the KEY
    // fail-safe: do NOT proceed. `agent/` stays exactly where it is and the
    // launch continues against the old (still-usable) dist. A locked agent must
    // never brick the backend. We retry the swap on the next launch.
    if agent.exists() {
        // VIBEDEV (agent-OTA Windows swap fix): on Windows a running process keeps a
        // file handle on the exe it launched, so a stray/orphan `vibedev-agent` from
        // a prior session (one that `kill_process_tree`'s PID-scoped kill never
        // caught) holds a handle under `agent/` and makes this rename fail with os
        // error 32 ("locked by another running instance") on EVERY launch — the swap
        // then never lands and OTA wedges forever. We only reach here when OUR OWN
        // sidecar is NOT live (next-launch: before the spawn loop; apply-now: after
        // the PID-kill + reap), so any `vibedev-agent` alive now is an orphan. Kill
        // them all by image name, then retry the rename a few times (Windows frees
        // handles lazily after a process dies). If it still won't budge, the
        // fail-safe keeps the current agent/ and we retry next launch (by when the
        // orphan we just killed is gone and its handle released).
        kill_stray_agents();
        let mut renamed = false;
        for attempt in 0..6u32 {
            match smol::fs::rename(&agent, &backup).await {
                Ok(()) => {
                    renamed = true;
                    break;
                }
                Err(error) => {
                    if attempt == 5 {
                        log::warn!(
                            "vibedev agent-OTA: could not move agent/ -> agent.old/ (likely locked by another running instance): {error:#}; keeping current agent/, will retry next launch"
                        );
                    } else {
                        smol::Timer::after(std::time::Duration::from_millis(250)).await;
                    }
                }
            }
        }
        if !renamed {
            return;
        }
    }

    // 4b. Move the staged build into place. If this fails, roll back: restore
    // the backup so we're left with the original working agent/ again.
    if let Err(error) = smol::fs::rename(&staged, &agent).await {
        log::warn!(
            "vibedev agent-OTA: failed to move agent.new/ -> agent/: {error:#}; rolling back"
        );
        if backup.exists() {
            match smol::fs::rename(&backup, &agent).await {
                Ok(()) => log::info!(
                    "vibedev agent-OTA: rolled back to previous agent/ after failed swap"
                ),
                Err(rollback_error) => log::warn!(
                    "vibedev agent-OTA: ROLLBACK FAILED restoring agent.old/ -> agent/: {rollback_error:#}; agent.old/ holds the previous dist"
                ),
            }
        }
        return;
    }

    // 4c. Delete the backup. Best-effort: the new agent/ is already live, so a
    // failure here only leaks an agent.old/ dir (cleaned up by the next
    // successful swap or a full reinstall) — it is NOT fatal and must not be
    // treated as a swap failure.
    if backup.exists() {
        if let Err(error) = smol::fs::remove_dir_all(&backup).await {
            log::warn!(
                "vibedev agent-OTA: swapped in new agent/ but failed to remove agent.old/: {error:#} (non-fatal; new agent is live)"
            );
        }
    }

    log::info!(
        "vibedev agent-OTA: applied staged agent update on launch (now {})",
        staged_ver
    );
}

/// VIBEDEV (agent-OTA): read + semver-parse a `VERSION` file. Returns `None` if
/// the file is absent, unreadable, or doesn't parse as semver (trimmed of
/// surrounding whitespace / a trailing newline). Async to keep the swap path off
/// any blocking file I/O on the UI thread.
async fn read_version(path: &std::path::Path) -> Option<semver::Version> {
    let contents = smol::fs::read_to_string(path).await.ok()?;
    semver::Version::parse(contents.trim()).ok()
}

/// VIBEDEV (agent-OTA): outcome of the supervisor's wait point — either the
/// child exited on its own (carrying its reaped status), or an "apply now"
/// restart was requested via `VibedevSidecar::request_restart`.
enum Wait {
    Exited(std::io::Result<ExitStatus>),
    RestartRequested,
}

/// VIBEDEV (agent-OTA): race the child-exit future against the restart signal.
///
/// Extracted as a free fn so the decision is unit-testable with dummy futures
/// and a `smol::channel` (no real process). `status_fut` must be the in-flight
/// `child.status()` future (which holds the only `&mut child` borrow); the
/// restart branch deliberately touches *only* `restart_rx`, so killing the
/// child happens by PID at the call site after this resolves — never via a
/// second borrow of `child` while `status_fut` is alive.
///
/// A closed `restart_rx` (Sender dropped — only at app teardown) can never win
/// the race over a live child, so it collapses to "wait for the child" rather
/// than spuriously reporting a restart.
async fn race_child_or_restart(
    status_fut: impl Future<Output = std::io::Result<ExitStatus>>,
    restart_rx: &smol::channel::Receiver<()>,
) -> Wait {
    smol::future::or(
        async { Wait::Exited(status_fut.await) },
        async {
            match restart_rx.recv().await {
                Ok(()) => Wait::RestartRequested,
                // Sender gone (app shutting down): never out-race a live child.
                Err(_) => std::future::pending().await,
            }
        },
    )
    .await
}

/// Supervision loop: spawn, wait for readiness, watch for exit, restart with
/// backoff. Returns only if the child can't be spawned at all (e.g. `bun`
/// missing), which is logged so the panel's "backend not ready" state explains
/// itself instead of silently retrying forever.
///
/// VIBEDEV (agent-OTA): the wait point also observes `restart_rx` so an
/// `request_restart` "apply now" forces an immediate (backoff-free) respawn from
/// the hot-swapped `agent/` dir.
async fn supervise(
    executor: BackgroundExecutor,
    http: Arc<dyn HttpClient>,
    child_pid: Arc<AtomicU32>,
    restart_rx: smol::channel::Receiver<()>,
    cx: &mut AsyncApp,
) {
    // VIBEDEV (agent-OTA): no-click / next-launch swap. BEFORE resolving the
    // sidecar entry or spawning bun, if a staged `agent.new/` is present and
    // newer, atomically swap it into `agent/` (our own sidecar isn't up yet, so
    // nothing we own holds the dir — the swap is a lock-free rename). On any
    // error this is a no-op that leaves the existing `agent/` intact, so the
    // launch below proceeds normally either way. `install_dir()` may be `None`
    // only if `current_exe()` can't resolve, in which case there's nothing to
    // swap and `agent_program()` falls back to its dev path regardless.
    if let Some(dir) = install_dir() {
        apply_staged_agent_update(&dir).await;
    }

    let agent_bin = agent_program();
    if !agent_bin.exists() {
        log::warn!(
            "vibedev agent binary not found at {}; backend features unavailable until built",
            agent_bin.display()
        );
        return;
    }

    // VIBEDEV: last loopback port we configured FIM against, so a crash-restart
    // that hands out the same port doesn't rewrite settings on every cycle.
    let mut configured_fim_port: Option<u16> = None;
    // VIBEDEV: same guard for the inline-assistant chat provider — but ALSO keyed
    // on the model count, so that once the user signs in (models go from 0 → N)
    // we re-run on the same port to actually select a model.
    let mut configured_inline_assistant: Option<(u16, usize)> = None;
    let mut backoff = RESTART_BACKOFF_MIN;
    loop {
        let mut child = match util::command::new_command(&agent_bin)
            .arg("--vibedev-sidecar")
            .kill_on_drop(true)
            .stdin(util::command::Stdio::null())
            .spawn()
        {
            Ok(child) => child,
            Err(error) => {
                log::error!("failed to spawn vibedev sidecar (vibedev-agent): {error:#}");
                return;
            }
        };
        // VIBEDEV: record the live PID so the on_app_quit hook can kill this child.
        child_pid.store(child.id(), Ordering::SeqCst);

        match await_ready(&executor, &http, SIDECAR_HEALTH_TIMEOUT).await {
            Ok(()) => {
                log::info!("vibedev sidecar is ready");
                backoff = RESTART_BACKOFF_MIN;
                // VIBEDEV: the sidecar is up and its handshake file is present, so
                // the loopback FIM endpoint is now reachable. Point Zed's
                // OpenAI-compatible edit-prediction provider at it, carrying the
                // per-launch bearer token. Done after every successful handshake
                // (the port/token rotate per launch) but skipped if the port is
                // unchanged from a prior cycle.
                configure_fim(cx, &mut configured_fim_port);
                // VIBEDEV: same idea for the inline assistant (内联助手) — register
                // an OpenAI-compatible CHAT provider at the loopback `/v1` and
                // select it as the inline-assistant + default model, so the inline
                // assistant has a configured provider instead of erroring
                // `NoProvider`. Models come from the sidecar (auth.json), so this
                // re-runs when the count changes (e.g. after the user signs in).
                configure_inline_assistant(&http, cx, &mut configured_inline_assistant)
                    .await;
            }
            Err(error) => {
                log::warn!("vibedev sidecar did not become ready: {error:#}");
            }
        }

        // Wait until the child exits OR an agent-OTA "apply now" restart is
        // requested, whichever comes first. `status()` takes &mut and reaps the
        // process on natural exit.
        // VIBEDEV (agent-OTA): kill via the PID (not the `child` handle) in the
        // restart branch — `race_child_or_restart` holds the only `&mut child`
        // borrow through the `child.status()` future, so re-borrowing `child`
        // here would not compile. After the race resolves the status future is
        // dropped, freeing `child` to be re-borrowed for the reap below.
        let status = match race_child_or_restart(child.status(), &restart_rx).await {
            Wait::Exited(status) => status,
            Wait::RestartRequested => {
                log::info!(
                    "vibedev sidecar restart requested (agent-OTA apply); killing + respawning from swapped agent/"
                );
                // Kill by PID via taskkill /T (whole tree): the bun child spawns
                // its own descendants, and `child.kill()` would only signal the
                // direct child, orphaning the grandchildren on Windows (see
                // reference_zed_orphan_bun_windows). The PID route is also what
                // satisfies the borrow note above.
                kill_process_tree(child_pid.load(Ordering::SeqCst));
                // Reap the just-killed child; returns promptly now that it's dead.
                let _ = child.status().await;
                // VIBEDEV: clear the PID before the quit-hook can act on a stale
                // (possibly reused) value.
                child_pid.store(0, Ordering::SeqCst);
                // VIBEDEV (agent-OTA): the child is now dead (its agent/ file handles are
                // released), so this is the safe window to apply a staged agent update before
                // respawning — this is what makes "apply now" actually swap. If nothing is
                // staged it's a no-op; if the swap can't proceed (e.g. still locked) the
                // fail-safe inside keeps the current agent/ and we respawn on that.
                if let Some(dir) = install_dir() {
                    apply_staged_agent_update(&dir).await;
                }
                // Coalesce any extra queued restart signals so a burst of
                // `request_restart` calls collapses into a single respawn.
                while restart_rx.try_recv().is_ok() {}
                // An intentional restart is not a crash: do NOT apply backoff.
                backoff = RESTART_BACKOFF_MIN;
                continue;
            }
        };
        // VIBEDEV: child is dead; clear the PID so the quit-hook can't later kill a
        // PID that has since been reused by an unrelated process.
        child_pid.store(0, Ordering::SeqCst);
        match status {
            Ok(status) if status.success() => {
                // VIBEDEV: child exited 0. Most likely the sidecar found another
                // instance already holding the single-instance lock + serving the
                // health endpoint, did its ref-count increment, and returned
                // (see services/vibedevSidecar/server.ts ~line 420 "an existing
                // instance is healthy — reusing it"). Without this branch the
                // supervisor would spin: spawn → "reuse" → exit 0 → restart →
                // "reuse" → exit 0 → ... forever, while the panel UI sees the
                // toggle and reports "Backend not ready" even though the sidecar
                // is up and serving. Instead, enter passive watch: poll the
                // health endpoint and only respawn after it goes dark (i.e. the
                // external owner died). Loopback HTTP, ~2s per poll, 30s cadence.
                log::info!(
                    "vibedev sidecar exited cleanly (existing owner serving the port); entering passive watch"
                );
                // VIBEDEV (agent-OTA): this passive-watch loop does NOT observe
                // restart_rx. "Apply now" only fires while our own sidecar is the
                // live owner (the common path above); a restart requested while we
                // are merely watching an external owner's instance is left queued
                // and consumed on the next supervision cycle (the race below, once
                // the external owner goes dark and we respawn).
                loop {
                    executor.timer(Duration::from_secs(30)).await;
                    if await_ready(&executor, &http, Duration::from_secs(3))
                        .await
                        .is_err()
                    {
                        log::warn!("vibedev sidecar owner gone; resuming supervision");
                        break;
                    }
                }
                backoff = RESTART_BACKOFF_MIN;
                continue;
            }
            Ok(status) => log::warn!("vibedev sidecar exited ({status}); restarting"),
            Err(error) => log::warn!("vibedev sidecar wait failed ({error:#}); restarting"),
        }

        executor.timer(backoff).await;
        backoff = (backoff * 2).min(RESTART_BACKOFF_MAX);
    }
}

/// Kill a process and its entire child tree by PID. Used by the app-quit hook to
/// tear the bun sidecar down on shutdown — Windows leaves children running when
/// the parent exits. Best-effort: an already-dead/missing PID is a no-op.
fn kill_process_tree(pid: u32) {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/T", "/PID"])
            .arg(pid.to_string())
            .creation_flags(CREATE_NO_WINDOW)
            .output();
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = std::process::Command::new("kill")
            .arg("-9")
            .arg(pid.to_string())
            .output();
    }
}

/// VIBEDEV (agent-OTA Windows swap fix): kill ALL `vibedev-agent` processes by
/// image name. `apply_staged_agent_update` only runs when our OWN sidecar is down,
/// so any `vibedev-agent` alive at swap time is an orphan from a prior session that
/// `kill_process_tree` (PID-scoped) didn't reap; on Windows it keeps a file handle
/// under `agent/` and blocks the swap rename forever. Best-effort; no-op if none
/// are running. Compiled out under `cfg(test)` so the filesystem unit tests never
/// reach out and kill a developer's live agent. Non-Windows doesn't lock running
/// exes against rename, so the swap there needs no kill.
fn kill_stray_agents() {
    #[cfg(all(target_os = "windows", not(test)))]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/T", "/IM", "vibedev-agent.exe"])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
    }
}

/// VIBEDEV: wire Zed's OpenAI-compatible edit-prediction (FIM) provider to the
/// sidecar's loopback completions endpoint, using the per-launch bearer token from
/// the handshake. Hops onto the app (settings + the key-state global both need
/// `&mut App`). No-op if the loopback port is unchanged from the last cycle.
fn configure_fim(cx: &AsyncApp, configured_port: &mut Option<u16>) {
    let endpoint = match read_endpoint() {
        Ok(endpoint) => endpoint,
        Err(error) => {
            log::warn!("vibedev FIM not configured; endpoint handshake unreadable: {error:#}");
            return;
        }
    };
    if *configured_port == Some(endpoint.port) {
        return;
    }
    let port = endpoint.port;
    let token = endpoint.token;
    // `update` takes the app lock and runs on the main thread; settings writes and
    // the key-state global both require `&mut App`. The supervisor task is owned by
    // the app-lifetime `GlobalVibedevSidecar`, so the app outlives this call.
    cx.update(move |cx| {
        edit_prediction::open_ai_compatible::vibedev_configure_fim(cx, port, token);
    });
    *configured_port = Some(port);
    log::info!("vibedev FIM edit-prediction pointed at loopback sidecar port {port}");
}

/// VIBEDEV: register an OpenAI-compatible CHAT provider in the
/// `LanguageModelRegistry` pointed at the sidecar's loopback `/v1`, and select it
/// as the inline-assistant + default model. The chat-capable model ids come from
/// the sidecar (`/vibedev/providers`, which the sidecar derives from
/// `~/.vibedev/auth.json`'s `available_models`).
///
/// Re-runs (on the same port) when the model count changes — i.e. once the user
/// signs in and models go from empty → populated — so a model actually gets
/// selected. Skipped when neither port nor model count changed, to avoid
/// rewriting settings on every supervision cycle.
async fn configure_inline_assistant(
    http: &Arc<dyn HttpClient>,
    cx: &AsyncApp,
    configured: &mut Option<(u16, usize)>,
) {
    let endpoint = match read_endpoint() {
        Ok(endpoint) => endpoint,
        Err(error) => {
            log::warn!(
                "vibedev inline assistant not configured; endpoint handshake unreadable: {error:#}"
            );
            return;
        }
    };

    // Pull the account's chat-capable model ids from the sidecar. A failure here
    // (e.g. not signed in yet) is fine: we register the provider + endpoint with
    // an empty model list now, and re-run later when the user has signed in.
    let mut models = match crate::fetch_providers(http.clone()).await {
        Ok(providers) => providers.available_models,
        Err(error) => {
            log::info!(
                "vibedev inline assistant: providers fetch failed ({error:#}); \
                 falling back to the cached model list"
            );
            Vec::new()
        }
    };
    // VIBEDEV: see read_cached_available_model_ids — the sidecar endpoints don't
    // expose the model list, so reuse VibeDev's cached auth.json list at launch.
    if models.is_empty() {
        models = crate::read_cached_available_model_ids();
    }

    // Guard: skip only when BOTH the port and the model count are unchanged. The
    // model-count component is what lets the post-sign-in cycle (0 → N models)
    // through to actually select a model.
    if *configured == Some((endpoint.port, models.len())) {
        return;
    }

    let port = endpoint.port;
    let token = endpoint.token;
    let model_count = models.len();
    cx.update(move |cx| {
        language_models::vibedev_configure_inline_assistant(cx, port, token, models);
    });
    *configured = Some((port, model_count));
    log::info!(
        "vibedev inline assistant pointed at loopback sidecar port {port} ({model_count} model(s))"
    );
}

/// Poll until the handshake file exists and `/vibedev/health` returns 200, or
/// the timeout elapses.
async fn await_ready(
    executor: &BackgroundExecutor,
    http: &Arc<dyn HttpClient>,
    timeout: Duration,
) -> Result<()> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if let Ok(endpoint) = read_endpoint()
            && health_ok(http, endpoint.port).await
        {
            return Ok(());
        }
        anyhow::ensure!(
            std::time::Instant::now() < deadline,
            "sidecar readiness timed out after {timeout:?}"
        );
        executor.timer(HEALTH_POLL_INTERVAL).await;
    }
}

/// Unauthenticated health ping (the contract's single-instance liveness check).
async fn health_ok(http: &Arc<dyn HttpClient>, port: u16) -> bool {
    let url = format!("http://127.0.0.1:{port}/vibedev/health");
    let Ok(request) = Request::get(url).body(AsyncBody::empty()) else {
        return false;
    };
    match http.send(request).await {
        Ok(response) => response.status().is_success(),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // VIBEDEV (agent-OTA): unit-test the wait-point decision in isolation, with
    // dummy child-exit futures + a real `smol::channel`, so no process is spawned.
    // Most cases assert on the `Wait` *variant* the race selects — the child-exit
    // future yields an `Err` here purely because `ExitStatus` has no portable
    // constructor; the supervisor treats `Ok`/`Err` identically at the call site
    // (both fall through to "child is dead, restart"). The closed-channel case
    // instead probes that the race *parks* (resolves to neither variant).

    /// A queued restart signal beats a child that never exits → RestartRequested.
    #[test]
    fn restart_wins_over_pending_child() {
        let (tx, rx) = smol::channel::unbounded::<()>();
        tx.try_send(()).expect("queue restart");
        let outcome = smol::block_on(race_child_or_restart(
            // Child that never exits on its own.
            std::future::pending::<std::io::Result<ExitStatus>>(),
            &rx,
        ));
        assert!(
            matches!(outcome, Wait::RestartRequested),
            "a queued restart signal must win over a never-exiting child"
        );
    }

    /// No restart queued + a child that exits → Exited (the natural-exit path).
    #[test]
    fn exit_wins_when_no_restart_queued() {
        let (_tx, rx) = smol::channel::unbounded::<()>();
        let outcome = smol::block_on(race_child_or_restart(
            // Child that has already exited (Err stands in for a reaped status).
            std::future::ready(Err(std::io::Error::other("exited"))),
            &rx,
        ));
        assert!(
            matches!(outcome, Wait::Exited(_)),
            "with no restart queued, the child-exit future must win"
        );
    }

    /// A *closed* restart channel (Sender dropped at app teardown) must NOT
    /// spuriously report a restart — otherwise the supervisor would spin in a
    /// tight kill/respawn loop on shutdown. The closed-channel arm maps `recv()`'s
    /// immediate `Err` to a parked `pending()`, so with a child that also never
    /// exits the race must STAY pending. We probe that by racing it against an
    /// immediately-ready `None`: the `None` only wins if the race is still
    /// pending. If the guard regressed to `Err` → RestartRequested, the race
    /// would resolve immediately and this test would fail.
    #[test]
    fn closed_channel_parks_instead_of_restarting() {
        let (tx, rx) = smol::channel::unbounded::<()>();
        drop(tx); // Sender gone → recv() resolves to Err immediately.
        let raced = race_child_or_restart(
            // Child never exits, so the closed-channel arm is the only future
            // that *could* resolve the race.
            std::future::pending::<std::io::Result<ExitStatus>>(),
            &rx,
        );
        let outcome = smol::block_on(smol::future::or(
            async { Some(raced.await) },
            async { None::<Wait> },
        ));
        assert!(
            outcome.is_none(),
            "a closed restart channel must park, never resolve the race"
        );
    }

    // VIBEDEV (agent-OTA): filesystem tests for `apply_staged_agent_update`.
    // Pure tempdir + rename tests — no process is spawned. Each writes a
    // `VERSION` file plus a `marker` file containing a unique tag so we can
    // assert WHICH dir won the swap (not just that a swap happened).

    use std::fs;
    use std::path::Path;

    /// Create `<root>/<name>/` containing `VERSION` (semver text, or `None` to
    /// omit it) and a `marker` file whose contents are `tag` (so a later read
    /// proves which dir's contents now live at the destination).
    fn make_agent_dir(root: &Path, name: &str, version: Option<&str>, tag: &str) {
        let dir = root.join(name);
        fs::create_dir_all(&dir).expect("create agent dir");
        if let Some(version) = version {
            fs::write(dir.join("VERSION"), version).expect("write VERSION");
        }
        fs::write(dir.join("marker"), tag).expect("write marker");
    }

    /// Read `<root>/<name>/marker`, or `None` if the dir/file is absent.
    fn read_marker(root: &Path, name: &str) -> Option<String> {
        fs::read_to_string(root.join(name).join("marker")).ok()
    }

    /// (a) Staged strictly newer → after the call, the new dist is at `agent/`
    /// (its marker won), and both `agent.new/` and `agent.old/` are gone.
    #[test]
    fn staged_newer_swaps_in_and_cleans_up() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        make_agent_dir(root, "agent", Some("1.0.0"), "OLD");
        make_agent_dir(root, "agent.new", Some("1.1.0"), "NEW");

        smol::block_on(apply_staged_agent_update(root));

        assert_eq!(
            read_marker(root, "agent").as_deref(),
            Some("NEW"),
            "the staged (newer) dist must now be at agent/"
        );
        assert!(
            !root.join("agent.new").exists(),
            "agent.new/ must be consumed by the swap"
        );
        assert!(
            !root.join("agent.old").exists(),
            "agent.old/ backup must be deleted after a successful swap"
        );
    }

    /// (b) Staged older → no swap: `agent/` keeps the old dist and the stale
    /// stage is left in place (we don't risk deleting on a version check).
    #[test]
    fn staged_older_does_not_swap() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        make_agent_dir(root, "agent", Some("2.0.0"), "CURRENT");
        make_agent_dir(root, "agent.new", Some("1.0.0"), "STAGED_OLD");

        smol::block_on(apply_staged_agent_update(root));

        assert_eq!(
            read_marker(root, "agent").as_deref(),
            Some("CURRENT"),
            "an older staged build must NOT replace the current agent/"
        );
        assert!(
            !root.join("agent.old").exists(),
            "no backup should be created when no swap happens"
        );
    }

    /// (b') Staged EQUAL → no swap (the gate is strictly-greater).
    #[test]
    fn staged_equal_does_not_swap() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        make_agent_dir(root, "agent", Some("1.2.3"), "CURRENT");
        make_agent_dir(root, "agent.new", Some("1.2.3"), "STAGED_EQUAL");

        smol::block_on(apply_staged_agent_update(root));

        assert_eq!(
            read_marker(root, "agent").as_deref(),
            Some("CURRENT"),
            "an equal-version staged build must NOT replace the current agent/"
        );
    }

    /// (c) No `agent.new/` staged → pure no-op; `agent/` is untouched.
    #[test]
    fn no_staged_dir_is_noop() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        make_agent_dir(root, "agent", Some("1.0.0"), "CURRENT");

        smol::block_on(apply_staged_agent_update(root));

        assert_eq!(
            read_marker(root, "agent").as_deref(),
            Some("CURRENT"),
            "with nothing staged, agent/ must be left exactly as-is"
        );
        assert!(!root.join("agent.old").exists());
    }

    /// (d) Crash recovery: `agent/` missing but `agent.old/` present (a swap
    /// died mid-flight) → `agent.old/` is restored to `agent/`. Here there is
    /// also no `agent.new/`, so recovery is the only action.
    #[test]
    fn crash_recovery_restores_backup() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        // No agent/ — only the leftover backup from an interrupted swap.
        make_agent_dir(root, "agent.old", Some("1.0.0"), "RECOVERED");

        smol::block_on(apply_staged_agent_update(root));

        assert_eq!(
            read_marker(root, "agent").as_deref(),
            Some("RECOVERED"),
            "a leftover agent.old/ must be restored to agent/ when agent/ is missing"
        );
        assert!(
            !root.join("agent.old").exists(),
            "agent.old/ must be consumed by the recovery rename"
        );
    }

    /// (d') Crash recovery THEN swap in the same call: `agent/` missing,
    /// `agent.old/` present (older), AND a newer `agent.new/` staged. Recovery
    /// restores the backup first, then the newer stage swaps in over it.
    #[test]
    fn crash_recovery_then_applies_newer_stage() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        make_agent_dir(root, "agent.old", Some("1.0.0"), "RECOVERED");
        make_agent_dir(root, "agent.new", Some("1.1.0"), "NEW");

        smol::block_on(apply_staged_agent_update(root));

        assert_eq!(
            read_marker(root, "agent").as_deref(),
            Some("NEW"),
            "after recovery, a newer staged build must still swap in"
        );
        assert!(!root.join("agent.new").exists());
        assert!(!root.join("agent.old").exists());
    }

    /// (e) Staged present but the CURRENT `agent/VERSION` is missing → the
    /// current dist is treated as "older", so the staged build swaps in.
    #[test]
    fn current_version_missing_treats_staged_as_newer() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        // agent/ exists but has NO VERSION file.
        make_agent_dir(root, "agent", None, "OLD_NO_VERSION");
        make_agent_dir(root, "agent.new", Some("1.0.0"), "NEW");

        smol::block_on(apply_staged_agent_update(root));

        assert_eq!(
            read_marker(root, "agent").as_deref(),
            Some("NEW"),
            "a missing current VERSION must let a parseable staged build win"
        );
        assert!(!root.join("agent.new").exists());
        assert!(!root.join("agent.old").exists());
    }

    /// (e') Inverse guard: the STAGED VERSION is missing/unparseable → abort,
    /// keep the current agent/ (never swap in an unversioned stage).
    #[test]
    fn staged_version_missing_aborts_swap() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        make_agent_dir(root, "agent", Some("1.0.0"), "CURRENT");
        // Staged dir exists but has NO VERSION file.
        make_agent_dir(root, "agent.new", None, "STAGED_NO_VERSION");

        smol::block_on(apply_staged_agent_update(root));

        assert_eq!(
            read_marker(root, "agent").as_deref(),
            Some("CURRENT"),
            "an unversioned staged build must NOT replace a versioned agent/"
        );
        assert!(
            root.join("agent.new").exists(),
            "the unparseable stage is left in place (no swap attempted)"
        );
        assert!(!root.join("agent.old").exists());
    }

    /// (f) Regression: a stale NON-EMPTY agent.old/ left by an earlier swap whose
    /// 4c cleanup never completed must NOT wedge OTA. With agent/ present, a newer
    /// agent.new/ staged, AND leftover agent.old/ debris, the swap must clear the
    /// debris and apply the new build — not silently refuse forever because
    /// `rename(agent -> agent.old)` keeps hitting DirectoryNotEmpty.
    #[test]
    fn stale_backup_is_cleared_then_swaps_in() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        make_agent_dir(root, "agent", Some("1.0.0"), "CURRENT");
        make_agent_dir(root, "agent.new", Some("1.1.0"), "NEW");
        // Debris from a prior swap whose 4c cleanup never completed.
        make_agent_dir(root, "agent.old", Some("0.9.0"), "STALE_DEBRIS");

        smol::block_on(apply_staged_agent_update(root));

        assert_eq!(
            read_marker(root, "agent").as_deref(),
            Some("NEW"),
            "a stale agent.old/ must not block applying a newer staged build"
        );
        assert!(
            !root.join("agent.new").exists(),
            "agent.new/ must be consumed by the swap"
        );
        assert!(
            !root.join("agent.old").exists(),
            "both the stale debris and the fresh backup must be gone after a clean swap"
        );
    }
}
