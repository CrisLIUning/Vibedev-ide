//! Opens the VibeDev "AgentApp" in a dedicated separate window: a real Zed
//! workspace configured into agent mode (no editor chrome), hosting a VibeDev
//! conversation in the center. (Phase 1: center conversation only; the left
//! thread sidebar and right panel container are follow-up phases.)

use std::sync::Arc;

use gpui::{App, AppContext as _, Context, TaskExt as _, Window};
use workspace::{AppState, MultiWorkspace, OpenOptions, Workspace};

/// Registers the `OpenAgentAppWindow` action handler on every workspace.
pub fn init(cx: &mut App) {
    cx.observe_new(|workspace: &mut Workspace, _window, _cx| {
        workspace.register_action(
            |workspace, _: &zed_actions::vibedev::OpenAgentAppWindow, _window, cx| {
                let app_state = workspace.app_state().clone();
                open_agent_app_window(app_state, cx);
            },
        );
    })
    .detach();
}

/// Opens a new OS window as the VibeDev AgentApp. The `init` closure runs against
/// the freshly created workspace before its first render, so we can flip it into
/// agent mode and install the conversation + panel without any editor flicker.
pub fn open_agent_app_window(app_state: Arc<AppState>, cx: &mut App) {
    workspace::open_new(
        OpenOptions {
            open_mode: workspace::OpenMode::NewWindow,
            ..Default::default()
        },
        app_state,
        cx,
        |workspace, window, cx| configure_agent_mode(workspace, window, cx),
    )
    .detach_and_log_err(cx);
}

/// Turns a fresh workspace window into the VibeDev AgentApp: a conversation
/// surface rather than an editor.
///
/// "照抄 + 改绘制" (reuse native logic, redraw the shell): we reuse two native
/// pieces and rearrange them into an agent workbench. (1) The native
/// project-group thread sidebar that every `MultiWorkspace` window already
/// registers (`Sidebar::new`, rendered by `MultiWorkspace::render` at the
/// multi-workspace level) — we just OPEN it. (2) The native per-workspace
/// `AgentPanel` conversation — we ensure it exists and inject it as the
/// workspace's *center* content via `set_agent_center_view`. Setting
/// `agent_mode` makes `Workspace::render` use `render_agent_layout`, which draws
/// a branded header + that center view and drops the editor pane group, status
/// bar, and side docks. The AgentPanel already defaults to the VibeDev
/// (`Agent::Custom`) agent, so the conversation/backend are unchanged; only the
/// window shell is redrawn.
fn configure_agent_mode(
    workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    // The initial workspace becomes an agent surface immediately.
    apply_agent_surface(workspace, window, cx);

    // Mark the window as an AgentApp and open the native project-group sidebar.
    // The `MultiWorkspace` wrapper does not exist yet (this runs inside
    // `Workspace::new`), and we must NOT hold a `&mut Workspace` lease while
    // touching it (`open_sidebar` -> `retain_active_workspace` reads the active
    // workspace and would double-lease), so defer and reach the wrapper through
    // the window root. The `agent_app` flag lets `zed`'s `ActiveWorkspaceChanged`
    // handler convert every workspace later activated in this window (e.g. via
    // "Open Project") into an agent surface too, so navigation never drops back
    // into the editor.
    let window_handle = window.window_handle();
    cx.defer(move |cx| {
        window_handle
            .update(cx, |_, window, cx| {
                if let Some(multi_workspace) = window.root::<MultiWorkspace>().flatten() {
                    multi_workspace.update(cx, |multi_workspace, cx| {
                        multi_workspace.set_agent_app(true);
                        multi_workspace.open_sidebar(cx);
                    });
                }
            })
            .ok();
    });
}

/// Renders `workspace` as an agent surface: flips it into `agent_mode` (so
/// `Workspace::render` uses the branded conversation layout instead of the
/// editor) and injects the native `AgentPanel` as the center conversation once
/// it has loaded. Shared by the initial AgentApp workspace and every workspace
/// later activated in the window, so navigating projects never drops back into
/// the editor IDE.
pub(crate) fn apply_agent_surface(
    workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    // Idempotent: the ActiveWorkspaceChanged handler may revisit an
    // already-converted workspace; don't re-register the open override or
    // re-ensure the panel.
    if workspace.agent_mode {
        return;
    }
    workspace.agent_mode = true;
    // Re-render into the agent layout now (drops the editor/docks immediately)
    // rather than waiting for the async panel injection below.
    cx.notify();
    // (Open Project is intercepted at the MultiWorkspace level — see
    // `MultiWorkspace::render` — so it opens in this AgentApp window rather than
    // routing through the global handler to the first editor window. A
    // workspace-scoped handler does not work here because the sidebar that hosts
    // the button lives at the MultiWorkspace level, outside the workspace's
    // dispatch path.)
    let ensure_panel = super::ensure_agent_panel_for_workspace(workspace, None, window, cx);
    cx.spawn_in(window, async move |workspace, cx| {
        ensure_panel.await?;
        workspace.update_in(cx, |workspace, _window, cx| {
            if let Some(panel) = workspace.panel::<agent_ui::AgentPanel>(cx) {
                workspace.set_agent_center_view(Some(panel.into()), cx);
            }
        })
    })
    .detach_and_log_err(cx);

    // Install the right panel host (stacked Files / Preview / Terminal modules).
    // We hold a `&mut Workspace` here, so building the host entity and setting
    // the view happen synchronously against this same workspace — no lease is
    // taken on the entity that is currently rendering. The host itself
    // lazy-loads its panels (spawned inside the host's own `Context`), so
    // nothing blocks here. The host is shown by default so the panels are
    // visible immediately; the titlebar toggle (see `render_agent_titlebar`)
    // can hide/show it.
    //
    // We hand the host this workspace's *center* `Pane` so its "Preview" module
    // can render it directly. In agent mode `render_agent_layout` draws the
    // injected center view rather than `self.center`, so this Pane is never
    // painted by the workspace — but it is still the open target for the project
    // panel, agent diffs, and tool file references. Rendering it inside the
    // right dock makes opened files and diffs visible without touching any of
    // those call sites. `active_pane()` is a plain field read, so no lease is
    // taken.
    let weak_workspace = workspace.weak_handle();
    let center_pane = workspace.active_pane().clone();
    // The right dock renders this center pane inside its fixed "File" module. A
    // user-triggered split or zoom would create a *new* registered center pane
    // (and drift `last_active_center_pane` onto it), after which file-opens land
    // on a pane the dock never paints — showing nothing. Defuse both entry
    // points on this pane: disable zoom, and clear the tab-bar buttons (which is
    // where the Split/Zoom/New affordances live; `set_can_split` only governs
    // drag-splitting, not these buttons). `center_pane` is an `Entity<Pane>`, so
    // updating it takes a lease on the *pane*, not on this `Workspace` — no
    // double-lease against the `&mut Context<Workspace>` held here.
    center_pane.update(cx, |pane, cx| {
        pane.set_can_toggle_zoom(false, cx);
        pane.set_render_tab_bar_buttons(cx, |_pane, _window, _cx| (None, None));
    });
    let project = workspace.project().clone();
    let right_dock = cx.new(|cx| super::vibedev_right_dock::VibedevRightDock::new(
        weak_workspace,
        center_pane,
        project,
        window,
        cx,
    ));
    // Register the dock's "Changes" pane as the agent-diff redirect target, so
    // agent diffs land in their own panel instead of the center "File" pane. We
    // hold `&mut Workspace` here, so this synchronous call against the same
    // workspace takes no extra lease; the pane is stored weakly.
    workspace.set_agent_changes_pane(Some(right_dock.read(cx).changes_pane().downgrade()), cx);
    workspace.set_agent_right_view(Some(right_dock.into()), cx);
}
