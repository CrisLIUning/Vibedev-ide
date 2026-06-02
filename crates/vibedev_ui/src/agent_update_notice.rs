//! VIBEDEV (agent-OTA): the corner notice that appears when the auto-updater has
//! staged a new backend ("agent") build, plus its "立即应用" (apply-now) action.
//!
//! Wiring (see [`init`]): `AutoUpdater` publishes a freshly-staged `agent.new/`
//! version via `cx.notify()` (its `staged_agent_version` field). We `cx.observe`
//! that entity and, when the staged version becomes `Some(v)` for a version we
//! haven't yet announced, show a `MessageNotification` mirroring the app-update
//! notice in `auto_update_ui::show_update_notification`. The notice's primary
//! action calls `request_sidecar_restart`, which drives the launcher's
//! kill → apply-staged-agent → respawn sequence (so the swap happens while the
//! sidecar is down — the agent/ file handles are released).
//!
//! Caveat encoded in the copy ("会中断当前会话"): the ACP agent session is
//! per-session, so "apply now" restarts the sidecar but an in-flight ACP turn
//! keeps the old agent until its next turn/session. The no-click path is the
//! launcher's next-launch swap — hence "否则下次重启自动应用".

use auto_update::AutoUpdater;
use gpui::{App, AppContext as _, BorrowAppContext as _, DismissEvent, Subscription};
use semver::Version;
use workspace::notifications::{
    NotificationId, show_app_notification, simple_message_notification::MessageNotification,
};

use crate::request_sidecar_restart;

/// Unique key for the agent-update notice so repeated shows replace (not stack)
/// and a dismiss in one workspace clears it everywhere.
struct AgentUpdateNotification;

/// VIBEDEV (agent-OTA): holds the `AutoUpdater` observation alive for the app
/// lifetime AND the dedupe state. Without keeping the `Subscription`, the
/// observe callback would never fire; without the last-shown version, every
/// `AutoUpdater::notify` (each poll, plus unrelated status changes) would
/// re-show the notice — this gates it to once per newly-staged version.
struct GlobalAgentUpdateNotice {
    _observe: Subscription,
    last_shown: Option<Version>,
}

impl gpui::Global for GlobalAgentUpdateNotice {}

/// Wire the agent-update notice. Called from `vibedev_ui::init` (which runs after
/// `auto_update::init` has registered the `GlobalAutoUpdate`, so the updater
/// entity is available here). No-op if there is no `AutoUpdater` (the global is
/// always registered in this build, but guard anyway for tests / robustness).
pub(crate) fn init(cx: &mut App) {
    let Some(updater) = AutoUpdater::get(cx) else {
        return;
    };

    // An agent build may already be staged before this observe is installed
    // (the poll runs after launch, but a same-process re-init or a fast poll
    // could land first); surface it once up front, then let the observe handle
    // every subsequent stage.
    let already_staged = updater.read(cx).staged_agent_version().cloned();

    let observe = cx.observe(&updater, |updater, cx| {
        let staged = updater.read(cx).staged_agent_version().cloned();
        maybe_show_agent_update_notice(staged, cx);
    });

    cx.set_global(GlobalAgentUpdateNotice {
        _observe: observe,
        last_shown: None,
    });

    if already_staged.is_some() {
        maybe_show_agent_update_notice(already_staged, cx);
    }
}

/// VIBEDEV (agent-OTA): the dedupe decision — show a notice for `staged` iff it's
/// a version we haven't announced yet. Pure + free-standing so the unit tests
/// exercise the REAL gate (the surrounding `maybe_show_agent_update_notice` reads
/// `last_shown` from the `GlobalAgentUpdateNotice` global, which needs an `App`).
/// `None` staged → never show.
fn should_show(staged: Option<&Version>, last_shown: Option<&Version>) -> bool {
    match staged {
        None => false,
        Some(v) => last_shown != Some(v),
    }
}

/// Show the agent-update notice for `staged` iff it's a version we haven't shown
/// before. Dedupe is keyed on the exact staged version, so the notice fires once
/// when a new agent build is staged and never re-fires on subsequent
/// `AutoUpdater` notifications for the same version (avoiding per-poll spam).
fn maybe_show_agent_update_notice(staged: Option<Version>, cx: &mut App) {
    let Some(version) = staged else {
        return;
    };

    // Dedupe against the last-announced version via the (unit-tested) `should_show`
    // gate. `has_global` is a robustness guard; in production this is only reached
    // after `init` set the global.
    if !cx.has_global::<GlobalAgentUpdateNotice>() {
        return;
    }
    let last_shown = cx.global::<GlobalAgentUpdateNotice>().last_shown.clone();
    if !should_show(Some(&version), last_shown.as_ref()) {
        return;
    }
    cx.update_global::<GlobalAgentUpdateNotice, _>(|state, _| {
        state.last_shown = Some(version.clone());
    });

    show_app_notification(
        NotificationId::unique::<AgentUpdateNotification>(),
        cx,
        move |cx| {
            cx.new(|cx| {
                MessageNotification::new(
                    "后端有更新 · 立即应用(会中断当前会话),否则下次重启自动应用",
                    cx,
                )
                .primary_message("立即应用")
                .primary_on_click(move |_window, cx| {
                    // VIBEDEV (agent-OTA): drive launcher's kill → apply-staged →
                    // respawn. The running ACP session keeps the old agent until
                    // its next turn (hence the "会中断当前会话" copy); the swap
                    // itself happens while the sidecar is down.
                    request_sidecar_restart(cx);
                    cx.emit(DismissEvent);
                })
                .show_suppress_button(false)
            })
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    // VIBEDEV (agent-OTA): the notice UI + the sidecar restart are NOT
    // unit-testable here (gpui app + a real bun process); they're covered by
    // `cargo check` + a later joint real-machine test. What IS pure logic — and
    // the load-bearing correctness property — is the dedupe decision: show only
    // when the staged version differs from the last-shown one. These tests
    // exercise the REAL `should_show` gate that `maybe_show_agent_update_notice`
    // calls (imported via `use super::*`).

    #[test]
    fn no_staged_version_never_shows() {
        assert!(!should_show(None, None));
        assert!(!should_show(None, Some(&Version::new(1, 0, 0))));
    }

    #[test]
    fn first_staged_version_shows() {
        assert!(should_show(Some(&Version::new(1, 1, 0)), None));
    }

    #[test]
    fn same_version_does_not_reshow() {
        let v = Version::new(1, 1, 0);
        // Simulates a second AutoUpdater::notify (e.g. another poll) for the
        // same staged build — must NOT re-show.
        assert!(!should_show(Some(&v), Some(&v)));
    }

    #[test]
    fn newer_staged_version_shows_again() {
        let prev = Version::new(1, 1, 0);
        let next = Version::new(1, 2, 0);
        assert!(should_show(Some(&next), Some(&prev)));
    }
}
