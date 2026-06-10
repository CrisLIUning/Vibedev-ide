//! The VibeDev AgentApp landing ("home") view.
//!
//! Shown as the AgentApp window's center content when the workspace has no
//! project yet (a fresh launch with the default startup surface set to
//! AgentApp, or an explicitly opened AgentApp window). Without it the window
//! was a dead end: the threads sidebar, the `AgentPanel`, and the right dock
//! all need a project, so a projectless window showed three different "open a
//! project" empty states and no way to start a conversation.
//!
//! Opening a project from here goes through `MultiWorkspace::open_project`,
//! which REPLACES the empty workspace with the project's workspace; the new
//! workspace is then converted to an agent surface by the window's
//! `ActiveWorkspaceChanged` handler (see `vibedev_agent_window`) and gets the
//! real `AgentPanel` conversation as its center. This view therefore never has
//! to "upgrade" itself in place.
//!
//! This is the pragmatic first slice of the designed agent home (see
//! `design-mockups/vibedev-agent-page.html`): brand hero, quick actions,
//! recent projects, and — when signed in — a compact usage KPI strip. The full
//! overview (usage heatmap, time ranges, per-model breakdown) needs sidecar
//! time-series endpoints and lands later.

use std::sync::Arc;

use fs::Fs;
use gpui::{Action as _, App, Context, FocusHandle, Focusable, Render, Task, WeakEntity, Window};
use ui::prelude::*;
use util::ResultExt as _;
use vibedev_account::AccountSnapshot;
use workspace::{
    MultiWorkspace, OpenMode, RecentWorkspace, SerializedWorkspaceLocation, WorkspaceDb,
};

pub struct VibedevAgentHome {
    recent_workspaces: Option<Vec<RecentWorkspace>>,
    account: Option<AccountSnapshot>,
    /// Focused on install (see `apply_agent_surface`). Without SOME focus
    /// inside the window, `window.dispatch_action` resolves along an empty
    /// focus path and never reaches the MultiWorkspace-level node that carries
    /// every `Workspace::register_action` handler — so titlebar actions like
    /// "Open IDE" silently did nothing while this landing page was shown.
    focus_handle: FocusHandle,
    /// Keep the loads alive for the view's lifetime; dropped with it.
    _load: [Task<()>; 2],
}

impl VibedevAgentHome {
    pub fn new(fs: Arc<dyn Fs>, cx: &mut Context<Self>) -> Self {
        // Same source as the welcome page's "Recent Projects" section.
        let db = WorkspaceDb::global(cx);
        let recents = cx.spawn(async move |this: WeakEntity<Self>, cx| {
            let workspaces = db
                .recent_project_workspaces(fs.as_ref())
                .await
                .log_err()
                .unwrap_or_default();
            this.update(cx, |this, cx| {
                this.recent_workspaces = Some(workspaces);
                cx.notify();
            })
            .ok();
        });
        // Account snapshot for the KPI strip. The view is typically created at
        // app launch, BEFORE the sidecar finishes its ~5s startup handshake, so
        // a single fetch would always miss — retry over a short window instead.
        // Still best-effort: not signed in / backend down simply leaves the
        // strip out (the account panel owns the full polling + retry story).
        let http = cx.http_client();
        let account = cx.spawn(async move |this: WeakEntity<Self>, cx| {
            for _attempt in 0..8 {
                if let Ok(snapshot) = vibedev_account::fetch_account(http.clone()).await {
                    this.update(cx, |this, cx| {
                        this.account = Some(snapshot);
                        cx.notify();
                    })
                    .ok();
                    return;
                }
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(2))
                    .await;
            }
        });
        Self {
            recent_workspaces: None,
            account: None,
            focus_handle: cx.focus_handle(),
            _load: [recents, account],
        }
    }

    fn open_recent(workspace: &RecentWorkspace, window: &mut Window, cx: &mut App) {
        match workspace.location {
            SerializedWorkspaceLocation::Local => {
                let paths = workspace.paths.paths().to_vec();
                // Mirror `sidebar_recent_projects`: open within THIS window's
                // MultiWorkspace, which replaces the empty workspace instead of
                // routing to (or opening) an editor window.
                //
                // Deferred, NOT immediate: this runs inside the window's own
                // click dispatch, so the window is already on the update stack
                // and an immediate `handle.update()` on it fails (silently,
                // via `log_err`) — clicks would appear to do nothing. Same
                // window-on-the-stack class as the documented
                // `open_ide_window` panic; same `cx.defer` cure as
                // `configure_agent_mode`'s deferred sidebar open.
                if let Some(handle) = window.window_handle().downcast::<MultiWorkspace>() {
                    cx.defer(move |cx| {
                        handle
                            .update(cx, |multi_workspace, window, cx| {
                                multi_workspace
                                    .open_project(paths, OpenMode::Activate, window, cx)
                                    .detach_and_log_err(cx);
                            })
                            .log_err();
                    });
                }
            }
            SerializedWorkspaceLocation::Remote(_) => {
                window.dispatch_action(zed_actions::OpenRecent::default().boxed_clone(), cx);
            }
        }
    }

    /// "Open Project": run the canonical in-window open flow (picker ->
    /// `MultiWorkspace::open_project`, which replaces this projectless
    /// workspace). NOT an `Open` action dispatch — dispatched actions resolve
    /// along the focus path, which from this center view missed the workspace
    /// handler and fell through to the app-global handler (= a NEW editor
    /// window). Deferred for the same window-on-the-stack reason as
    /// `open_recent`.
    fn open_project_picker(window: &mut Window, cx: &mut App) {
        let Some(handle) = window.window_handle().downcast::<MultiWorkspace>() else {
            return;
        };
        cx.defer(move |cx| {
            handle
                .update(cx, |multi_workspace, window, cx| {
                    let workspace = multi_workspace.workspace().clone();
                    let app_state = workspace.read(cx).app_state().clone();
                    workspace.update(cx, |workspace, cx| {
                        workspace::prompt_for_open_path_and_open(
                            workspace,
                            app_state,
                            gpui::PathPromptOptions {
                                files: true,
                                directories: true,
                                multiple: true,
                                prompt: None,
                            },
                            false,
                            window,
                            cx,
                        );
                    });
                })
                .log_err();
        });
    }

    fn render_quick_action(
        id: &'static str,
        icon: IconName,
        label: &'static str,
        on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let hover_bg = cx.theme().colors().element_hover;
        let border = cx.theme().colors().border_variant;
        h_flex()
            .id(id)
            .gap_2()
            .px_3()
            .py_2()
            .rounded_md()
            .border_1()
            .border_color(border)
            .cursor_pointer()
            .hover(move |style| style.bg(hover_bg))
            .child(Icon::new(icon).color(Color::Muted).size(IconSize::Small))
            .child(Label::new(label))
            .on_click(on_click)
    }

    fn render_kpi(label: &'static str, value: String) -> impl IntoElement {
        v_flex()
            .gap_0p5()
            .items_center()
            .flex_1()
            .child(Label::new(value).size(LabelSize::Large))
            .child(Label::new(label).size(LabelSize::Small).color(Color::Muted))
    }
}

/// `1234567` -> `1.2M`, `12345` -> `12.3k`. Matches the account panel's
/// compact-count convention (duplicated: that helper is private to
/// `vibedev_ui` and not worth a cross-crate export for two lines).
fn compact_count(count: u64) -> String {
    if count >= 1_000_000 {
        format!("{:.1}M", count as f64 / 1_000_000.0)
    } else if count >= 1_000 {
        format!("{:.1}k", count as f64 / 1_000.0)
    } else {
        count.to_string()
    }
}

fn format_usd(value: f64) -> String {
    format!("${value:.2}")
}

impl Focusable for VibedevAgentHome {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for VibedevAgentHome {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        let editor_bg = colors.editor_background;
        let border = colors.border_variant;
        let hover_bg = colors.element_hover;

        let recents: Vec<_> = self
            .recent_workspaces
            .as_ref()
            .into_iter()
            .flatten()
            .take(6)
            .cloned()
            .collect();

        let kpis = self
            .account
            .as_ref()
            .filter(|snapshot| snapshot.signed_in)
            .and_then(|snapshot| snapshot.usage.as_ref())
            .map(|usage| {
                h_flex()
                    .w_full()
                    .gap_2()
                    .p_3()
                    .rounded_lg()
                    .border_1()
                    .border_color(border)
                    .child(Self::render_kpi(
                        "Tokens",
                        compact_count(usage.total_tokens),
                    ))
                    .child(Self::render_kpi(
                        "Requests",
                        compact_count(usage.total_requests),
                    ))
                    .child(Self::render_kpi("Cost", format_usd(usage.total_cost)))
                    .child(Self::render_kpi(
                        "Latency",
                        format!("{:.0}ms", usage.avg_latency_ms),
                    ))
            });

        v_flex()
            .track_focus(&self.focus_handle)
            .size_full()
            .items_center()
            .justify_center()
            .bg(editor_bg)
            .child(
                v_flex()
                    .w(px(440.))
                    .gap_4()
                    // Brand hero.
                    .child(
                        v_flex()
                            .items_center()
                            .gap_1()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .child(
                                        Icon::new(IconName::Thread)
                                            .color(Color::Accent)
                                            .size(IconSize::XLarge),
                                    )
                                    .child(
                                        Label::new("VibeDev").size(LabelSize::Large).weight(
                                            gpui::FontWeight::BOLD,
                                        ),
                                    ),
                            )
                            .child(
                                Label::new("Open a project to start a conversation")
                                    .size(LabelSize::Small)
                                    .color(Color::Muted),
                            ),
                    )
                    // Quick actions. (No "Clone Repository" here: the clone
                    // modal needs a `GitPanel`, which a projectless AgentApp
                    // workspace doesn't have — the action handler bails out
                    // silently, so the button would be a no-op.)
                    .child(
                        h_flex()
                            .gap_2()
                            .justify_center()
                            .child(Self::render_quick_action(
                                "agent-home-open-project",
                                IconName::FolderOpen,
                                "Open Project",
                                |_event, window, cx| Self::open_project_picker(window, cx),
                                cx,
                            )),
                    )
                    // Usage KPI strip (signed-in only).
                    .children(kpis)
                    // Recent projects.
                    .when(!recents.is_empty(), |this| {
                        this.child(
                            v_flex()
                                .gap_1()
                                .child(
                                    Label::new("Recent Projects")
                                        .size(LabelSize::Small)
                                        .color(Color::Muted),
                                )
                                .children(recents.into_iter().enumerate().map(
                                    |(index, workspace)| {
                                        // Display identity: `identity_paths` (the project's
                                        // canonical identity — welcome.rs uses the same), while
                                        // OPENING uses `paths` (the raw worktree set).
                                        let name = workspace
                                            .identity_paths
                                            .paths()
                                            .first()
                                            .and_then(|path| path.file_name())
                                            .map(|name| name.to_string_lossy().into_owned())
                                            .unwrap_or_else(|| "Project".to_string());
                                        let path_label = workspace
                                            .identity_paths
                                            .paths()
                                            .first()
                                            .map(|path| path.to_string_lossy().into_owned())
                                            .unwrap_or_default();
                                        let icon = match workspace.location {
                                            SerializedWorkspaceLocation::Local => IconName::Folder,
                                            SerializedWorkspaceLocation::Remote(_) => {
                                                IconName::Server
                                            }
                                        };
                                        h_flex()
                                            .id(("agent-home-recent", index))
                                            .gap_2()
                                            .px_2()
                                            .py_1p5()
                                            .rounded_md()
                                            .cursor_pointer()
                                            .hover(move |style| style.bg(hover_bg))
                                            .child(
                                                Icon::new(icon)
                                                    .color(Color::Muted)
                                                    .size(IconSize::Small),
                                            )
                                            .child(Label::new(name))
                                            .child(
                                                Label::new(path_label)
                                                    .size(LabelSize::Small)
                                                    .color(Color::Muted)
                                                    .truncate(),
                                            )
                                            .on_click(move |_event, window, cx| {
                                                Self::open_recent(&workspace, window, cx);
                                            })
                                    },
                                )),
                        )
                    }),
            )
    }
}
