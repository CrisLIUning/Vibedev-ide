use std::rc::Rc;
use std::sync::Arc;

use agent_client_protocol::schema as acp;
use agent_servers::AgentServer;
use gpui::{App, AppContext as _, Entity, SharedString, WeakEntity, Window};
use project::Project;
use workspace::{PathList, Workspace};

use crate::{
    Agent, AgentInitialContent, AgentThreadSource,
    agent_connection_store::AgentConnectionStore,
    conversation_view::ConversationView,
    thread_metadata_store::ThreadId,
};

/// Creates a standalone `ConversationView` entity without any AgentPanel-specific side
/// effects (no panel state persistence, no sibling-host installation, no events).
/// Other surfaces that need to host an agent conversation should use this instead of
/// duplicating the construction logic.
pub fn create_conversation_view(
    workspace: WeakEntity<Workspace>,
    project: Entity<Project>,
    connection_store: Entity<AgentConnectionStore>,
    fs: Arc<dyn fs::Fs>,
    thread_store: Entity<agent::ThreadStore>,
    agent: Agent,
    server_override: Option<Rc<dyn AgentServer>>,
    resume_thread_id: Option<ThreadId>,
    resume_session_id: Option<acp::SessionId>,
    work_dirs: Option<PathList>,
    title: Option<SharedString>,
    initial_content: Option<AgentInitialContent>,
    source: AgentThreadSource,
    window: &mut Window,
    cx: &mut App,
) -> Entity<ConversationView> {
    let thread_id = resume_thread_id.unwrap_or_else(ThreadId::new);
    let server = server_override
        .unwrap_or_else(|| agent.server(fs.clone(), thread_store.clone()));
    let native_thread_store = server
        .clone()
        .downcast::<agent::NativeAgentServer>()
        .is_some()
        .then(|| thread_store.clone());

    cx.new(|cx| {
        ConversationView::new(
            server,
            connection_store,
            agent,
            resume_session_id,
            Some(thread_id),
            work_dirs,
            title,
            initial_content,
            workspace,
            project,
            native_thread_store,
            source,
            window,
            cx,
        )
    })
}
