use agent_client_protocol::schema as acp;
use agent_ui::thread_metadata_store::ThreadMetadataStore;
use agent_ui::{Agent, AgentConnectionStore, AgentThreadSource, create_conversation_view};
use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    ParentElement, Render, SharedString, Styled, Subscription, WeakEntity, Window, px,
};
use project::{AgentId, Project, Worktree, WorktreeId};
use ui::prelude::*;
use ui::{IconButton, Tooltip};
use workspace::{PathList, Workspace};

use crate::VibedevConversationItem;

/// Left session sidebar for the VibeDev AgentApp window: a branded header, a
/// multi-root "Folders" zone, a "new conversation" button, and a list of the
/// user's existing threads. It is constructed in the `zed` crate (which can
/// depend on `agent`/`agent_ui`) and injected into the `workspace` crate's
/// `render_agent_layout` left slot via `Workspace::set_agent_left_sidebar`,
/// mirroring the `titlebar_item` pattern.
///
/// Held views: only a `WeakEntity<Workspace>`, so click handlers can swap the
/// hosted conversation in the center pane without owning the workspace.
///
/// The history list reads `agent_ui::ThreadMetadataStore` (a GPUI global) rather
/// than `agent::ThreadStore`. The latter is the native agent's library cache and
/// is only written by `NativeAgent::save_thread`; our conversations run through
/// `Agent::Custom { "VibeDev" }` (an external ACP server) and never touch it, so
/// observing it never fires. `ThreadMetadataStore` optimistically caches threads
/// from *any* agent and notifies synchronously on save, so the list refreshes
/// as soon as a conversation is sent.
pub struct VibedevSessionSidebar {
    workspace: WeakEntity<Workspace>,
    focus_handle: FocusHandle,
    /// Holds the `ThreadMetadataStore` observe + project worktree-change
    /// subscription so the history list and folder zone re-render when threads
    /// or worktrees change.
    _subscriptions: Vec<Subscription>,
}

impl VibedevSessionSidebar {
    /// `project` is passed in by the caller rather than read from `workspace`
    /// here on purpose: the constructor runs inside `configure_agent_mode`, which
    /// holds a `&mut Workspace` lease, so reading the workspace entity through its
    /// handle in this scope would double-lease and panic at window startup. The
    /// caller already has `&mut Workspace`, so `workspace.project().clone()` (a
    /// field access, not an entity lease) hands us the project conflict-free.
    pub fn new(
        workspace: WeakEntity<Workspace>,
        project: Entity<Project>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut subscriptions = Vec::new();

        // Re-render when any agent's threads load/change so the history list
        // reflects the latest set (see the struct doc for why this is the
        // correct store).
        subscriptions
            .push(cx.observe(&ThreadMetadataStore::global(cx), |_, _, cx| cx.notify()));

        // Re-render the folder zone and re-scope the history list when folders
        // are added to / removed from the window's project.
        subscriptions.push(cx.subscribe(&project, |_, _, event, cx| match event {
            project::Event::WorktreeAdded(_)
            | project::Event::WorktreeRemoved(_)
            | project::Event::WorktreeOrderChanged => cx.notify(),
            _ => {}
        }));

        Self {
            workspace,
            focus_handle: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    /// Builds a fresh VibeDev conversation (optionally resuming an existing
    /// thread by its `acp::SessionId`) and swaps it into the AgentApp window's
    /// center pane, replacing whatever conversation was previously there.
    ///
    /// `resume_session_id` is `None` for a brand-new conversation; for a list
    /// entry it is the thread's session id, which drives `ConversationView`'s
    /// load/resume path (see `conversation_view::new_thread_view`).
    ///
    /// `work_dirs` is left `None`: `ConversationView::initial_state` falls back
    /// to `project.default_path_list(cx)` (`project.rs:2415`), which returns the
    /// window's worktree roots when any folder is mounted. So once a folder is
    /// added, new conversations automatically root to it without extra wiring.
    fn open_conversation(
        &mut self,
        resume_session_id: Option<acp::SessionId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(thread_store) = agent::ThreadStore::try_global(cx) else {
            return;
        };
        let Some(workspace) = self.workspace.upgrade() else {
            return;
        };

        workspace.update(cx, |workspace, cx| {
            let project = workspace.project().clone();
            let fs = workspace.app_state().fs.clone();
            // Like `configure_agent_mode`, stand up a fresh per-conversation
            // connection store rather than reusing a global one.
            let connection_store = cx.new(|cx| AgentConnectionStore::new(project.clone(), cx));

            let conversation = create_conversation_view(
                workspace.weak_handle(),
                project,
                connection_store,
                fs,
                thread_store,
                Agent::Custom {
                    id: AgentId::new("VibeDev"),
                },
                None,
                None,
                resume_session_id,
                None,
                Some(SharedString::from("VibeDev")),
                None,
                AgentThreadSource::Sidebar,
                window,
                cx,
            );
            let item = cx.new(|cx| VibedevConversationItem::new(conversation, cx));

            // Swap the center: add the new conversation first, then drop the
            // previously-hosted items so the AgentApp window shows exactly one
            // conversation. The new item is added + activated; removing the rest
            // with `close_pane_if_empty = false` keeps the pane alive.
            let center_pane = workspace.active_pane().clone();
            let new_item_id = item.entity_id();
            center_pane.update(cx, |pane, cx| {
                pane.add_item(Box::new(item), true, true, None, window, cx);
                let stale_ids: Vec<_> = pane
                    .items()
                    .map(|item| item.item_id())
                    .filter(|id| *id != new_item_id)
                    .collect();
                for id in stale_ids {
                    pane.remove_item(id, false, false, window, cx);
                }
            });
        });
    }

    fn render_new_conversation_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        Button::new("vibedev-new-conversation", "New Conversation")
            .start_icon(Icon::new(IconName::Plus))
            .full_width()
            .on_click(cx.listener(|this, _event, window, cx| {
                this.open_conversation(None, window, cx);
            }))
    }

    /// Multi-root "Folders" zone: lists the window's visible worktrees with a
    /// remove button each, plus a header "+" that triggers the standard
    /// `AddFolderToProject` flow (folder picker + worktree add).
    fn render_folders(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().colors().text_muted;
        let hover_bg = cx.theme().colors().element_hover;

        // (display name, id, full path) for each mounted folder.
        let folders: Vec<(SharedString, WorktreeId, SharedString)> = self
            .workspace
            .upgrade()
            .map(|workspace| {
                workspace
                    .read(cx)
                    .visible_worktrees(cx)
                    .map(|worktree| Self::folder_row_info(&worktree, cx))
                    .collect()
            })
            .unwrap_or_default();

        let header = h_flex()
            .w_full()
            .px_2()
            .py_1()
            .justify_between()
            .child(
                Label::new("Folders")
                    .size(LabelSize::Small)
                    .color(Color::Muted),
            )
            .child(
                IconButton::new("vibedev-add-folder", IconName::Plus)
                    .icon_size(IconSize::Small)
                    .tooltip(Tooltip::text("Add Folder to Project"))
                    .on_click(cx.listener(|_, _event, window, cx| {
                        // Dispatches to `Workspace::add_folder_to_project`
                        // (registered on every workspace at workspace.rs:7294),
                        // which opens the folder picker and adds a worktree.
                        window.dispatch_action(Box::new(workspace::AddFolderToProject), cx);
                    })),
            );

        let mut list = v_flex().w_full().gap_px();
        list = list.child(header);

        if folders.is_empty() {
            return list.child(
                div().px_2().py_1().child(
                    Label::new("No folder — add one")
                        .size(LabelSize::Small)
                        .color(Color::Muted),
                ),
            );
        }

        for (name, worktree_id, full_path) in folders {
            list = list.child(
                h_flex()
                    .id(SharedString::from(format!("vibedev-folder-{}", worktree_id)))
                    .w_full()
                    .px_2()
                    .py_1()
                    .gap_1()
                    .rounded_md()
                    .justify_between()
                    .text_color(muted)
                    .hover(|style| style.bg(hover_bg))
                    .child(
                        Label::new(name)
                            .size(LabelSize::Small)
                            .truncate(),
                    )
                    .child(
                        IconButton::new(
                            SharedString::from(format!("vibedev-remove-folder-{}", worktree_id)),
                            IconName::Trash,
                        )
                        .icon_size(IconSize::Small)
                        .tooltip(Tooltip::text(full_path))
                        .on_click(cx.listener(move |this, _event, _window, cx| {
                            this.remove_folder(worktree_id, cx);
                        })),
                    ),
            );
        }

        list
    }

    /// Derives the display name (last path segment) and full path for a
    /// worktree folder row.
    fn folder_row_info(
        worktree: &gpui::Entity<Worktree>,
        cx: &App,
    ) -> (SharedString, WorktreeId, SharedString) {
        let worktree = worktree.read(cx);
        let abs_path = worktree.abs_path();
        let name = abs_path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| abs_path.to_string_lossy().to_string());
        let full_path = abs_path.to_string_lossy().to_string();
        (
            SharedString::from(name),
            worktree.id(),
            SharedString::from(full_path),
        )
    }

    /// Removes a folder (worktree) from the window's project. The
    /// `WorktreeRemoved` event the project emits re-renders this sidebar via the
    /// subscription set up in `new`.
    fn remove_folder(&mut self, worktree_id: WorktreeId, cx: &mut Context<Self>) {
        let Some(workspace) = self.workspace.upgrade() else {
            return;
        };
        let project = workspace.read(cx).project().clone();
        project.update(cx, |project, cx| {
            project.remove_worktree(worktree_id, cx);
        });
    }

    fn render_session_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        // History entries as `(session_id, title)`. `session_id` is `None` for
        // the current draft thread (not yet sent); clicking it is a no-op since
        // it is already the active conversation.
        let entries = self.session_entries(cx);

        let muted = cx.theme().colors().text_muted;
        let hover_bg = cx.theme().colors().element_hover;

        let mut list = v_flex().flex_1().w_full().gap_px().overflow_hidden();

        if entries.is_empty() {
            return list.child(
                div().px_2().py_1().child(
                    Label::new("No conversations yet")
                        .size(LabelSize::Small)
                        .color(Color::Muted),
                ),
            );
        }

        for (index, (session_id, title)) in entries.into_iter().enumerate() {
            // `session_id` is unique per real thread, but draft rows all carry
            // `None`; fall back to the index so element ids stay unique.
            let element_id = match &session_id {
                Some(session_id) => SharedString::from(format!("vibedev-session-{}", session_id)),
                None => SharedString::from(format!("vibedev-session-draft-{}", index)),
            };
            list = list.child(
                div()
                    .id(element_id)
                    .w_full()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .cursor_pointer()
                    .text_color(muted)
                    .overflow_hidden()
                    .text_ellipsis()
                    .hover(|style| style.bg(hover_bg))
                    .child(Label::new(title).size(LabelSize::Small))
                    .on_click(cx.listener(move |this, _event, window, cx| {
                        // Draft rows (no session id) are the live conversation;
                        // resuming would be a no-op, so do nothing.
                        if let Some(session_id) = session_id.clone() {
                            this.open_conversation(Some(session_id), window, cx);
                        }
                    })),
            );
        }

        list
    }

    /// Reads history from `ThreadMetadataStore`, scoping to the window's folders
    /// when any are mounted, and sorts most-recently-interacted first.
    ///
    /// When the window has visible worktrees, `entries_for_path` is used (and it
    /// excludes archived threads — fine, because threads sent with folders
    /// mounted aren't auto-archived). When the window has no folder, the broad
    /// `entries()` is used so the just-sent conversation is still visible even
    /// though `handle_conversation_event` auto-archives folder-less threads
    /// (`thread_metadata_store.rs:1325`); `entries()` includes archived threads.
    fn session_entries(&self, cx: &App) -> Vec<(Option<acp::SessionId>, SharedString)> {
        let store = ThreadMetadataStore::global(cx);
        let store = store.read(cx);

        let workspace = match self.workspace.upgrade() {
            Some(workspace) => workspace,
            None => return Vec::new(),
        };
        let workspace = workspace.read(cx);
        let has_folders = workspace.visible_worktrees(cx).next().is_some();

        // For a local project this is `None`; we mirror the official sidebar's
        // derivation (sidebar.rs:600) so remote windows would behave too.
        let project = workspace.project().read(cx);
        let remote_connection = project.remote_connection_options(cx);

        let mut metadatas: Vec<_> = if has_folders {
            let path_list = PathList::new(&workspace.root_paths(cx));
            store
                .entries_for_path(&path_list, remote_connection.as_ref())
                .cloned()
                .collect()
        } else {
            store.entries().cloned().collect()
        };

        // Most-recently-interacted first; fall back to `updated_at` for threads
        // never interacted with (e.g. resumed-but-untouched).
        metadatas.sort_by_key(|metadata| {
            std::cmp::Reverse(metadata.interacted_at.unwrap_or(metadata.updated_at))
        });

        metadatas
            .into_iter()
            .map(|metadata| (metadata.session_id.clone(), metadata.display_title()))
            .collect()
    }
}

impl Focusable for VibedevSessionSidebar {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<()> for VibedevSessionSidebar {}

impl Render for VibedevSessionSidebar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let border = cx.theme().colors().border;
        let bg = cx.theme().colors().panel_background;

        v_flex()
            .w(px(248.))
            .flex_none()
            .h_full()
            .bg(bg)
            .border_r_1()
            .border_color(border)
            .child(
                div()
                    .w_full()
                    .px_1()
                    .pt_1()
                    .child(self.render_folders(cx)),
            )
            .child(
                div()
                    .w_full()
                    .px_2()
                    .py_1p5()
                    .child(
                        Label::new("Conversations")
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .px_2()
                    .pb_1p5()
                    .child(self.render_new_conversation_button(cx)),
            )
            .child(
                div()
                    .id("vibedev-session-list")
                    .flex_1()
                    .w_full()
                    .px_1()
                    .overflow_y_scroll()
                    .child(self.render_session_list(cx)),
            )
    }
}
