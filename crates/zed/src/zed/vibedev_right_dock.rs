//! The VibeDev AgentApp right-panel host.
//!
//! Phase 2 of the AgentApp surface: a *stacked* container rendered in the
//! workspace's `agent_right_view` slot (see `render_agent_layout` in the
//! `workspace` crate). Rather than a single tab being visible at a time, every
//! enabled module is stacked vertically and each can be collapsed or closed;
//! a header "panels" menu re-enables hidden modules.
//!
//! The three modules are:
//!   * `Files`    — the native file tree (`ProjectPanel`).
//!   * `File`  — the workspace's *center* `Pane`. In agent mode
//!     `render_agent_layout` draws the injected center conversation rather than
//!     `self.center`, so this Pane is never painted by the workspace itself, yet
//!     it is still the common open target for the project panel
//!     (`open_path_preview` -> `last_active_center_pane`), agent diffs
//!     (`add_item_to_center`), and agent tool file references. Rendering it here
//!     surfaces opened files, diffs, and references without touching any of
//!     those call sites or `ProjectPanel`/`AgentDiffPane`.
//!   * `Terminal` — the terminal (`TerminalPanel`).
//!
//! The AgentApp window skips `initialize_panels` (see `zed.rs`), so neither
//! `ProjectPanel` nor `TerminalPanel` is loaded into a dock. Instead this host
//! *owns* each panel as a standalone `Entity<_>` and renders it directly:
//! `ProjectPanel`, `TerminalPanel`, and `Pane` all implement `Render`, so they
//! can be used as elements without going through the (disabled) dock machinery.
//! Panels are lazy-loaded: the file tree is requested on construction (Files is
//! enabled by default) and the terminal is requested the first time its module
//! is enabled.
//!
//! Because the terminal is never added to a dock, no `Panel::set_active` is ever
//! driven for it, and a freshly-`load`ed `TerminalPanel` owns *zero* terminals
//! (the first shell is normally spawned by the dock calling
//! `Panel::set_active(true)`). To avoid an empty terminal pane, after loading we
//! call `Panel::set_active(true, ..)` ourselves, reusing the panel's own
//! deferred `add_terminal_shell`. NOTE: `TerminalPanel::set_active` spawns its
//! shell with `RevealStrategy::Always`, which focuses the terminal; on real
//! hardware this may steal focus from the chat input on first enable. If that
//! proves disruptive, switch to a `NoFocus` reveal in the terminal panel.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;

use gpui::{Action, Entity, FocusHandle, Focusable, Subscription, WeakEntity, prelude::*};
use project_panel::ProjectPanel;
use terminal_view::terminal_panel::TerminalPanel;
use ui::{ContextMenu, IconButton, IconName, IconPosition, PopoverMenu, prelude::*};
use workspace::{NewFile, Pane, Workspace, pane};

/// One stackable module in the right dock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum DockModule {
    Files,
    File,
    Changes,
    Terminal,
}

impl DockModule {
    const ALL: [DockModule; 4] = [
        DockModule::Files,
        DockModule::File,
        DockModule::Changes,
        DockModule::Terminal,
    ];

    fn title(self) -> &'static str {
        match self {
            DockModule::Files => "Files",
            DockModule::File => "File",
            DockModule::Changes => "Changes",
            DockModule::Terminal => "Terminal",
        }
    }

    fn element_id(self) -> &'static str {
        match self {
            DockModule::Files => "vibedev-dpanel-files",
            DockModule::File => "vibedev-dpanel-preview",
            DockModule::Changes => "vibedev-dpanel-changes",
            DockModule::Terminal => "vibedev-dpanel-terminal",
        }
    }

    /// Stable per-module index, used to disambiguate `ElementId`s for the
    /// per-panel collapse/close buttons (`(&str, usize)` implements
    /// `Into<ElementId>`; `(&str, &str)` does not).
    fn index(self) -> usize {
        DockModule::ALL
            .iter()
            .position(|m| *m == self)
            .unwrap_or(0)
    }
}

/// Stacked host for the AgentApp right panel. Owns its panels directly rather
/// than relying on the workspace dock (which is disabled in agent mode). The
/// `center_pane` is the workspace's own center `Pane`, handed to us so the
/// "File" module can render it; we never mutate it, only render its handle.
pub struct VibedevRightDock {
    workspace: WeakEntity<Workspace>,
    center_pane: Entity<Pane>,
    /// A `Pane` owned by this dock (NOT registered in `workspace.panes`) that
    /// hosts the agent diff (`AgentDiffPane`). Keeping the diff out of the
    /// center pane lets "File" (opened file contents) and "Changes" (the diff)
    /// live in separate modules. Agent diffs are redirected here via
    /// `Workspace::agent_changes_pane` (see `agent_diff::deploy_in_workspace`).
    changes_pane: Entity<Pane>,
    project_panel: Option<Entity<ProjectPanel>>,
    terminal_panel: Option<Entity<TerminalPanel>>,
    /// Modules currently shown (in display order). Closing a module removes it
    /// here; the header menu adds it back.
    enabled: Vec<DockModule>,
    /// Modules that are enabled but collapsed (header visible, body hidden).
    collapsed: HashSet<DockModule>,
    /// Set once a `TerminalPanel::load` has been kicked off, so re-enabling the
    /// Terminal module does not spawn a second load.
    terminal_load_started: bool,
    focus_handle: FocusHandle,
    /// Event subscriptions kept alive for the dock's lifetime (dropped with it).
    /// Currently holds the center-pane subscription that auto-surfaces the
    /// `File` module when a file is opened into the center pane.
    _subscriptions: Vec<Subscription>,
}

impl VibedevRightDock {
    pub fn new(
        workspace: WeakEntity<Workspace>,
        center_pane: Entity<Pane>,
        project: Entity<project::Project>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // A standalone `Pane` to host the agent diff ("Changes"). It is NOT
        // added to `workspace.panes` (so it never participates in the editor's
        // pane navigation/focus chain); the dock simply renders its handle and
        // agent diffs are routed into it (see `agent_changes_pane`). Built with
        // the same arguments the workspace uses for its center pane
        // (`workspace.rs` center-pane construction) so behaviour matches.
        let changes_pane = cx.new(|cx| {
            Pane::new(
                workspace.clone(),
                project,
                Arc::new(AtomicUsize::new(0)),
                None,
                NewFile.boxed_clone(),
                false,
                window,
                cx,
            )
        });

        // Surface the `File` module whenever a file is opened into the center
        // pane. The center pane is the common open target for the project panel,
        // agent diffs, and agent tool file references (see the module docs), so
        // observing its `AddItem` event lets *any* of those call sites bring the
        // File panel out without the user having to enable it first.
        //
        // Symmetrically, surface the `Changes` module whenever a diff is opened
        // into our own `changes_pane` (agent diffs are redirected there). Both
        // callbacks only mutate this dock's own `enabled`/`collapsed` fields.
        let _subscriptions = vec![
            cx.subscribe(&center_pane, Self::handle_center_pane_event),
            cx.subscribe(&changes_pane, Self::handle_changes_pane_event),
        ];

        let mut this = Self {
            workspace,
            center_pane,
            changes_pane,
            project_panel: None,
            terminal_panel: None,
            enabled: vec![DockModule::Files, DockModule::File],
            collapsed: HashSet::new(),
            terminal_load_started: false,
            focus_handle: cx.focus_handle(),
            _subscriptions,
        };
        // Files is enabled by default, so eagerly load the project panel. The
        // center pane needs no loading (it is already live); the terminal loads
        // lazily when its module is first enabled.
        this.load_project_panel(window, cx);
        this
    }

    /// Reacts to the center pane opening a file: ensures the `File` module is
    /// enabled and expanded so the freshly-opened file is actually visible,
    /// even if the user had previously closed or collapsed the module.
    ///
    /// Only `AddItem` (a file/diff *opened* into the pane) triggers this, not
    /// every `ActivateItem` (tab switch / re-activation); otherwise merely
    /// clicking around tabs would keep re-expanding a panel the user just
    /// collapsed.
    ///
    /// Lease safety: this callback runs from the event dispatch (never from
    /// `render`), and only mutates this dock's own `enabled`/`collapsed` fields
    /// plus `cx.notify()`. It never updates the workspace or the center pane
    /// (the `event` is read-only and the `Entity<Pane>` is ignored), so no
    /// second lease on either is taken.
    fn handle_center_pane_event(
        &mut self,
        _pane: Entity<Pane>,
        event: &pane::Event,
        cx: &mut Context<Self>,
    ) {
        if !matches!(event, pane::Event::AddItem { .. }) {
            return;
        }
        if !self.enabled.contains(&DockModule::File) {
            // Insert in canonical order so the stack stays stable (File sits
            // right after Files), matching `enable_module`'s ordering.
            self.enabled.push(DockModule::File);
            self.enabled.sort_by_key(|m| {
                DockModule::ALL
                    .iter()
                    .position(|x| x == m)
                    .unwrap_or(usize::MAX)
            });
        }
        self.collapsed.remove(&DockModule::File);
        cx.notify();
    }

    /// The dock-owned `Pane` that hosts the agent diff. Handed to the workspace
    /// (as a `WeakEntity`) so `agent_diff::deploy_in_workspace` can route diffs
    /// here instead of into the center pane.
    pub fn changes_pane(&self) -> Entity<Pane> {
        self.changes_pane.clone()
    }

    /// Reacts to a diff being opened into the dock-owned `changes_pane`: ensures
    /// the `Changes` module is enabled and expanded so the freshly-opened diff
    /// is visible, even if the user had previously closed or collapsed it. Same
    /// `AddItem`-only / canonical-order rationale and lease safety as
    /// `handle_center_pane_event`: only this dock's `enabled`/`collapsed` fields
    /// plus `cx.notify()` are touched.
    fn handle_changes_pane_event(
        &mut self,
        _pane: Entity<Pane>,
        event: &pane::Event,
        cx: &mut Context<Self>,
    ) {
        if !matches!(event, pane::Event::AddItem { .. }) {
            return;
        }
        if !self.enabled.contains(&DockModule::Changes) {
            self.enabled.push(DockModule::Changes);
            self.enabled.sort_by_key(|m| {
                DockModule::ALL
                    .iter()
                    .position(|x| x == m)
                    .unwrap_or(usize::MAX)
            });
        }
        self.collapsed.remove(&DockModule::Changes);
        cx.notify();
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

    /// Lazily loads the terminal. Same pattern as `load_project_panel`, with one
    /// extra step: a freshly-loaded `TerminalPanel` owns no terminals (the dock
    /// normally spawns the first via `Panel::set_active(true)`), so we drive
    /// `set_active(true)` ourselves once the panel is in hand. This is done
    /// inside `update_in` on *this* host — not nested inside any
    /// `workspace.update` — because `TerminalPanel::set_active` itself takes a
    /// workspace lease on a later tick (`cx.defer_in` -> `default_working_directory`).
    fn load_terminal_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.terminal_load_started {
            return;
        }
        self.terminal_load_started = true;
        let workspace = self.workspace.clone();
        cx.spawn_in(window, async move |this, cx| {
            let panel = TerminalPanel::load(workspace, cx.clone()).await?;
            this.update_in(cx, |this, window, cx| {
                // Spawn the first terminal shell. `set_active` is idempotent
                // (it early-returns unless newly active with no terminals) and
                // reuses the dock's deferred `add_terminal_shell` path.
                panel.update(cx, |panel, cx| {
                    use workspace::dock::Panel;
                    panel.set_active(true, window, cx);
                });
                this.terminal_panel = Some(panel);
                cx.notify();
            })
        })
        .detach_and_log_err(cx);
    }

    /// Enables `module` if not already shown, loading any backing panel and
    /// clearing its collapsed state.
    fn enable_module(&mut self, module: DockModule, window: &mut Window, cx: &mut Context<Self>) {
        if self.enabled.contains(&module) {
            return;
        }
        // Re-insert in canonical order so the stack stays stable.
        self.enabled.push(module);
        self.enabled
            .sort_by_key(|m| DockModule::ALL.iter().position(|x| x == m).unwrap_or(usize::MAX));
        self.collapsed.remove(&module);
        match module {
            DockModule::Files => self.load_project_panel(window, cx),
            DockModule::Terminal => self.load_terminal_panel(window, cx),
            DockModule::File | DockModule::Changes => {}
        }
        cx.notify();
    }

    /// Removes `module` from the stack (the header menu can re-add it).
    fn disable_module(&mut self, module: DockModule, cx: &mut Context<Self>) {
        self.enabled.retain(|m| *m != module);
        self.collapsed.remove(&module);
        cx.notify();
    }

    /// Toggles a module on/off from the header "panels" menu.
    fn toggle_module(&mut self, module: DockModule, window: &mut Window, cx: &mut Context<Self>) {
        if self.enabled.contains(&module) {
            self.disable_module(module, cx);
        } else {
            self.enable_module(module, window, cx);
        }
    }

    /// Collapses/expands a module's body (header stays visible either way).
    fn toggle_collapsed(&mut self, module: DockModule, cx: &mut Context<Self>) {
        if !self.collapsed.remove(&module) {
            self.collapsed.insert(module);
        }
        cx.notify();
    }

    /// The header "panels" menu: one checkmarked toggle per module.
    ///
    /// `PopoverMenu::menu` hands us only `&mut Window, &mut App` (no
    /// `Context<Self>`), so we capture a `WeakEntity<Self>` and route each
    /// toggle back through it. The menu is *built* (and thus the `enabled` set
    /// snapshotted) each time the popover opens, so the checkmarks stay current.
    fn render_panels_menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let host = cx.entity().downgrade();
        PopoverMenu::new("vibedev-dock-menu")
            .trigger(
                IconButton::new("vibedev-dock-panels", IconName::Ellipsis)
                    .icon_size(IconSize::Small),
            )
            .menu(move |window, cx| {
                let host = host.clone();
                let enabled: HashSet<DockModule> = host
                    .read_with(cx, |this, _| this.enabled.iter().copied().collect())
                    .unwrap_or_default();
                Some(ContextMenu::build(window, cx, move |mut menu, _window, _cx| {
                    menu = menu.header("Panels");
                    for module in DockModule::ALL {
                        let checked = enabled.contains(&module);
                        let host = host.clone();
                        menu = menu.toggleable_entry(
                            module.title(),
                            checked,
                            IconPosition::End,
                            None,
                            move |window, cx| {
                                host.update(cx, |this, cx| {
                                    this.toggle_module(module, window, cx);
                                })
                                .ok();
                            },
                        );
                    }
                    menu
                }))
            })
    }

    /// Renders one stacked module: a header (title + collapse + close) plus,
    /// unless collapsed, its body. Each `.dpanel` is `flex_1()` + `min_h_0()` so
    /// it shares the column and can actually shrink; the body is
    /// `overflow_hidden()` so an oversized child cannot push siblings off-screen.
    fn render_dpanel(
        &self,
        module: DockModule,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let colors = cx.theme().colors();
        let collapsed = self.collapsed.contains(&module);

        // Body: each owned panel / the center pane implements `Render`, so its
        // `Entity<_>` handle can be cloned in directly as a child. We unify the
        // (panel | placeholder) branches into `AnyElement`. These are pure
        // handle clones — no `update`, no `await`, no workspace lease.
        let body: AnyElement = match module {
            DockModule::Files => self
                .project_panel
                .clone()
                .map(IntoElement::into_any_element)
                .unwrap_or_else(loading_placeholder),
            DockModule::File => self.center_pane.clone().into_any_element(),
            DockModule::Changes => self.changes_pane.clone().into_any_element(),
            DockModule::Terminal => self
                .terminal_panel
                .clone()
                .map(IntoElement::into_any_element)
                .unwrap_or_else(loading_placeholder),
        };

        let module_index = module.index();
        let (collapse_icon, collapse_id) = if collapsed {
            (IconName::ChevronDown, "vibedev-dpanel-expand")
        } else {
            (IconName::ChevronUp, "vibedev-dpanel-collapse")
        };

        v_flex()
            .id(module.element_id())
            .flex_1()
            .min_h_0()
            .border_t_1()
            .border_color(colors.border)
            .child(
                h_flex()
                    .flex_none()
                    .h(px(30.))
                    .w_full()
                    .px_2()
                    .gap_1()
                    .justify_between()
                    .bg(colors.tab_bar_background)
                    .child(
                        Label::new(module.title())
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    )
                    .child(
                        h_flex()
                            .gap_0p5()
                            .child(
                                IconButton::new((collapse_id, module_index), collapse_icon)
                                    .icon_size(IconSize::XSmall)
                                    .on_click(cx.listener(move |this, _event, _window, cx| {
                                        this.toggle_collapsed(module, cx);
                                    })),
                            )
                            .child(
                                IconButton::new(
                                    ("vibedev-dpanel-close", module_index),
                                    IconName::Close,
                                )
                                .icon_size(IconSize::XSmall)
                                .on_click(cx.listener(move |this, _event, _window, cx| {
                                    this.disable_module(module, cx);
                                })),
                            ),
                    ),
            )
            .when(!collapsed, |this| {
                this.child(
                    div()
                        .flex_1()
                        .w_full()
                        .min_h_0()
                        .overflow_hidden()
                        .child(body),
                )
            })
    }
}

/// Lightweight placeholder shown while a module's panel is still loading.
fn loading_placeholder() -> AnyElement {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .child(Label::new("Loading…").color(Color::Muted))
        .into_any_element()
}

impl Focusable for VibedevRightDock {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for VibedevRightDock {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Render is read-only: it clones handles and reads fields. All mutation
        // (toggle/collapse/close, and the workspace-leasing terminal spawn) runs
        // from event handlers or spawned tasks, never synchronously here, so no
        // double-lease on the workspace or any rendered entity can occur. The
        // center pane is safe to render because agent mode does not also paint
        // `self.center`, and `Pane::render` only reads.
        //
        // Build the panel elements first (each `render_dpanel` borrows `cx`),
        // then read theme colors, so the two `cx` borrows don't overlap.
        let modules: Vec<DockModule> = self.enabled.clone();
        let panels: Vec<AnyElement> = modules
            .into_iter()
            .map(|module| self.render_dpanel(module, window, cx).into_any_element())
            .collect();

        let colors = cx.theme().colors();

        v_flex()
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(colors.panel_background)
            .child(
                h_flex()
                    .flex_none()
                    .w_full()
                    .h(px(30.))
                    .px_2()
                    .justify_between()
                    .items_center()
                    .bg(colors.tab_bar_background)
                    .border_b_1()
                    .border_color(colors.border)
                    .child(
                        Label::new("Workbench")
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    )
                    .child(self.render_panels_menu(cx)),
            )
            .child(v_flex().flex_1().w_full().min_h_0().children(panels))
    }
}
