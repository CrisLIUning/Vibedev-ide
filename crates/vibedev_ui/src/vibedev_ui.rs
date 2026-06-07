mod account_panel;
mod agent_update_notice;
mod cost_status_item;
mod global_skills;
mod usage_dashboard;

pub use account_panel::VibedevAccountPanel;
pub use cost_status_item::VibedevCostStatusItem;
pub use usage_dashboard::VibedevUsageDashboard;

use std::any::TypeId;

use command_palette_hooks::CommandPaletteFilter;
use gpui::App;
use vibedev_account::VibedevSidecar;
use workspace::Workspace;

/// Called from `main.rs` next to `project_panel::init`. Spawns + supervises the
/// backend sidecar (which owns the sub2api signing key and token), and registers
/// the panels' toggle actions so the command palette / keybindings can focus
/// them.
pub fn init(cx: &mut App) {
    // Spawn the long-lived backend sidecar. The handle lives in a global so the
    // supervisor task survives for the lifetime of the app rather than being
    // dropped (which would tear the sidecar down). Account/providers/cost all
    // talk to it over loopback using the handshake token.
    let sidecar = VibedevSidecar::spawn(cx);
    cx.set_global(GlobalVibedevSidecar(sidecar));

    // VIBEDEV (agent-OTA): show a corner notice (+ "立即应用" action) when the
    // auto-updater stages a new backend build. `auto_update::init` ran earlier in
    // main.rs, so the AutoUpdater entity this observes is already registered.
    agent_update_notice::init(cx);

    cx.observe_new(|workspace: &mut Workspace, _, cx| {
        workspace.register_action(
            |workspace, _: &zed_actions::vibedev::ToggleAccountFocus, window, cx| {
                workspace.toggle_panel_focus::<VibedevAccountPanel>(window, cx);
            },
        );

        workspace.register_action(
            |workspace, _: &zed_actions::vibedev::ToggleUsageDashboard, window, cx| {
                workspace.toggle_panel_focus::<VibedevUsageDashboard>(window, cx);
            },
        );

        // VIBEDEV: publish ~/.vibedev/skills into the SkillIndex so the
        // `#agent.skills` settings page lists them. The NativeAgent path that
        // normally does this doesn't run under our ACP agent. See global_skills.rs.
        global_skills::refresh_global_skill_index(workspace, cx);
    })
    .detach();

    // VIBEDEV: hide Zed-specific commands from the command palette — they point at
    // Zed's GitHub repo / feedback inbox / the `zed://` URL scheme / Zed-AI edit-
    // prediction onboarding, none of which apply to VibeDev. Display-only: the
    // actions still exist (keymaps unaffected), they're just not listed in the
    // palette. `command_palette::init` runs earlier in main.rs (677 vs 745), so
    // the filter global is already set when this runs.
    CommandPaletteFilter::update_global(cx, |filter, _| {
        filter.hide_action_types(&[
            TypeId::of::<zed_actions::feedback::EmailZed>(),
            TypeId::of::<feedback::OpenZedRepo>(),
            TypeId::of::<install_cli::RegisterZedScheme>(),
            TypeId::of::<zed_actions::OpenZedPredictOnboarding>(),
        ]);
    });
}

struct GlobalVibedevSidecar(VibedevSidecar);

impl gpui::Global for GlobalVibedevSidecar {}

/// VIBEDEV (agent-OTA): forward an "apply now" restart to the running sidecar.
/// Called from the agent-update notice's "立即应用" click handler. Uses
/// `try_global` so it's a safe no-op when the sidecar was never initialized
/// (e.g. in tests), and `request_restart` is itself best-effort (a closed
/// channel during shutdown is harmless).
fn request_sidecar_restart(cx: &App) {
    if let Some(global) = cx.try_global::<GlobalVibedevSidecar>() {
        global.0.request_restart();
    }
}
