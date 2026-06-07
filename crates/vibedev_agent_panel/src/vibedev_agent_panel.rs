use collections::HashMap;
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, ParentElement, Pixels,
    Render, Styled, WeakEntity, Window, actions,
};
use project::Project;
use ui::prelude::*;
use workspace::{
    Pane, PaneGroup, SplitMode, Workspace,
    dock::{DockPosition, Panel, PanelEvent},
    pane,
};

const VIBEDEV_AGENT_PANEL_KEY: &str = "VibedevAgentPanel";

actions!(
    vibedev_agent_panel,
    [
        /// Placeholder action dispatched when an empty tab bar in the panel is
        /// double-clicked. Carries no behavior yet (the panel hosts custom views
        /// rather than openable items), it only satisfies `Pane::new`.
        ToggleFocus
    ]
);

/// VibeDev "super panel": a workspace dock panel that hosts a `PaneGroup`
/// (mirroring `TerminalPanel`) so conversation views and custom VibeDev panels
/// can be split, dragged, zoomed, and (later) persisted inside the right dock.
///
/// This is the skeleton (T5): it stands up the panel + `PaneGroup` host + the
/// workspace registration. Populating panes with conversation content is a
/// follow-up (T8); for now the first pane is created empty.
pub struct VibedevAgentPanel {
    pub(crate) active_pane: Entity<Pane>,
    pub(crate) center: PaneGroup,
    workspace: WeakEntity<Workspace>,
    focus_handle: FocusHandle,
    active: bool,
}

/// Registers the panel's toggle action on every workspace, mirroring how
/// `vibedev_ui::init` registers `ToggleAccountFocus`. Called from `main.rs`
/// right after `vibedev_ui::init`.
pub fn init(cx: &mut App) {
    cx.observe_new(|workspace: &mut Workspace, _, _cx| {
        workspace.register_action(
            |workspace, _: &zed_actions::vibedev::ToggleAgentSurfaceFocus, window, cx| {
                workspace.toggle_panel_focus::<VibedevAgentPanel>(window, cx);
            },
        );
    })
    .detach();
}

impl VibedevAgentPanel {
    pub fn new(workspace: &Workspace, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let project = workspace.project().clone();
        let pane = new_vibedev_pane(workspace.weak_handle(), project, window, cx);
        let center = PaneGroup::new(pane.clone());
        Self {
            center,
            active_pane: pane,
            workspace: workspace.weak_handle(),
            focus_handle: cx.focus_handle(),
            active: false,
        }
    }

    /// Async loader matching `VibedevAccountPanel::load` / `TerminalPanel::load`,
    /// so `initialize_panels` can drive it through `add_panel_when_ready`.
    pub async fn load(
        workspace: WeakEntity<Workspace>,
        mut cx: gpui::AsyncWindowContext,
    ) -> anyhow::Result<Entity<Self>> {
        workspace.update_in(&mut cx, |workspace, window, cx| {
            cx.new(|cx| VibedevAgentPanel::new(workspace, window, cx))
        })
    }

    fn handle_pane_event(
        &mut self,
        pane: &Entity<Pane>,
        event: &pane::Event,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            pane::Event::Remove { focus_on_pane } => {
                let pane_count_before_removal = self.center.panes().len();
                let _removal_result = self.center.remove(pane, cx);
                if pane_count_before_removal == 1 {
                    self.center.first_pane().update(cx, |pane, cx| {
                        pane.set_zoomed(false, cx);
                    });
                    cx.emit(PanelEvent::Close);
                } else if let Some(focus_on_pane) =
                    focus_on_pane.as_ref().or_else(|| self.center.panes().pop())
                {
                    focus_on_pane.focus_handle(cx).focus(window, cx);
                }
            }
            pane::Event::ZoomIn => {
                for pane in self.center.panes() {
                    pane.update(cx, |pane, cx| {
                        pane.set_zoomed(true, cx);
                    })
                }
                cx.emit(PanelEvent::ZoomIn);
                cx.notify();
            }
            pane::Event::ZoomOut => {
                for pane in self.center.panes() {
                    pane.update(cx, |pane, cx| {
                        pane.set_zoomed(false, cx);
                    })
                }
                cx.emit(PanelEvent::ZoomOut);
                cx.notify();
            }
            &pane::Event::Split { direction, mode } => {
                // Only structural moves (drag an existing item into a split) are
                // wired here. ClonePane/EmptyPane need an item factory the panel
                // doesn't have yet (T8 supplies conversation views), so they no-op.
                if matches!(mode, SplitMode::MovePane) {
                    let Some(item) = pane.update(cx, |pane, cx| pane.take_active_item(window, cx))
                    else {
                        return;
                    };
                    let Ok(project) = self
                        .workspace
                        .update(cx, |workspace, _| workspace.project().clone())
                    else {
                        return;
                    };
                    let new_pane = new_vibedev_pane(self.workspace.clone(), project, window, cx);
                    new_pane.update(cx, |new_pane, cx| {
                        new_pane.add_item(item, true, true, None, window, cx);
                    });
                    self.center.split(pane, &new_pane, direction, cx);
                    window.focus(&new_pane.focus_handle(cx), cx);
                }
            }
            pane::Event::Focus => {
                self.active_pane = pane.clone();
            }
            _ => {}
        }
    }
}

/// Builds a `Pane` configured for hosting inside the super panel's `PaneGroup`,
/// mirroring `new_terminal_pane` but trimmed: no search bar / breadcrumbs /
/// custom split predicate (those belong to editor/terminal panes; the super
/// panel hosts custom views). Subscribes the panel to the pane's events.
fn new_vibedev_pane(
    workspace: WeakEntity<Workspace>,
    project: Entity<Project>,
    window: &mut Window,
    cx: &mut Context<VibedevAgentPanel>,
) -> Entity<Pane> {
    let pane = cx.new(|cx| {
        let mut pane = Pane::new(
            workspace.clone(),
            project,
            Default::default(),
            None,
            Box::new(ToggleFocus),
            false,
            window,
            cx,
        );
        pane.set_can_navigate(false, cx);
        pane.display_nav_history_buttons(None);
        pane.set_should_display_tab_bar(|_, _| true);
        pane.set_zoom_out_on_close(false);
        pane
    });

    cx.subscribe_in(&pane, window, VibedevAgentPanel::handle_pane_event)
        .detach();
    cx.observe(&pane, |_, _, cx| cx.notify()).detach();

    pane
}

impl Focusable for VibedevAgentPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<PanelEvent> for VibedevAgentPanel {}

impl Render for VibedevAgentPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.workspace
            .update(cx, |workspace, cx| {
                div().size_full().child(self.center.render(
                    workspace.zoomed_item(),
                    &workspace::PaneRenderContext {
                        follower_states: &HashMap::default(),
                        active_call: workspace.active_call(),
                        active_pane: &self.active_pane,
                        app_state: workspace.app_state(),
                        project: workspace.project(),
                        workspace: &workspace.weak_handle(),
                    },
                    window,
                    cx,
                ))
            })
            .ok()
            .unwrap_or_else(|| div().size_full())
    }
}

impl Panel for VibedevAgentPanel {
    fn persistent_name() -> &'static str {
        "Vibedev Agent Panel"
    }

    fn panel_key() -> &'static str {
        VIBEDEV_AGENT_PANEL_KEY
    }

    fn position(&self, _window: &Window, _cx: &App) -> DockPosition {
        DockPosition::Right
    }

    fn position_is_valid(&self, position: DockPosition) -> bool {
        matches!(position, DockPosition::Left | DockPosition::Right)
    }

    fn set_position(
        &mut self,
        _position: DockPosition,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        // Dock position persistence is deferred (matches VibedevAccountPanel);
        // the panel defaults to the right dock and can be dragged within a session.
    }

    fn default_size(&self, _window: &Window, _cx: &App) -> Pixels {
        px(640.)
    }

    fn icon(&self, _window: &Window, _cx: &App) -> Option<ui::IconName> {
        Some(ui::IconName::ZedAssistant)
    }

    fn icon_tooltip(&self, _window: &Window, _cx: &App) -> Option<&'static str> {
        Some("VibeDev Agent")
    }

    fn toggle_action(&self) -> Box<dyn gpui::Action> {
        Box::new(zed_actions::vibedev::ToggleAgentSurfaceFocus)
    }

    fn activation_priority(&self) -> u32 {
        10
    }

    fn is_zoomed(&self, _window: &Window, cx: &App) -> bool {
        self.active_pane.read(cx).is_zoomed()
    }

    fn set_zoomed(&mut self, zoomed: bool, _window: &mut Window, cx: &mut Context<Self>) {
        for pane in self.center.panes() {
            pane.update(cx, |pane, cx| {
                pane.set_zoomed(zoomed, cx);
            })
        }
        cx.notify();
    }

    fn set_active(&mut self, active: bool, _window: &mut Window, _cx: &mut Context<Self>) {
        self.active = active;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fs::FakeFs;
    use gpui::TestAppContext;
    use settings::SettingsStore;

    fn init_test(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let settings_store = SettingsStore::test(cx);
            cx.set_global(settings_store);
            theme_settings::init(theme::LoadThemes::JustBase, cx);
        });
    }

    // Mirrors the account_panel registration test: assert the panel is actually
    // registered into the workspace, guarding against "compiles but never wired
    // into the dock". `load` is a thin `update_in`/`cx.new` wrapper around `new`,
    // so constructing via `new` + `add_panel` exercises the same registration
    // path that `initialize_panels` drives.
    #[gpui::test]
    async fn vibedev_agent_panel_loads_and_registers(cx: &mut TestAppContext) {
        init_test(cx);

        let fs = FakeFs::new(cx.executor());
        let project = project::Project::test(fs, [], cx).await;
        let (workspace, cx) =
            cx.add_window_view(|window, cx| Workspace::test_new(project.clone(), window, cx));

        workspace.update_in(cx, |workspace, window, cx| {
            let panel = cx.new(|cx| VibedevAgentPanel::new(workspace, window, cx));
            workspace.add_panel(panel, window, cx);
        });

        workspace.read_with(cx, |workspace, cx| {
            assert!(
                workspace.panel::<VibedevAgentPanel>(cx).is_some(),
                "VibedevAgentPanel should be registered into the workspace"
            );
        });
    }
}
