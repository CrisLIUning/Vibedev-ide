//! Opens the VibeDev "AgentApp" in a dedicated separate window: a real Zed
//! workspace configured into agent mode (no editor chrome), hosting a VibeDev
//! conversation in the center. (Phase 1: center conversation only; the left
//! thread sidebar and right panel container are follow-up phases.)

use std::sync::Arc;

use gpui::{App, AppContext as _, Context, TaskExt as _, Window};
use vibedev_agent_panel::VibedevConversationItem;
use workspace::{AppState, OpenOptions, Workspace};

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

/// Turns a fresh workspace window into the agent workbench (runs before render).
fn configure_agent_mode(
    workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    workspace.agent_mode = true;
    workspace.centered_layout = true;

    // Hide the project tree: the AgentApp window is a conversation surface, not a
    // file editor, so the left dock starts closed.
    workspace.left_dock().update(cx, |dock, cx| {
        dock.set_open(false, window, cx);
    });

    // Center: the reused VibeDev conversation. Because this is a real workspace
    // (not a stub), `ConversationView` is fully reusable here and does not panic.
    if let Some(thread_store) = agent::ThreadStore::try_global(cx) {
        let project = workspace.project().clone();
        let fs = workspace.app_state().fs.clone();
        let connection_store =
            cx.new(|cx| agent_ui::AgentConnectionStore::new(project.clone(), cx));
        let conversation = agent_ui::create_conversation_view(
            workspace.weak_handle(),
            project,
            connection_store,
            fs,
            thread_store,
            agent_ui::Agent::Custom {
                id: project::AgentId::new("VibeDev"),
            },
            None,
            None,
            None,
            None,
            Some("VibeDev".into()),
            None,
            agent_ui::AgentThreadSource::Sidebar,
            window,
            cx,
        );
        let item = cx.new(|cx| VibedevConversationItem::new(conversation, cx));
        workspace.add_item_to_center(Box::new(item), window, cx);
    }

    // Strip editor chrome from the center pane: in the AgentApp window the
    // conversation is the page itself, not a closeable editor tab. Hiding the tab
    // strip removes the tab label, the new-tab "+", split, and zoom buttons in one
    // gate (the pane renders the whole strip behind `should_display_tab_bar`).
    let center_pane = workspace.active_pane().clone();
    center_pane.update(cx, |pane, cx| {
        pane.set_should_display_tab_bar(|_, _| false);
        cx.notify();
    });
}
