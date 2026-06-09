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

use agent_client_protocol::schema as acp;
use agent_ui::subagent_fanout::SubagentFanoutModel;
use agent_ui::{AgentPanel, AgentPanelEvent, SelectedSubagent};
use gpui::{Action, App, Entity, FocusHandle, Focusable, Subscription, WeakEntity, prelude::*};
use markdown::{Markdown, MarkdownElement, MarkdownFont, MarkdownStyle};
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
    Execution,
    Plan,
    Terminal,
}

impl DockModule {
    const ALL: [DockModule; 6] = [
        DockModule::Files,
        DockModule::File,
        DockModule::Changes,
        DockModule::Execution,
        DockModule::Plan,
        DockModule::Terminal,
    ];

    fn title(self) -> &'static str {
        match self {
            DockModule::Files => "Files",
            DockModule::File => "File",
            DockModule::Changes => "Changes",
            DockModule::Execution => "Execution",
            DockModule::Plan => "Plan",
            DockModule::Terminal => "Terminal",
        }
    }

    fn element_id(self) -> &'static str {
        match self {
            DockModule::Files => "vibedev-dpanel-files",
            DockModule::File => "vibedev-dpanel-preview",
            DockModule::Changes => "vibedev-dpanel-changes",
            DockModule::Execution => "vibedev-dpanel-execution",
            DockModule::Plan => "vibedev-dpanel-plan",
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
    /// Subscription to the `AgentPanel`'s `ActiveViewChanged` events, lazily
    /// installed from `render` once the panel has been async-injected into the
    /// workspace (it is not present when this dock is first constructed). Kept
    /// alive for the dock's lifetime; refreshes `active_thread` on each change.
    agent_panel_subscription: Option<Subscription>,
    /// The currently active root conversation thread (per the `AgentPanel`).
    /// A strong handle so it can be `observe`d; replaced wholesale whenever the
    /// active view changes.
    active_thread: Option<Entity<acp_thread::AcpThread>>,
    /// `cx.observe` on `active_thread`: any thread mutation (plan/subagent
    /// updates `cx.notify()`) re-renders the Execution panel and auto-surfaces
    /// the module the first time subagent progress appears.
    active_thread_subscription: Option<Subscription>,
    /// Cached Markdown entity for the drilled-into subagent's current stream.
    /// `SubagentStep.stream` is a plain `String` that changes as the agent
    /// streams; parsing it every frame would re-parse the whole buffer, so we
    /// keep the last `(stream_text, parsed_entity)` and only rebuild the entity
    /// when the stream text changes. Built/refreshed from `render_execution_panel`
    /// (which holds `&mut Context<Self>`); rendered read-only.
    detail_markdown: Option<(String, Entity<Markdown>)>,
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
            agent_panel_subscription: None,
            active_thread: None,
            active_thread_subscription: None,
            detail_markdown: None,
            focus_handle: cx.focus_handle(),
            _subscriptions,
        };
        // Re-render whenever the drilled-into subagent selection changes (a
        // fan-out card was clicked in the conversation, a dock list row was
        // clicked, or the detail panel's back button cleared it). The callback
        // ONLY notifies — it never writes the global or updates another entity,
        // so it cannot re-enter the `set_global -> observe -> set_global` cycle.
        this._subscriptions
            .push(cx.observe_global::<SelectedSubagent>(|_this, cx| cx.notify()));
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
        pane: Entity<Pane>,
        event: &pane::Event,
        cx: &mut Context<Self>,
    ) {
        if !matches!(event, pane::Event::AddItem { .. }) {
            return;
        }
        // TEMP diagnostic for the "clicking a file tracks in the tree but the
        // File panel stays empty" report. If this line never prints when a file
        // is opened, the file landed in some pane other than the dock's
        // `center_pane` (routing); if it prints with active_item_present=true
        // but the panel shows nothing, it is a paint/layout issue. Revert once
        // the root cause is confirmed.
        log::info!(
            "VIBEDEV-DIAG[file-open]: AddItem on pane id={:?} (dock center_pane id={:?}), active_item_present={}",
            pane.entity_id(),
            self.center_pane.entity_id(),
            pane.read(cx).active_item().is_some(),
        );
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

    /// Resolves the `AgentPanel`'s currently active root thread and, if it
    /// changed, re-`observe`s it. Always re-checks whether the Execution module
    /// should surface (the freshly-selected thread may already have subagents).
    ///
    /// Lease safety: only reads through `panel` and the resolved thread; the
    /// sole mutations are this dock's own fields plus `cx.notify()`. No second
    /// lease on the panel, thread, or workspace is taken.
    fn refresh_active_thread(&mut self, panel: &Entity<AgentPanel>, cx: &mut Context<Self>) {
        let thread = panel.read(cx).active_agent_thread(cx);
        let changed = self.active_thread.as_ref().map(|t| t.entity_id())
            != thread.as_ref().map(|t| t.entity_id());
        if changed {
            self.active_thread = thread.clone();
            self.active_thread_subscription = thread.as_ref().map(|t| {
                cx.observe(t, |this, thread, cx| {
                    this.on_thread_updated(&thread, cx);
                })
            });
            // The active conversation changed, so any subagent drilled-into from
            // the previous thread no longer exists in the new model: clear the
            // selection so the Execution panel falls back to its list. This is
            // the ONLY place the dock writes the global, and it runs on the
            // `ActiveViewChanged` event path (never from `render` or the global
            // observer). `cx` is a `&mut Context<Self>`, which derefs to the
            // `&mut App` that `SelectedSubagent::set` needs.
            SelectedSubagent::set(None, cx);
        }
        // Initial resolution and every switch both try to auto-surface.
        if let Some(thread) = thread.as_ref() {
            self.maybe_surface_execution(thread, cx);
            self.maybe_surface_plan(thread, cx);
        }
        cx.notify();
    }

    /// `cx.observe` callback for the active thread: re-renders the Execution
    /// panel and auto-surfaces the module the first time subagent progress
    /// appears. Lease safety: reads `thread`, mutates only this dock's fields.
    fn on_thread_updated(
        &mut self,
        thread: &Entity<acp_thread::AcpThread>,
        cx: &mut Context<Self>,
    ) {
        self.maybe_surface_execution(thread, cx);
        self.maybe_surface_plan(thread, cx);
        cx.notify();
    }

    /// Surfaces (enables + expands, in canonical order) the Execution module the
    /// first time the active thread carries any `SubagentProgress` entry. Reads
    /// only; the caller is responsible for `cx.notify()` so this never doubles a
    /// notify. Takes `&App` so it composes with both the observe callback's
    /// `Context<Self>` (which derefs to `App`) and the render path.
    fn maybe_surface_execution(&mut self, thread: &Entity<acp_thread::AcpThread>, cx: &App) {
        let has_subagents = thread.read(cx).entries().iter().any(|entry| {
            matches!(entry, acp_thread::AgentThreadEntry::SubagentProgress(_))
        });
        if has_subagents && !self.enabled.contains(&DockModule::Execution) {
            self.enabled.push(DockModule::Execution);
            self.enabled.sort_by_key(|m| {
                DockModule::ALL
                    .iter()
                    .position(|x| x == m)
                    .unwrap_or(usize::MAX)
            });
            self.collapsed.remove(&DockModule::Execution);
        }
    }

    /// Surfaces (enables + expands, in canonical order) the Plan module the first
    /// time the active thread carries a non-empty `update_plan` (ACP) plan. Same
    /// read-only / caller-notifies contract as `maybe_surface_execution`: reads
    /// only through `thread`, mutates only this dock's `enabled`/`collapsed`, and
    /// leaves `cx.notify()` to the caller so a single update never double-notifies.
    fn maybe_surface_plan(&mut self, thread: &Entity<acp_thread::AcpThread>, cx: &App) {
        let has_plan = !thread.read(cx).plan().is_empty();
        if has_plan && !self.enabled.contains(&DockModule::Plan) {
            self.enabled.push(DockModule::Plan);
            self.enabled.sort_by_key(|m| {
                DockModule::ALL
                    .iter()
                    .position(|x| x == m)
                    .unwrap_or(usize::MAX)
            });
            self.collapsed.remove(&DockModule::Plan);
        }
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
            DockModule::File | DockModule::Changes | DockModule::Execution | DockModule::Plan => {}
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

    /// Renders the "Execution" module body: the subagent fan-out tree for the
    /// currently active conversation thread. Reads `self.active_thread`, folds
    /// its `SubagentProgress` entries into a `SubagentFanoutModel`, and renders
    /// one card per node (status icon + agent type + title + token count, with
    /// child subagents indented).
    ///
    /// v1 simplification: live step streaming (`SubagentStep.stream`) and the
    /// full subagent reply are intentionally NOT rendered here — only static
    /// status/title/token/count. Streaming + full output are deferred to T8.
    ///
    /// Lease safety: reads through `self.active_thread` (a handle on the active
    /// thread) and the theme; takes no lease on either. Takes `&mut Context<Self>`
    /// (rather than `&App`) only so the drill-in branch can refresh the
    /// `detail_markdown` cache via `cx.new(...)` — that builds a *new* `Markdown`
    /// entity, never updating `self`'s host entity, so there is no re-entrant
    /// lease. `render_dpanel` builds this body before binding its theme `colors`,
    /// so the `&mut` borrow here does not collide with that immutable borrow.
    fn render_execution_panel(&mut self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        // No active conversation selected yet.
        let Some(thread) = self.active_thread.as_ref() else {
            return execution_empty_state("无活跃会话");
        };

        // Collect every `SubagentProgress` entry on the thread (not just a
        // contiguous run — the dock shows the whole active session's fan-out).
        let progresses: Vec<acp_thread::SubagentProgress> = thread
            .read(cx)
            .entries()
            .iter()
            .filter_map(|entry| match entry {
                acp_thread::AgentThreadEntry::SubagentProgress(progress) => {
                    Some(progress.clone())
                }
                _ => None,
            })
            .collect();

        let model = SubagentFanoutModel::from_entries(&progresses);
        if model.nodes.is_empty() {
            return execution_empty_state("暂无子任务执行");
        }

        // If a subagent is drilled-into and still present in the current model,
        // render its detail view instead of the list. A stale selection (the
        // session was switched, or the node is gone) silently falls back to the
        // list below. Reading the global here is a pure read.
        if let Some(selected) = SelectedSubagent::get(cx) {
            if let Some(node) = model.node(&selected) {
                // Refresh the Markdown cache for the selected node's live stream.
                // The stream text changes as the agent streams, but re-parsing it
                // every frame is wasteful — so we keep the last `(text, entity)`
                // and only rebuild the entity when the text changes. `node`
                // borrows the local `model` (not `self`), so mutating
                // `self.detail_markdown` and calling `cx.new(...)` here is sound;
                // `cx.new` builds a *fresh* Markdown entity and takes no lease on
                // this dock.
                let stream = node.step.as_ref().and_then(|step| step.stream.clone());
                let markdown = match stream {
                    Some(text) => {
                        let needs_rebuild = self
                            .detail_markdown
                            .as_ref()
                            .map(|(cached, _)| cached != &text)
                            .unwrap_or(true);
                        if needs_rebuild {
                            let entity =
                                cx.new(|cx| Markdown::new(text.clone().into(), None, None, cx));
                            self.detail_markdown = Some((text, entity));
                        }
                        self.detail_markdown
                            .as_ref()
                            .map(|(_, entity)| entity.clone())
                    }
                    None => {
                        // Selected node has no stream yet; drop any stale cache so
                        // a later stream for a *different* node rebuilds cleanly.
                        self.detail_markdown = None;
                        None
                    }
                };
                return self.render_subagent_detail(node, markdown, window, cx);
            }
        }

        let colors = cx.theme().colors();
        let running = model.running_count();
        let total = model.total_count();
        let node_count = model.nodes.len();
        let row_border = colors.border;

        v_flex()
            .id("vibedev-execution-tree")
            .size_full()
            .overflow_y_scroll()
            .child(
                h_flex()
                    .flex_none()
                    .px_2()
                    .py_1()
                    .gap_1()
                    .child(
                        Icon::new(IconName::Person)
                            .size(IconSize::Small)
                            .color(Color::Muted),
                    )
                    .child(
                        Label::new(format!("{running}/{total} 运行中"))
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    ),
            )
            .children(model.nodes.iter().enumerate().map(|(index, node)| {
                // Icon + color per status, copied from
                // `thread_view.rs::render_subagent_fanout` (3581-3588). v1 omits
                // the `with_rotate_animation` spinner on Running used there.
                let (status_icon, status_color) = match node.status {
                    acp_thread::SubagentStatus::Running => (IconName::ArrowCircle, Color::Accent),
                    acp_thread::SubagentStatus::Done => (IconName::Check, Color::Success),
                    acp_thread::SubagentStatus::Failed => (IconName::XCircle, Color::Error),
                    acp_thread::SubagentStatus::Queued => (IconName::Circle, Color::Muted),
                };

                let tokens_label = node.tokens_used.map(|tokens| {
                    Label::new(format!("{tokens} tok"))
                        .size(LabelSize::Small)
                        .color(Color::Muted)
                });

                v_flex()
                    .id((
                        ElementId::from("vibedev-execution-row"),
                        SharedString::from(node.subagent_id.clone()),
                    ))
                    .py_1()
                    .px_2()
                    .gap_1()
                    .cursor_pointer()
                    .when(index < node_count - 1, |this| {
                        this.border_b_1().border_color(row_border)
                    })
                    // Indent child subagents one level under their parent.
                    .when(node.is_child(), |this| this.pl_4())
                    // Dock-side drill-in entry point: clicking a row writes the
                    // same `agent_ui` global the conversation cards write. The
                    // closure only needs `&mut App` to set the global; no lease.
                    .on_click({
                        let id = node.subagent_id.clone();
                        move |_event, _window, cx: &mut App| {
                            SelectedSubagent::set(Some(id.clone()), cx);
                        }
                    })
                    .child(
                        h_flex()
                            .gap_1p5()
                            .child(
                                Icon::new(status_icon)
                                    .size(IconSize::Small)
                                    .color(status_color),
                            )
                            .child(
                                Label::new(node.agent_type.clone())
                                    .size(LabelSize::Small)
                                    .color(Color::Muted),
                            )
                            .child(
                                Label::new(node.title.clone())
                                    .size(LabelSize::Small)
                                    .truncate(),
                            )
                            .when_some(tokens_label, |this, label| {
                                this.child(div().flex_1()).child(label)
                            }),
                    )
                    .into_any_element()
            }))
            .into_any_element()
    }

    /// Renders the drill-in detail for a single subagent inside the Execution
    /// module: a back row, the node's status/type/title/tokens, and its current
    /// step (label + the full, un-truncated stream in a scroll region).
    ///
    /// The current step's `stream` is rendered as Markdown (rich text / code /
    /// lists) via the pre-parsed `markdown` entity the caller cached, rather than
    /// as a plain `Label`. `markdown` is `None` when the node has no stream yet.
    ///
    /// Lease safety: read-only. Reads the passed-in `node` (a borrow into the
    /// caller's freshly-built `SubagentFanoutModel`), the cached `markdown`
    /// entity (rendered, not updated), and the theme; takes no lease and performs
    /// no `update`. `MarkdownStyle::themed` needs only `&Window` + `&App`. The
    /// back button's `on_click` only writes the `agent_ui` global on the click
    /// event path — never from this render.
    fn render_subagent_detail(
        &self,
        node: &agent_ui::subagent_fanout::SubagentNode,
        markdown: Option<Entity<Markdown>>,
        window: &Window,
        cx: &App,
    ) -> AnyElement {
        let colors = cx.theme().colors();

        // Status icon + color + label, matching `render_execution_panel`'s
        // list rows.
        let (status_icon, status_color, status_label) = match node.status {
            acp_thread::SubagentStatus::Running => {
                (IconName::ArrowCircle, Color::Accent, "运行中")
            }
            acp_thread::SubagentStatus::Done => (IconName::Check, Color::Success, "已完成"),
            acp_thread::SubagentStatus::Failed => (IconName::XCircle, Color::Error, "失败"),
            acp_thread::SubagentStatus::Queued => (IconName::Circle, Color::Muted, "排队中"),
        };

        let tokens_label = node.tokens_used.map(|tokens| {
            Label::new(format!("{tokens} tok"))
                .size(LabelSize::Small)
                .color(Color::Muted)
        });

        v_flex()
            .id("vibedev-execution-detail")
            .size_full()
            .overflow_y_scroll()
            // Back row: returns to the list by clearing the global selection.
            .child(
                h_flex()
                    .flex_none()
                    .px_2()
                    .py_1()
                    .gap_1p5()
                    .border_b_1()
                    .border_color(colors.border)
                    .child(
                        IconButton::new("vibedev-execution-detail-back", IconName::ArrowLeft)
                            .icon_size(IconSize::Small)
                            .on_click(|_event, _window, cx: &mut App| {
                                SelectedSubagent::set(None, cx);
                            }),
                    )
                    .child(
                        Label::new(node.agent_type.clone())
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    )
                    .child(
                        Label::new(node.title.clone())
                            .size(LabelSize::Small)
                            .truncate(),
                    ),
            )
            // Status + token summary row.
            .child(
                h_flex()
                    .flex_none()
                    .px_2()
                    .py_1()
                    .gap_1p5()
                    .child(
                        Icon::new(status_icon)
                            .size(IconSize::Small)
                            .color(status_color),
                    )
                    .child(
                        Label::new(status_label)
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    )
                    .when_some(tokens_label, |this, label| {
                        this.child(div().flex_1()).child(label)
                    }),
            )
            // Current step: the step label, then the full stream rendered as
            // Markdown (NOT truncated) inside a scroll region so a long reply
            // stays contained. The markdown is the caller's cached, pre-parsed
            // entity for *this* node's stream; rendering it is read-only.
            .map(|this| match node.step.as_ref() {
                Some(step) => this.child(
                    v_flex()
                        .id("vibedev-execution-detail-step")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .px_2()
                        .py_1()
                        .gap_1()
                        .child(
                            Label::new(step.label.clone())
                                .size(LabelSize::Small)
                                .color(Color::Muted),
                        )
                        .when_some(markdown, |this, markdown| {
                            this.child(MarkdownElement::new(
                                markdown,
                                MarkdownStyle::themed(MarkdownFont::Agent, window, cx),
                            ))
                        }),
                ),
                None => this.child(
                    div()
                        .flex_none()
                        .px_2()
                        .py_1()
                        .child(Label::new("暂无步骤详情").color(Color::Muted)),
                ),
            })
            .into_any_element()
    }

    /// Renders the "Plan" module body: the active conversation thread's plan
    /// (ACP `update_plan`), one row per entry with a status icon and the entry's
    /// label, plus a header showing completion progress.
    ///
    /// Each entry's `content` is already an `Entity<Markdown>` on the thread
    /// (built by `Plan` from the ACP `update_plan` payload), so it is rendered as
    /// a `MarkdownElement` (rich bold/code/links) rather than as the raw source
    /// via `Label`. No entity is created here — the thread owns it.
    ///
    /// Lease safety: read-only. Reads through `self.active_thread`, the thread's
    /// `plan()`, and each entry's `content` markdown entity (cloned and rendered,
    /// never updated), plus the theme; takes no lease and no `update`.
    /// `MarkdownStyle::themed` needs only `&Window` + `&App`.
    fn render_plan_panel(&self, window: &Window, cx: &App) -> AnyElement {
        let colors = cx.theme().colors();

        // No active conversation selected yet.
        let Some(thread) = self.active_thread.as_ref() else {
            return execution_empty_state("无活跃会话");
        };

        let plan = thread.read(cx).plan();
        if plan.is_empty() {
            return execution_empty_state("暂无计划");
        }

        let stats = plan.stats();
        let total = plan.entries.len();
        let entry_count = total;
        let row_border = colors.border;

        v_flex()
            .id("vibedev-plan-list")
            .size_full()
            .overflow_y_scroll()
            .child(
                h_flex()
                    .flex_none()
                    .px_2()
                    .py_1()
                    .gap_1()
                    .child(
                        Icon::new(IconName::ListTodo)
                            .size(IconSize::Small)
                            .color(Color::Muted),
                    )
                    .child(
                        Label::new(format!("{}/{} 完成", stats.completed, total))
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    ),
            )
            .children(plan.entries.iter().enumerate().map(|(index, entry)| {
                // Icon + color per status, copied from
                // `thread_view.rs::render` plan rows (3387-3406). v1 omits the
                // `with_rotate_animation` spinner on InProgress used there.
                let (status_icon, status_color) = match entry.status {
                    acp::PlanEntryStatus::InProgress => (IconName::TodoProgress, Color::Accent),
                    acp::PlanEntryStatus::Completed => (IconName::TodoComplete, Color::Success),
                    acp::PlanEntryStatus::Pending | _ => (IconName::TodoPending, Color::Muted),
                };
                v_flex()
                    .py_1()
                    .px_2()
                    .gap_1p5()
                    .when(index < entry_count - 1, |this| {
                        this.border_b_1().border_color(row_border)
                    })
                    .child(
                        h_flex()
                            .gap_1p5()
                            .items_start()
                            .child(
                                Icon::new(status_icon)
                                    .size(IconSize::Small)
                                    .color(status_color),
                            )
                            // Render the entry's pre-parsed markdown (the thread
                            // owns the `Entity<Markdown>`; we only clone + render).
                            .child(div().flex_1().min_w_0().child(MarkdownElement::new(
                                entry.content.clone(),
                                MarkdownStyle::themed(MarkdownFont::Agent, window, cx),
                            ))),
                    )
                    .into_any_element()
            }))
            .into_any_element()
    }

    /// Renders one stacked module: a header (title + collapse + close) plus,
    /// unless collapsed, its body. Each `.dpanel` is `flex_1()` + `min_h_0()` so
    /// it shares the column and can actually shrink; the body is
    /// `overflow_hidden()` so an oversized child cannot push siblings off-screen.
    fn render_dpanel(
        &mut self,
        module: DockModule,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let collapsed = self.collapsed.contains(&module);

        // Body: each owned panel / the center pane implements `Render`, so its
        // `Entity<_>` handle can be cloned in directly as a child. We unify the
        // (panel | placeholder) branches into `AnyElement`. These are pure
        // handle clones — no `update`, no `await`, no workspace lease. Built
        // *before* the theme `colors` borrow below so the Execution arm can take
        // a `&mut Context<Self>` (it refreshes the markdown cache) without
        // colliding with that immutable borrow of `cx`.
        let body: AnyElement = match module {
            DockModule::Files => self
                .project_panel
                .clone()
                .map(IntoElement::into_any_element)
                .unwrap_or_else(loading_placeholder),
            // Render whichever center pane is the *current* file-open target, not
            // the one captured at construction time. A split moves
            // `last_active_center_pane` onto the new pane, and file-opens follow
            // it (`open_path_preview` -> `last_active_center_pane`); rendering the
            // stale captured pane would show nothing. This is a pure read on the
            // workspace (`upgrade` + `read`); no lease. Falls back to the captured
            // pane if the workspace is gone or has no center pane yet.
            DockModule::File => self
                .workspace
                .upgrade()
                .and_then(|workspace| workspace.read(cx).last_active_center_pane())
                .unwrap_or_else(|| self.center_pane.clone())
                .into_any_element(),
            DockModule::Changes => self.changes_pane.clone().into_any_element(),
            DockModule::Execution => self.render_execution_panel(window, cx),
            DockModule::Plan => self.render_plan_panel(window, cx),
            DockModule::Terminal => self
                .terminal_panel
                .clone()
                .map(IntoElement::into_any_element)
                .unwrap_or_else(loading_placeholder),
        };

        let colors = cx.theme().colors();
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
                            // The Terminal module owns its `TerminalPanel` directly (not via a
                            // dock), so the panel's built-in "New Terminal" affordance is dead in
                            // agent mode (the global action resolves the panel through a dock
                            // lookup that finds nothing) and `set_active` can't re-spawn once the
                            // last terminal is closed. A header "+" that spawns on the owned entity
                            // is the only working way to add/recover a terminal.
                            .when(module == DockModule::Terminal, |this| {
                                this.child(
                                    IconButton::new(
                                        ("vibedev-dpanel-new-terminal", module_index),
                                        IconName::Plus,
                                    )
                                    .icon_size(IconSize::XSmall)
                                    .on_click(cx.listener(|this, _event, window, cx| {
                                        if let Some(panel) = this.terminal_panel.clone() {
                                            panel.update(cx, |panel, cx| {
                                                panel.spawn_new_terminal(window, cx);
                                            });
                                        }
                                    })),
                                )
                            })
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

/// Centered empty-state for the Execution module (no active session, or an
/// active session with no subagents yet). Mirrors `loading_placeholder`'s style.
fn execution_empty_state(message: &'static str) -> AnyElement {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .child(Label::new(message).color(Color::Muted))
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
        // Lazily install the `AgentPanel` subscription. The dock is built
        // synchronously in `apply_agent_surface`, but the `AgentPanel` is
        // injected asynchronously, so it may not exist on the first frame; we
        // keep retrying from `render` until it does. `render` holds `&mut self`
        // and a `Context<Self>`, so registering subscriptions here is sound. The
        // callback only *registers* — it takes no lease — and we clone the panel
        // out before calling the `&mut self` method to avoid a borrow conflict.
        if self.agent_panel_subscription.is_none()
            && let Some(panel) = self
                .workspace
                .upgrade()
                .and_then(|workspace| workspace.read(cx).panel::<AgentPanel>(cx))
        {
            self.agent_panel_subscription =
                Some(cx.subscribe(&panel, |this, panel, event, cx| {
                    if matches!(event, AgentPanelEvent::ActiveViewChanged) {
                        this.refresh_active_thread(&panel, cx);
                    }
                }));
            let panel = panel.clone();
            self.refresh_active_thread(&panel, cx);
        }

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
