//! The VibeDev AgentApp right-panel host.
//!
//! Phase 2 of the AgentApp surface: a tabbed container rendered in the
//! workspace's `agent_right_view` slot (see `render_agent_layout` in the
//! `workspace` crate). It currently hosts the native file tree
//! (`ProjectPanel`) and terminal (`TerminalPanel`), with room for a future
//! "Changes" (diff) tab.
//!
//! The AgentApp window skips `initialize_panels` (see `zed.rs`), so neither
//! panel is loaded into a dock. Instead this host *owns* each panel as a
//! standalone `Entity<_>` and renders it directly: both `ProjectPanel` and
//! `TerminalPanel` implement `Render`, so they can be used as elements without
//! going through the (disabled) dock machinery. Panels are lazy-loaded: the
//! file tree is requested on construction (it is the default tab) and the
//! terminal is requested the first time its tab is activated.

use gpui::{Entity, FocusHandle, Focusable, WeakEntity};
use project_panel::ProjectPanel;
use terminal_view::terminal_panel::TerminalPanel;
use ui::prelude::*;
use workspace::Workspace;

/// Which panel the right dock is currently showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RightTab {
    Files,
    Terminal,
}

/// Tabbed host for the AgentApp right panel. Owns its panels directly rather
/// than relying on the workspace dock (which is disabled in agent mode).
pub struct VibedevRightDock {
    workspace: WeakEntity<Workspace>,
    project_panel: Option<Entity<ProjectPanel>>,
    terminal_panel: Option<Entity<TerminalPanel>>,
    active_tab: RightTab,
    /// Set once a `TerminalPanel::load` has been kicked off, so re-activating
    /// the Terminal tab does not spawn a second load.
    terminal_load_started: bool,
    focus_handle: FocusHandle,
}

impl VibedevRightDock {
    pub fn new(
        workspace: WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self {
            workspace,
            project_panel: None,
            terminal_panel: None,
            active_tab: RightTab::Files,
            terminal_load_started: false,
            focus_handle: cx.focus_handle(),
        };
        // Files is the default tab, so eagerly load the project panel.
        this.load_project_panel(window, cx);
        this
    }

    /// Lazily loads the file tree. `ProjectPanel::load` consumes an
    /// `AsyncWindowContext` (which `spawn_in` provides as the inner `cx`) and
    /// returns a `Task`; we `await` that task *inside* the spawned closure and
    /// store the resulting entity back into this host from the host's own
    /// `WeakEntity` (no workspace lease taken here). The `await` never runs in
    /// `render`/`update`.
    fn load_project_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.project_panel.is_some() {
            return;
        }
        let workspace = self.workspace.clone();
        cx.spawn_in(window, async move |this, cx| {
            let panel = ProjectPanel::load(workspace, cx.clone()).await?;
            this.update(cx, |this, cx| {
                this.project_panel = Some(panel);
                cx.notify();
            })
        })
        .detach_and_log_err(cx);
    }

    /// Lazily loads the terminal. Same pattern as `load_project_panel`.
    fn load_terminal_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.terminal_load_started {
            return;
        }
        self.terminal_load_started = true;
        let workspace = self.workspace.clone();
        cx.spawn_in(window, async move |this, cx| {
            let panel = TerminalPanel::load(workspace, cx.clone()).await?;
            this.update(cx, |this, cx| {
                this.terminal_panel = Some(panel);
                cx.notify();
            })
        })
        .detach_and_log_err(cx);
    }

    fn activate_tab(&mut self, tab: RightTab, window: &mut Window, cx: &mut Context<Self>) {
        if self.active_tab == tab {
            return;
        }
        self.active_tab = tab;
        match tab {
            RightTab::Files => self.load_project_panel(window, cx),
            RightTab::Terminal => self.load_terminal_panel(window, cx),
        }
        cx.notify();
    }

    fn render_tab(
        &self,
        tab: RightTab,
        label: &'static str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let selected = self.active_tab == tab;
        let colors = cx.theme().colors();
        let (text_color, border_color) = if selected {
            (Color::Default, colors.text_accent)
        } else {
            (Color::Muted, colors.border_transparent)
        };
        let element_id = match tab {
            RightTab::Files => "vibedev-right-tab-files",
            RightTab::Terminal => "vibedev-right-tab-terminal",
        };
        let hover_bg = colors.element_hover;
        div()
            .id(element_id)
            .px_3()
            .py_1p5()
            .cursor_pointer()
            .border_b_2()
            .border_color(border_color)
            .hover(|style| style.bg(hover_bg))
            .child(Label::new(label).color(text_color).size(LabelSize::Small))
            .on_click(cx.listener(move |this, _event, window, cx| {
                this.activate_tab(tab, window, cx);
            }))
    }
}

impl Focusable for VibedevRightDock {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for VibedevRightDock {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        let tab_bar_bg = colors.tab_bar_background;
        let border = colors.border;

        // Content for the active tab. Each owned panel implements `Render`, so
        // it can be cloned in directly as a child. While a panel is still
        // loading we show a lightweight placeholder.
        let content = match self.active_tab {
            RightTab::Files => self
                .project_panel
                .clone()
                .map(IntoElement::into_any_element),
            RightTab::Terminal => self
                .terminal_panel
                .clone()
                .map(IntoElement::into_any_element),
        }
        .unwrap_or_else(|| {
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child(Label::new("Loading…").color(Color::Muted))
                .into_any_element()
        });

        v_flex()
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(colors.panel_background)
            .child(
                h_flex()
                    .flex_none()
                    .w_full()
                    .h(px(34.))
                    .px_1()
                    .gap_1()
                    .bg(tab_bar_bg)
                    .border_b_1()
                    .border_color(border)
                    .child(self.render_tab(RightTab::Files, "Files", cx))
                    .child(self.render_tab(RightTab::Terminal, "Terminal", cx)),
            )
            .child(
                div()
                    .flex_1()
                    .w_full()
                    .min_h_0()
                    .overflow_hidden()
                    .child(content),
            )
    }
}
