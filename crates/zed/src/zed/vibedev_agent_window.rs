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

/// Turns a fresh workspace window into the VibeDev AgentApp.
///
/// Phase 1 ("照抄" / reuse-native): instead of a bespoke agent layout, we reuse
/// the native project-group thread sidebar that every `MultiWorkspace` window
/// already registers (see `MultiWorkspace::new` / `Sidebar::new` in the workspace
/// crate) together with its per-workspace `AgentPanel`. We only have to OPEN the
/// sidebar so the window lands on the agent experience; clicking a thread or "new
/// conversation" in the sidebar drives the native `AgentPanel`, which already
/// defaults to the VibeDev (`Agent::Custom`) agent and reuses the same
/// `ConversationView`. The earlier bespoke layer (`agent_mode` + the custom
/// `render_agent_layout` / `VibedevSessionSidebar` / center conversation) is left
/// intact but inert (never activated), to be reintroduced as branding ("改绘制")
/// in a later phase.
///
/// The `MultiWorkspace` wrapper does not exist yet at this point — this runs as
/// the `open_new` init callback, inside `Workspace::new`, before
/// `MultiWorkspace::new` wraps the workspace and calls `set_multi_workspace`. So
/// defer opening the sidebar until the wrapper (and its registered sidebar) exist.
fn configure_agent_mode(
    _workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    // Reach the MultiWorkspace through the window root, NOT by leasing the
    // Workspace (e.g. `defer_in` hands back `&mut Workspace`): `open_sidebar` ->
    // `retain_active_workspace` reads the active Workspace, which would
    // double-lease the very workspace the closure holds and panic. An App-level
    // `defer` keeps the workspace unleased while the sidebar opens (mirrors the
    // production sidebar setup in `zed::initialize_workspace`).
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
