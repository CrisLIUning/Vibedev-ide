use gpui::{App, Global};

/// The subagent currently drilled-into in the AgentApp Execution detail panel.
/// Written by the conversation fan-out card `on_click` (`ThreadView`), observed
/// by `VibedevRightDock`. Lives in `agent_ui` because both `zed` (the dock) and
/// `agent_ui` (the cards) depend on it, without polluting the `acp_thread`
/// protocol model.
#[derive(Clone, Default)]
pub struct SelectedSubagent(pub Option<String>);

impl Global for SelectedSubagent {}

impl SelectedSubagent {
    pub fn get(cx: &App) -> Option<String> {
        cx.try_global::<SelectedSubagent>().and_then(|g| g.0.clone())
    }

    /// `set_global` (not `update_global`) so it is safe even before the first
    /// write (`update_global` panics if the global is absent). Writing the
    /// global triggers any registered global observers.
    pub fn set(id: Option<String>, cx: &mut App) {
        cx.set_global(SelectedSubagent(id));
    }
}
