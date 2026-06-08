use agent_client_protocol::schema as acp;
use agent_ui::{Agent, AgentConnectionStore, AgentThreadSource, create_conversation_view};
use gpui::{
    App, AppContext as _, Context, EventEmitter, FocusHandle, Focusable, IntoElement,
    ParentElement, Render, SharedString, Styled, WeakEntity, Window, px,
};
use project::AgentId;
use ui::prelude::*;
use workspace::Workspace;

use crate::VibedevConversationItem;

/// Left session sidebar for the VibeDev AgentApp window: a branded header, a
/// "new conversation" button, and a list of the user's existing threads. It is
/// constructed in the `zed` crate (which can depend on `agent`/`agent_ui`) and
/// injected into the `workspace` crate's `render_agent_layout` left slot via
/// `Workspace::set_agent_left_sidebar`, mirroring the `titlebar_item` pattern.
///
/// Held views: only a `WeakEntity<Workspace>`, so click handlers can swap the
/// hosted conversation in the center pane without owning the workspace.
pub struct VibedevSessionSidebar {
    workspace: WeakEntity<Workspace>,
    focus_handle: FocusHandle,
}

impl VibedevSessionSidebar {
    pub fn new(workspace: WeakEntity<Workspace>, cx: &mut Context<Self>) -> Self {
        // Re-render when threads load/change so the list reflects the latest set.
        if let Some(thread_store) = agent::ThreadStore::try_global(cx) {
            cx.observe(&thread_store, |_, _, cx| cx.notify()).detach();
        }
        Self {
            workspace,
            focus_handle: cx.focus_handle(),
        }
    }

    /// Builds a fresh VibeDev conversation (optionally resuming an existing
    /// thread by its `acp::SessionId`) and swaps it into the AgentApp window's
    /// center pane, replacing whatever conversation was previously there.
    ///
    /// `resume_session_id` is `None` for a brand-new conversation; for a list
    /// entry it is the thread's session id, which drives `ConversationView`'s
    /// load/resume path (see `conversation_view::new_thread_view`).
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

    fn render_session_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entries: Vec<(acp::SessionId, SharedString)> = agent::ThreadStore::try_global(cx)
            .map(|store| {
                store
                    .read(cx)
                    .entries()
                    .map(|thread| (thread.id, thread.title))
                    .collect()
            })
            .unwrap_or_default();

        let muted = cx.theme().colors().text_muted;
        let hover_bg = cx.theme().colors().element_hover;

        let mut list = v_flex().flex_1().w_full().gap_px().overflow_hidden();

        if entries.is_empty() {
            return list.child(
                div()
                    .px_2()
                    .py_1()
                    .child(Label::new("No conversations yet").size(LabelSize::Small).color(Color::Muted)),
            );
        }

        for (session_id, title) in entries {
            let row_title = if title.is_empty() {
                SharedString::from("Untitled")
            } else {
                title
            };
            list = list.child(
                div()
                    .id(SharedString::from(format!("vibedev-session-{}", session_id)))
                    .w_full()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .cursor_pointer()
                    .text_color(muted)
                    .overflow_hidden()
                    .text_ellipsis()
                    .hover(|style| style.bg(hover_bg))
                    .child(Label::new(row_title).size(LabelSize::Small))
                    .on_click(cx.listener({
                        let session_id = session_id.clone();
                        move |this, _event, window, cx| {
                            this.open_conversation(Some(session_id.clone()), window, cx);
                        }
                    })),
            );
        }

        list
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
