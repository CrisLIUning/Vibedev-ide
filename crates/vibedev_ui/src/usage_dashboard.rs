use std::sync::Arc;

use anyhow::Result;
use gpui::{
    AsyncWindowContext, Entity, EventEmitter, FocusHandle, Focusable, Task, WeakEntity,
};
use http_client::HttpClient;
use ui::{Label, prelude::*};
use vibedev_account::{ACCOUNT_POLL_INTERVAL, AccountSnapshot, AccountUsage};
use workspace::{
    Workspace,
    dock::{DockPosition, Panel, PanelEvent},
};

const VIBEDEV_USAGE_DASHBOARD_KEY: &str = "VibedevUsageDashboard";

// ─── Usage dashboard ────────────────────────────────────────────────────────
//
// A focused read-only view of the gateway usage KPIs. It reuses the same
// `GET /vibedev/account` snapshot the account panel polls, but renders only the
// usage KPI row (tokens / requests / cost / latency) — no sign-in/out, balance,
// or subscription chrome. The KPI cards + compaction helpers are deliberately
// copied from `account_panel.rs` (kept self-contained; the small duplication is
// an intentional trade-off rather than coupling the two panels).
//
// FUTURE (v2 ACP usage enhancement): the sidecar will grow per-request cache
// counters (cache-read / cache-write tokens + hit rate) once the ACP usage
// stream is wired through. Those become extra KPI cards here; the snapshot does
// not carry them today, so this pass renders only the fields that exist on
// `AccountUsage`.

/// Where the data shown in the dashboard currently stands. Distinguishes "still
/// connecting to the sidecar" from "sidecar unreachable" so the render path can
/// show the right message instead of a bare error.
#[derive(Clone)]
enum UsageStatus {
    Connecting,
    BackendUnavailable,
    Loaded(AccountSnapshot),
}

pub struct VibedevUsageDashboard {
    focus_handle: FocusHandle,
    http: Arc<dyn HttpClient>,
    status: UsageStatus,
    /// Periodic snapshot poll; dropped (cancelled) when the panel is dropped.
    _poll: Task<()>,
}

impl VibedevUsageDashboard {
    fn new(
        _workspace: &mut Workspace,
        _window: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> Entity<Self> {
        let http = cx.http_client();
        cx.new(|cx| {
            let poll = Self::spawn_poll(http.clone(), cx);
            Self {
                focus_handle: cx.focus_handle(),
                http,
                status: UsageStatus::Connecting,
                _poll: poll,
            }
        })
    }

    /// Polls `fetch_account` on an interval and pushes the snapshot into
    /// `status`. The sidecar is the source of truth for every usage counter; we
    /// never aggregate locally.
    fn spawn_poll(http: Arc<dyn HttpClient>, cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |this, cx| {
            loop {
                let snapshot = vibedev_account::fetch_account(http.clone()).await;
                let updated = this.update(cx, |this, cx| {
                    this.status = match snapshot {
                        Ok(snapshot) => UsageStatus::Loaded(snapshot),
                        Err(_) => UsageStatus::BackendUnavailable,
                    };
                    cx.notify();
                });
                if updated.is_err() {
                    break;
                }
                cx.background_executor().timer(ACCOUNT_POLL_INTERVAL).await;
            }
        })
    }

    pub async fn load(
        workspace: WeakEntity<Workspace>,
        mut cx: AsyncWindowContext,
    ) -> Result<Entity<Self>> {
        workspace.update_in(&mut cx, |workspace, window, cx| {
            VibedevUsageDashboard::new(workspace, window, cx)
        })
    }

    /// Trigger one immediate refresh (the periodic poll would otherwise leave
    /// the KPIs stale for up to a full interval after the user clicks Refresh).
    fn refresh_now(&self, cx: &mut Context<Self>) {
        let http = self.http.clone();
        cx.spawn(async move |this, cx| {
            let snapshot = vibedev_account::fetch_account(http).await;
            this.update(cx, |this, cx| {
                this.status = match snapshot {
                    Ok(snapshot) => UsageStatus::Loaded(snapshot),
                    Err(_) => UsageStatus::BackendUnavailable,
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// KPI row (mirrors the account panel's `render_kpi_row`): four label / big
    /// value / sub-line cards (Tokens, Requests, Cost, Latency). Wraps so the
    /// cards reflow in the narrow right dock instead of overflowing.
    fn render_kpi_row(&self, usage: &AccountUsage, cx: &Context<Self>) -> impl IntoElement {
        let tokens = Self::kpi_card(
            "Tokens",
            compact_count(usage.total_tokens),
            format!("today: {}", compact_count(usage.today_tokens)),
            cx,
        );
        let requests = Self::kpi_card(
            "Requests",
            compact_count(usage.total_requests),
            format!(
                "today {} · rpm {}",
                compact_count(usage.today_requests),
                compact_rate(usage.rpm),
            ),
            cx,
        );
        let cost = Self::kpi_card(
            "Cost",
            format_usd(usage.total_cost),
            format!("today {}", format_usd(usage.today_cost)),
            cx,
        );
        let latency = Self::kpi_card(
            "Latency",
            format!("{} ms", compact_rate(usage.avg_latency_ms)),
            format!("tpm {}", compact_rate(usage.tpm)),
            cx,
        );

        h_flex()
            .w_full()
            .flex_wrap()
            .gap_2()
            .child(tokens)
            .child(requests)
            .child(cost)
            .child(latency)
    }

    /// A KPI card: title (muted), big value, muted sub-line. Flexes to share the
    /// KPI row width evenly; `min_w` keeps a sane minimum before wrapping.
    fn kpi_card(
        title: &'static str,
        value: String,
        sub_line: String,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        v_flex()
            .flex_1()
            .min_w(px(120.))
            .p_2()
            .gap_0p5()
            .border_1()
            .border_color(cx.theme().colors().border)
            .rounded_md()
            .child(Label::new(title).size(LabelSize::Small).color(Color::Muted))
            .child(Label::new(value).size(LabelSize::Large))
            .child(Label::new(sub_line).size(LabelSize::Small).color(Color::Muted))
    }
}

/// Compact a large count to a short human label (e.g. `1234567` → `1.2M`,
/// `34000` → `34k`). Mirrors v1's `formatCompact` so the KPI cards stay narrow
/// in the dock. Values under 1000 are rendered as-is.
fn compact_count(value: u64) -> String {
    const THRESHOLDS: [(u64, char); 3] = [(1_000_000_000, 'B'), (1_000_000, 'M'), (1_000, 'k')];
    for (divisor, suffix) in THRESHOLDS {
        if value >= divisor {
            let scaled = value as f64 / divisor as f64;
            // One decimal below 10 (1.2M), none above (340M) — matches v1.
            return if scaled < 10.0 {
                format!("{scaled:.1}{suffix}")
            } else {
                format!("{:.0}{suffix}", scaled)
            };
        }
    }
    value.to_string()
}

/// Compact a fractional rate (rpm/tpm/latency) to at most one decimal, dropping
/// a trailing `.0`. Large rates reuse the count compaction (e.g. tpm `12k`).
fn compact_rate(value: f64) -> String {
    if !value.is_finite() {
        return "—".into();
    }
    if value >= 1000.0 {
        return compact_count(value.round() as u64);
    }
    let rounded = (value * 10.0).round() / 10.0;
    if (rounded.fract()).abs() < f64::EPSILON {
        format!("{:.0}", rounded)
    } else {
        format!("{rounded:.1}")
    }
}

/// Format a USD cost. Sub-$10 keeps cents (`$3.42`); larger values round to
/// whole dollars and compact thousands (`$1.2k`) to stay legible in a KPI cell.
fn format_usd(value: f64) -> String {
    if !value.is_finite() {
        return "—".into();
    }
    if value.abs() >= 1000.0 {
        format!("${}", compact_count(value.round() as u64))
    } else if value.abs() >= 10.0 {
        format!("${:.0}", value)
    } else {
        format!("${value:.2}")
    }
}

impl Focusable for VibedevUsageDashboard {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<PanelEvent> for VibedevUsageDashboard {}

impl Render for VibedevUsageDashboard {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut root = v_flex()
            .size_full()
            .p_4()
            .gap_2()
            .child(Label::new("VibeDev Usage").size(LabelSize::Large));

        root = match self.status.clone() {
            UsageStatus::Connecting => {
                root.child(Label::new("Connecting to backend…").color(Color::Muted))
            }
            UsageStatus::BackendUnavailable => {
                root.child(Label::new("Backend not ready. Retrying…").color(Color::Muted))
            }
            UsageStatus::Loaded(snapshot) => match snapshot.usage {
                Some(usage) => root.child(self.render_kpi_row(&usage, cx)),
                None => root.child(Label::new("No usage data yet.").color(Color::Muted)),
            },
        };

        // Footer: a manual "Refresh" that pulls a fresh snapshot immediately
        // instead of waiting out the poll interval.
        root.child(
            Button::new("vibedev-usage-refresh", "Refresh")
                .start_icon(Icon::new(IconName::ArrowCircle).size(IconSize::Small))
                .label_size(LabelSize::Small)
                .on_click(cx.listener(|this, _event, _window, cx| {
                    this.refresh_now(cx);
                })),
        )
    }
}

impl Panel for VibedevUsageDashboard {
    fn persistent_name() -> &'static str {
        "Vibedev Usage Dashboard"
    }

    fn panel_key() -> &'static str {
        VIBEDEV_USAGE_DASHBOARD_KEY
    }

    fn position(&self, _window: &Window, _cx: &App) -> DockPosition {
        DockPosition::Right
    }

    fn position_is_valid(&self, position: DockPosition) -> bool {
        matches!(position, DockPosition::Left | DockPosition::Right)
    }

    fn set_position(
        &mut self,
        _position: DockPosition,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        // Dock position persistence is deferred; the panel defaults to the right
        // dock and can be dragged within a session.
    }

    fn default_size(&self, _window: &Window, _cx: &App) -> Pixels {
        px(360.)
    }

    fn icon(&self, _window: &Window, _cx: &App) -> Option<ui::IconName> {
        Some(ui::IconName::SignalHigh)
    }

    fn icon_tooltip(&self, _window: &Window, _cx: &App) -> Option<&'static str> {
        Some("VibeDev Usage")
    }

    fn toggle_action(&self) -> Box<dyn gpui::Action> {
        Box::new(zed_actions::vibedev::ToggleUsageDashboard)
    }

    fn activation_priority(&self) -> u32 {
        11
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fs::FakeFs;
    use gpui::TestAppContext;
    use project::Project;
    use settings::SettingsStore;

    fn init_test(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let settings_store = SettingsStore::test(cx);
            cx.set_global(settings_store);
            theme_settings::init(theme::LoadThemes::JustBase, cx);
        });
    }

    // Mirrors the account panel's registration guard: assert the dashboard is
    // actually registered into the workspace, guarding against "compiles but
    // never wired into the dock". `VibedevUsageDashboard::load` is a thin
    // `update_in` wrapper around `new`, so constructing via `new` + `add_panel`
    // exercises the same registration path that `initialize_panels` drives.
    #[gpui::test]
    async fn usage_dashboard_loads_and_registers(cx: &mut TestAppContext) {
        init_test(cx);

        let fs = FakeFs::new(cx.executor());
        let project = Project::test(fs, [], cx).await;
        let (workspace, cx) =
            cx.add_window_view(|window, cx| Workspace::test_new(project.clone(), window, cx));

        workspace.update_in(cx, |workspace, window, cx| {
            let panel = VibedevUsageDashboard::new(workspace, window, cx);
            workspace.add_panel(panel, window, cx);
        });

        workspace.read_with(cx, |workspace, cx| {
            assert!(
                workspace.panel::<VibedevUsageDashboard>(cx).is_some(),
                "VibedevUsageDashboard should be registered into the workspace"
            );
        });
    }
}
