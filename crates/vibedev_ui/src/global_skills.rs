//! VIBEDEV: populate the `agent.skills` settings page for the ACP agent.
//!
//! Zed's native skill index (`agent_skills::SkillIndex` global, read by
//! `settings_ui`'s skills page) is only ever filled by `NativeAgent`'s
//! `publish_skill_index`. VibeDev runs an external ACP agent instead of the
//! NativeAgent, so that path never runs and the "Manage Skills" / `#agent.skills`
//! page shows "no global skills installed" even though the ccb backend loads
//! and serves them from `~/.vibedev/skills`.
//!
//! Here we scan `agent_skills::global_skills_dir()` (which we aligned to
//! `~/.vibedev/skills` — see PATCHES.md) once per workspace open and publish the
//! result into `SkillIndex.global_skills`, preserving any `project_skills` the
//! NativeAgent path may have set. This keeps the settings UI honest about which
//! global skills exist without coupling to the NativeAgent lifecycle.

use agent_skills::{SkillIndex, SkillSource, global_skills_dir, load_skills_from_directory};
use gpui::App;
use workspace::Workspace;

/// Scan the global skills directory and publish the result into the
/// `SkillIndex` global so the settings skills page reflects reality. Runs in
/// the background; the global is updated on the foreground once the scan
/// completes. Best-effort — a missing directory just yields an empty list.
pub(crate) fn refresh_global_skill_index(workspace: &Workspace, cx: &mut App) {
    let fs = workspace.app_state().fs.clone();
    cx.spawn(async move |cx| {
        let dir = global_skills_dir();
        let results = load_skills_from_directory(&fs, &dir, SkillSource::Global).await;
        let global_skills = results.into_iter().filter_map(Result::ok).collect::<Vec<_>>();

        let _ = cx.update(|cx| {
            // Preserve project_skills if the NativeAgent path (or a prior run)
            // already populated them; only replace the global list.
            let project_skills = cx
                .try_global::<SkillIndex>()
                .map(|idx| idx.project_skills.clone())
                .unwrap_or_default();
            cx.set_global(SkillIndex {
                global_skills,
                project_skills,
            });
        });
    })
    .detach();
}
