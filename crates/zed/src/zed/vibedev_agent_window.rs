//! Opens the VibeDev "AgentApp" in a dedicated separate window: a real Zed
//! workspace configured into agent mode (no editor chrome), hosting a VibeDev
//! conversation in the center. (Phase 1: center conversation only; the left
//! thread sidebar and right panel container are follow-up phases.)

use std::sync::Arc;

use gpui::{App, Context, TaskExt as _, Window};
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
    // Render the body as the agent workbench (branded header + injected center,
    // no editor/status bar). `render_agent_layout` keys off this flag.
    workspace.agent_mode = true;

    // Center: ensure the native AgentPanel exists for this workspace, then inject
    // it as the agent-mode center conversation surface once it has loaded.
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

    // Left: open the native project-group sidebar. The `MultiWorkspace` wrapper
    // does not exist yet (this runs inside `Workspace::new`), and we must NOT
    // hold a `&mut Workspace` lease while opening it (`open_sidebar` ->
    // `retain_active_workspace` reads the active workspace and would
    // double-lease), so defer and reach the wrapper through the window root.
    let window_handle = window.window_handle();
    cx.defer(move |cx| {
        window_handle
            .update(cx, |_, window, cx| {
                if let Some(multi_workspace) = window.root::<MultiWorkspace>().flatten() {
                    multi_workspace.update(cx, |multi_workspace, cx| {
                        multi_workspace.open_sidebar(cx);
                    });
                }
            })
            .ok();
    });
}
