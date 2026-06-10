//! Opens the VibeDev "AgentApp" in a dedicated separate window: a real Zed
//! workspace configured into agent mode (no editor chrome), hosting a VibeDev
//! conversation in the center. (Phase 1: center conversation only; the left
//! thread sidebar and right panel container are follow-up phases.)

use std::sync::Arc;

use editor::Editor;
use gpui::{App, AppContext as _, Context, Focusable as _, Render, TaskExt as _, Window, WindowId};
use ui::prelude::*;
use workspace::{AppState, MultiWorkspace, OpenOptions, Workspace};

use super::vibedev_agent_home::VibedevAgentHome;

/// Registers the VibeDev window-switching action handlers on every workspace.
pub fn init(cx: &mut App) {
    cx.observe_new(|workspace: &mut Workspace, _window, _cx| {
        workspace.register_action(
            |workspace, _: &zed_actions::vibedev::OpenAgentAppWindow, window, cx| {
                // Already inside an AgentApp window: nothing to open or focus.
                if workspace.agent_mode {
                    return;
                }
                let app_state = workspace.app_state().clone();
                // The dispatching window is on the stack; pass its id so the
                // existing-window scan skips it (read_with on it would panic) —
                // same guard as `OpenIdeWindow` below.
                let current = window.window_handle().window_id();
                open_agent_app_window(app_state, Some(current), cx);
            },
        );
        workspace.register_action(
            |workspace, _: &zed_actions::vibedev::OpenIdeWindow, window, cx| {
                let app_state = workspace.app_state().clone();
                // The window dispatching this action is already on the stack;
                // read_with() on it panics ("attempted to read a window that is
                // already on the stack"). Pass its id so open_ide_window skips it.
                let current = window.window_handle().window_id();
                open_ide_window(app_state, Some(current), cx);
            },
        );
    })
    .detach();

    // Global fallbacks. The workspace-registered handlers above resolve along
    // the FOCUS path; on the projectless AgentApp landing page nothing useful
    // is focused (the workspace's focus delegate is the center pane, which the
    // agent layout does not render), so dispatches from the titlebar buttons
    // never reach them. gpui runs global bubble listeners only when no tree
    // handler consumed the action, so these cannot double-fire. Deferred so
    // the dispatching window is off the update stack before we read windows.
    cx.on_action(|_: &zed_actions::vibedev::OpenIdeWindow, cx| {
        cx.defer(|cx| {
            let app_state = AppState::global(cx);
            open_ide_window(app_state, None, cx);
        });
    });
    cx.on_action(|_: &zed_actions::vibedev::OpenAgentAppWindow, cx| {
        cx.defer(|cx| {
            let app_state = AppState::global(cx);
            open_agent_app_window(app_state, None, cx);
        });
    });
}

/// Focuses the existing VibeDev IDE window (a non-agent editor window), opening a
/// fresh one only if none exists. This is the inverse of `open_agent_app_window`:
/// from inside the AgentApp the user can jump (back) to the editor IDE without
/// ever stacking up duplicate empty editor windows.
///
/// "IDE window" means a `MultiWorkspace` whose `is_agent_app()` is `false`. The
/// default `workspace::open_new` path produces exactly that (only
/// `configure_agent_mode` flips the flag to `true`), so a freshly opened window
/// is guaranteed to be a non-agent IDE window.
pub fn open_ide_window(app_state: Arc<AppState>, skip_window: Option<WindowId>, cx: &mut App) {
    // Look for an already-open IDE (non-agent) window and just activate it.
    // Skip `skip_window` (the window that dispatched us): it is on the stack, so
    // read_with() on it would panic. `None` from deferred (off-stack) callers.
    let ide_window = cx
        .windows()
        .into_iter()
        .filter(|window| Some(window.window_id()) != skip_window)
        .filter_map(|window| window.downcast::<MultiWorkspace>())
        .find(|window| {
            window
                .read_with(cx, |multi_workspace, _cx| !multi_workspace.is_agent_app())
                .unwrap_or(false)
        });

    if let Some(ide_window) = ide_window {
        ide_window
            .update(cx, |_multi_workspace, window, _cx| {
                window.activate_window();
            })
            .ok();
        return;
    }

    // No IDE window exists: open a plain editor window. We intentionally do NOT
    // pass an agent init closure, so the new `MultiWorkspace` keeps the default
    // `is_agent_app() == false` and renders as the standard editor IDE.
    workspace::open_new(
        OpenOptions {
            open_mode: workspace::OpenMode::NewWindow,
            ..Default::default()
        },
        app_state,
        cx,
        |workspace, window, cx| {
            Editor::new_file(workspace, &Default::default(), window, cx);
        },
    )
    .detach_and_log_err(cx);
}

/// Focuses the existing VibeDev AgentApp window, opening a fresh one only if
/// none exists — the mirror of `open_ide_window`, so repeatedly clicking the
/// IDE titlebar button never stacks up duplicate AgentApp windows.
///
/// `skip_window` is the dispatching window (already on the stack — `read_with`
/// on it would panic); `None` at launch, when no window exists yet. For a new
/// window, the `init` closure runs against the freshly created workspace
/// before its first render, so we can flip it into agent mode and install the
/// conversation + panel without any editor flicker.
pub fn open_agent_app_window(
    app_state: Arc<AppState>,
    skip_window: Option<WindowId>,
    cx: &mut App,
) {
    let agent_window = cx
        .windows()
        .into_iter()
        .filter(|window| Some(window.window_id()) != skip_window)
        .filter_map(|window| window.downcast::<MultiWorkspace>())
        .find(|window| {
            window
                .read_with(cx, |multi_workspace, _cx| multi_workspace.is_agent_app())
                .unwrap_or(false)
        });

    if let Some(agent_window) = agent_window {
        agent_window
            .update(cx, |_multi_workspace, window, _cx| {
                window.activate_window();
            })
            .ok();
        return;
    }

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

    // Platform min/max/close for the custom agent titlebar. The agent layout
    // replaces the normal `titlebar_item` (and with it `PlatformTitleBar`), so
    // on Windows/Linux the window controls must be re-injected or the window
    // cannot be dragged or closed. Built here — not in `workspace` — because
    // the platform implementations live in `platform_title_bar`, which depends
    // on the `workspace` crate (importing it from `workspace` would be a
    // dependency cycle).
    let window_controls = cx.new(|_cx| AgentTitlebarWindowControls);
    workspace.set_agent_titlebar_window_controls(Some(window_controls.into()), cx);

    // A projectless workspace (fresh AgentApp launch / explicit AgentApp open)
    // cannot host a conversation: the `AgentPanel`, the threads sidebar, and
    // the right dock's file modules all need a project, so installing them here
    // produced a dead window of stacked "open a project" empty states. Show the
    // landing/home view instead and stop. Opening any project from it (or the
    // sidebar) goes through `MultiWorkspace::open_project`, which REPLACES this
    // empty workspace with the project's workspace — that one re-enters here
    // with a project and takes the normal panel + right-dock path below.
    if workspace
        .project()
        .read(cx)
        .visible_worktrees(cx)
        .next()
        .is_none()
    {
        let fs = workspace.app_state().fs.clone();
        let home = cx.new(|cx| VibedevAgentHome::new(fs, cx));
        // Focus the landing page. Action dispatch resolves along the FOCUS
        // path, and every `Workspace::register_action` handler hangs off the
        // MultiWorkspace-level root node — with nothing focused, titlebar
        // actions like "Open IDE" never reached it from this page.
        window.focus(&home.focus_handle(cx), cx);
        workspace.set_agent_center_view(Some(home.into()), cx);
        return;
    }

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
    // user-triggered *split* would create a *new* registered center pane (and
    // drift `last_active_center_pane` onto it), after which file-opens land on a
    // pane the dock never paints — showing nothing. So hide only the Split
    // affordance via `set_show_split_button(false)`, keeping New / Zoom /
    // navigation. Zoom is safe to keep: it does not create a new pane, and the
    // dock's File body renders a placeholder while this pane is zoomed (the
    // native `zoomed_overlay` paints it full-screen instead — avoiding a
    // double-render of the same entity). `center_pane` is an `Entity<Pane>`, so
    // updating it takes a lease on the *pane*, not on this `Workspace` — no
    // double-lease against the `&mut Context<Workspace>` held here.
    center_pane.update(cx, |pane, cx| {
        pane.set_show_split_button(false, cx);
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

/// The platform window controls (minimize / maximize / close) injected into the
/// AgentApp titlebar via `Workspace::set_agent_titlebar_window_controls`.
///
/// Windows: pure `WindowControlArea` hit-test caption buttons — the OS handles
/// clicks, hover snap layouts, and double-click natively, so no click handlers
/// are needed. Sized to the agent titlebar's 38px bar (the IDE titlebar's
/// controls use `platform_title_bar_height`, which is shorter). Linux renders
/// the client-side-decoration buttons only when the compositor doesn't draw
/// server-side decorations; macOS returns nothing (native traffic lights float
/// over the transparent titlebar).
struct AgentTitlebarWindowControls;

impl Render for AgentTitlebarWindowControls {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        match PlatformStyle::platform() {
            PlatformStyle::Windows => div().h_full().child(
                platform_title_bar::platforms::platform_windows::WindowsWindowControls::new(px(
                    38.,
                )),
            ),
            _ => div().h_full().children(
                platform_title_bar::render_right_window_controls(
                    cx.button_layout(),
                    Box::new(workspace::CloseWindow),
                    window,
                ),
            ),
        }
    }
}
