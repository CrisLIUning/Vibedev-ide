use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use gpui::{
    AsyncWindowContext, Entity, EventEmitter, FocusHandle, Focusable, Task, WeakEntity,
};
use http_client::HttpClient;
use ui::{Label, prelude::*};
use vibedev_account::{
    ACCOUNT_POLL_INTERVAL, AccountSnapshot, AccountSubscription, AccountUsage,
};
use workspace::{
    Workspace,
    dock::{DockPosition, Panel, PanelEvent},
};

const VIBEDEV_ACCOUNT_PANEL_KEY: &str = "VibedevAccountPanel";

// ─── Usage + subscription detail view ───────────────────────────────────────
//
// Modeled on the v1 product's Account tab (the Provider Manager webview in the
// VSCodium fork: `vibedevProviderManager.ts`, `renderAccount*` functions). v1
// rendered six sections from six api-key-authed sub2api endpoints. The v2
// sidecar folds the equivalent data into a single `GET /vibedev/account`
// snapshot, so this panel renders directly from `AccountSnapshot` rather than
// fanning out to per-section endpoints:
//
//   (a) Hero/identity      — email + role, balance, plan pill
//                            (AccountSnapshot.email/profile.role/balance/plan)
//   (b) KPI row (4 cards)   — tokens, requests, cost, latency, each with a
//                            today/rate sub-line (AccountSnapshot.usage)
//   (c) Subscriptions       — per-sub status + daily/weekly/monthly used/limit
//                            USD + expiry (AccountSnapshot.subscriptions)
//
// Every richer field is OPTIONAL on the snapshot: a sidecar that hasn't been
// upgraded yet omits `profile`/`usage`/`subscriptions`, and each section is
// gated behind `if let Some(..)` so the balance/plan/email render never breaks.
//
// FUTURE (intentionally skipped for now, both present in v1): the usage *trend*
// chart (stacked-by-provider daily token bars, v1
// /v1/account/usage/dashboard/trend) and the *by-model* table (model / calls /
// tokens / cost / share bar, v1 /v1/account/usage/dashboard/models). Both need
// time-series / per-model arrays the snapshot does not yet carry, and a gpui
// charting primitive. Add when the sidecar exposes those arrays.

/// Where the data shown in the panel currently stands. Distinguishes "still
/// connecting to the sidecar" from "sidecar said you're signed out" so the
/// render path can show the right message instead of a bare error.
#[derive(Clone)]
enum AccountStatus {
    Connecting,
    BackendUnavailable,
    Loaded(AccountSnapshot),
}

pub struct VibedevAccountPanel {
    focus_handle: FocusHandle,
    http: Arc<dyn HttpClient>,
    status: AccountStatus,
    /// Periodic snapshot poll; dropped (cancelled) when the panel is dropped.
    _poll: Task<()>,
}

impl VibedevAccountPanel {
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
                status: AccountStatus::Connecting,
                _poll: poll,
            }
        })
    }

    /// Polls `fetch_account` on an interval and pushes results into `status`.
    /// The sidecar is the source of truth for balance/plan; we never compute a
    /// cost estimate here.
    fn spawn_poll(http: Arc<dyn HttpClient>, cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |this, cx| {
            // The sidecar handshake is not always up the instant the panel
            // mounts: a cold launch is still spawning bun, and a stale endpoint
            // left by a hard-killed previous IDE is being purged + replaced
            // (see launcher::supervise / clear_stale_handshake). A SINGLE failed
            // fetch must NOT flip the panel to "Backend not ready" — that flashed
            // the alarming message on essentially every launch even though the
            // backend was about to come up. So stay in `Connecting` (and, once
            // signed in, keep showing the last snapshot) until several failures
            // in a row, and retry quickly while not yet connected so the
            // cold-start window resolves in a couple of seconds instead of the
            // full 20s poll interval.
            const FAILURES_BEFORE_UNAVAILABLE: u32 = 4;
            const RETRY_WHILE_CONNECTING: Duration = Duration::from_millis(1500);
            let mut consecutive_failures: u32 = 0;
            loop {
                let snapshot = vibedev_account::fetch_account(http.clone()).await;
                let connected = snapshot.is_ok();
                if connected {
                    consecutive_failures = 0;
                } else {
                    consecutive_failures = consecutive_failures.saturating_add(1);
                }
                let failures = consecutive_failures;
                let updated = this.update(cx, |this, cx| {
                    match snapshot {
                        Ok(snapshot) => this.status = AccountStatus::Loaded(snapshot),
                        Err(_) => {
                            // Only surface "Backend not ready" after repeated
                            // failures; until then keep the current status
                            // (Connecting on cold start, or the last Loaded
                            // snapshot on a transient blip) so a few misses don't
                            // flash the alarming message.
                            if failures >= FAILURES_BEFORE_UNAVAILABLE {
                                this.status = AccountStatus::BackendUnavailable;
                            }
                        }
                    }
                    cx.notify();
                });
                if updated.is_err() {
                    break;
                }
                // Retry fast until we cross the failure threshold; once connected
                // (or already showing unavailable, where frequent retries add no
                // value) fall back to the normal poll cadence.
                let delay = if connected || failures >= FAILURES_BEFORE_UNAVAILABLE {
                    ACCOUNT_POLL_INTERVAL
                } else {
                    RETRY_WHILE_CONNECTING
                };
                cx.background_executor().timer(delay).await;
            }
        })
    }

    /// Trigger one immediate refresh (after a sign-in click, the periodic poll
    /// would otherwise leave the panel stale for up to a full interval).
    fn refresh_now(&self, cx: &mut Context<Self>) {
        let http = self.http.clone();
        cx.spawn(async move |this, cx| {
            let snapshot = vibedev_account::fetch_account(http).await;
            this.update(cx, |this, cx| {
                if let Ok(snapshot) = snapshot {
                    this.status = AccountStatus::Loaded(snapshot);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Kick off the sub2api login flow: ask the sidecar for the login URL, open
    /// it in the browser, then poll until the account reports `signed_in`.
    fn sign_in(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let http = self.http.clone();
        cx.spawn_in(window, async move |this, cx| {
            let login_url = match vibedev_account::start_login(http.clone()).await {
                Ok(url) => url,
                Err(error) => {
                    log::error!("vibedev login start failed: {error:#}");
                    return;
                }
            };
            cx.update(|_, cx| cx.open_url(&login_url)).ok();

            // Poll until signed in (or the panel goes away). The browser round
            // trip + sidecar token exchange takes a few seconds.
            for _ in 0..150 {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(2))
                    .await;
                let snapshot = vibedev_account::fetch_account(http.clone()).await;
                let signed_in = matches!(&snapshot, Ok(snapshot) if snapshot.signed_in);
                let alive = this
                    .update(cx, |this, cx| {
                        if let Ok(snapshot) = snapshot {
                            this.status = AccountStatus::Loaded(snapshot);
                            cx.notify();
                        }
                    })
                    .is_ok();
                if signed_in || !alive {
                    // VIBEDEV: now that the account has models, (re)register +
                    // (re)select the inline-assistant chat model. The sidecar
                    // supervisor can't notice this 0 → N model change mid-session
                    // (it's parked on the child's exit), so the panel drives it.
                    if signed_in {
                        vibedev_account::reconfigure_inline_assistant(http.clone(), cx).await;
                    }
                    break;
                }
            }
        })
        .detach();
    }

    /// Sign out: tell the sidecar to drop the api_key, then immediately re-poll
    /// so the panel flips to the signed-out hint without waiting a full poll
    /// interval. A sidecar/transport failure is logged and leaves the panel
    /// state intact (the user can retry).
    fn sign_out(&self, cx: &mut Context<Self>) {
        let http = self.http.clone();
        cx.spawn(async move |this, cx| {
            if let Err(error) = vibedev_account::sign_out(http.clone()).await {
                log::error!("vibedev sign-out failed: {error:#}");
                return;
            }
            let snapshot = vibedev_account::fetch_account(http).await;
            this.update(cx, |this, cx| {
                if let Ok(snapshot) = snapshot {
                    this.status = AccountStatus::Loaded(snapshot);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Refresh the gateway model catalog. Tells the sidecar to re-fetch
    /// `/v1/models`, then triggers an account re-poll so the panel reflects
    /// the new list without a 20 s lag. Failure is logged at warn; the picker
    /// keeps the previous list so a transient gateway blip never wipes it.
    fn refresh_models(&self, cx: &mut Context<Self>) {
        let http = self.http.clone();
        cx.spawn(async move |this, cx| {
            match vibedev_account::refresh_models(http.clone()).await {
                Ok(model_count) => {
                    log::info!("vibedev model catalog refreshed: {model_count} models");
                }
                Err(error) => {
                    log::warn!("vibedev model catalog refresh failed: {error:#}");
                    return;
                }
            }
            // VIBEDEV: the catalog changed, so re-sync the inline-assistant chat
            // provider's model list + selection to match.
            vibedev_account::reconfigure_inline_assistant(http.clone(), cx).await;
            let snapshot = vibedev_account::fetch_account(http).await;
            this.update(cx, |this, cx| {
                if let Ok(snapshot) = snapshot {
                    this.status = AccountStatus::Loaded(snapshot);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    pub async fn load(
        workspace: WeakEntity<Workspace>,
        mut cx: AsyncWindowContext,
    ) -> Result<Entity<Self>> {
        workspace.update_in(&mut cx, |workspace, window, cx| {
            VibedevAccountPanel::new(workspace, window, cx)
        })
    }

    fn render_signed_in(&self, snapshot: &AccountSnapshot, cx: &Context<Self>) -> impl IntoElement {
        v_flex()
            .gap_3()
            .child(self.render_hero(snapshot, cx))
            .when_some(snapshot.usage.clone(), |this, usage| {
                this.child(self.render_kpi_row(&usage, cx))
            })
            .when_some(snapshot.subscriptions.clone(), |this, subscriptions| {
                this.children(self.render_subscriptions(&subscriptions, cx))
            })
    }

    /// Section (a) — Hero (mirrors v1 `renderAccountHeader`): identity (email +
    /// role) on the left, balance + plan pill on the right. `role` is shown as a
    /// muted sub-line under the email when present; `total_recharged` (also on
    /// `profile`) is intentionally not surfaced here to keep the hero compact.
    /// The avatar (`profile.avatar_url`) needs an image-loading element and is
    /// left for a future pass.
    fn render_hero(&self, snapshot: &AccountSnapshot, cx: &Context<Self>) -> impl IntoElement {
        let email = snapshot
            .email
            .clone()
            .unwrap_or_else(|| "Signed in".into());
        let role = snapshot
            .profile
            .as_ref()
            .and_then(|profile| profile.role.clone());
        let balance = snapshot
            .balance
            .map(|balance| format!("${balance:.2}"));

        let identity = v_flex()
            .gap_0p5()
            .child(Label::new(email).size(LabelSize::Default))
            .when_some(role, |this, role| {
                this.child(Label::new(role).size(LabelSize::Small).color(Color::Muted))
            });

        let trailing = h_flex()
            .items_center()
            .gap_2()
            .when_some(balance, |this, balance| {
                this.child(Label::new(balance).size(LabelSize::Default))
            })
            .when_some(snapshot.plan.clone(), |this, plan| {
                // Plan "pill" — small bordered chip echoing v1's `pm-acct-plan-pill`.
                this.child(
                    h_flex()
                        .px_1p5()
                        .py_0p5()
                        .border_1()
                        .border_color(cx.theme().colors().border)
                        .rounded_md()
                        .child(
                            Label::new(plan.to_uppercase())
                                .size(LabelSize::Small)
                                .color(Color::Accent),
                        ),
                )
            });

        Self::card(
            "Account",
            h_flex()
                .w_full()
                .items_start()
                .justify_between()
                .gap_2()
                .child(identity)
                .child(trailing),
            cx,
        )
    }

    /// Section (b) — KPI row (mirrors v1 `renderAccountKpi`): four label / big
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

    /// Section (c) — Subscriptions (mirrors v1 `renderAccountSubs`): one card
    /// listing each subscription's group/status and its daily/weekly/monthly
    /// "used / limit" USD (only periods that declare a limit) plus expiry.
    /// Returns `None` when the list is empty so the caller skips the card.
    fn render_subscriptions(
        &self,
        subscriptions: &[AccountSubscription],
        cx: &Context<Self>,
    ) -> Option<impl IntoElement> {
        if subscriptions.is_empty() {
            return None;
        }
        let entries = subscriptions
            .iter()
            .map(|subscription| self.render_subscription_entry(subscription))
            .collect::<Vec<_>>();
        Some(Self::card("Subscription", v_flex().gap_2().children(entries), cx))
    }

    fn render_subscription_entry(&self, subscription: &AccountSubscription) -> impl IntoElement {
        let periods = [
            ("Daily", subscription.daily_used_usd, subscription.daily_limit_usd),
            ("Weekly", subscription.weekly_used_usd, subscription.weekly_limit_usd),
            (
                "Monthly",
                subscription.monthly_used_usd,
                subscription.monthly_limit_usd,
            ),
        ];
        // Only periods that declare a limit are shown (v1 hid uncapped periods).
        let period_rows = periods
            .into_iter()
            .filter_map(|(label, used, limit)| {
                let limit = limit?;
                let used = used.unwrap_or(0.0);
                Some(Self::detail_row(
                    label,
                    format!("{} / {}", format_usd(used), format_usd(limit)),
                ))
            })
            .collect::<Vec<_>>();

        v_flex()
            .gap_1()
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        Label::new(subscription.group_name.clone()).size(LabelSize::Small),
                    )
                    .child(
                        Label::new(subscription.status.clone())
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    ),
            )
            .children(period_rows)
            .when_some(subscription.expires_at.clone(), |this, expires_at| {
                this.child(
                    Label::new(format!("expires {expires_at}"))
                        .size(LabelSize::Small)
                        .color(Color::Muted),
                )
            })
    }

    /// A KPI card: title (muted), big value, muted sub-line. Flexes to share the
    /// KPI row width evenly; `flex_basis` keeps a sane minimum before wrapping.
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

    /// A titled, bordered container — the gpui analogue of v1's `acct-*` card
    /// styling, matching the borders/rounding used by the agent panels.
    fn card(
        title: &'static str,
        body: impl IntoElement,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        v_flex()
            .w_full()
            .p_2()
            .gap_1p5()
            .border_1()
            .border_color(cx.theme().colors().border)
            .rounded_md()
            .child(
                Label::new(title)
                    .size(LabelSize::Small)
                    .color(Color::Muted),
            )
            .child(body)
    }

    /// A label/value row (label muted on the left, value on the right) — the
    /// shape v1 used inside its `acct-sub-cell` / KPI cells.
    fn detail_row(label: &'static str, value: String) -> impl IntoElement {
        h_flex()
            .w_full()
            .justify_between()
            .gap_2()
            .child(Label::new(label).size(LabelSize::Small).color(Color::Muted))
            .child(Label::new(value).size(LabelSize::Small))
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

impl Focusable for VibedevAccountPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<PanelEvent> for VibedevAccountPanel {}

impl Render for VibedevAccountPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut root = v_flex()
            .size_full()
            .p_4()
            .gap_2()
            .child(Label::new("VibeDev Account").size(LabelSize::Large));

        root = match self.status.clone() {
            AccountStatus::Connecting => {
                root.child(Label::new("Connecting to backend…").color(Color::Muted))
            }
            AccountStatus::BackendUnavailable => root.child(
                Label::new("Backend not ready. Retrying…").color(Color::Muted),
            ),
            AccountStatus::Loaded(snapshot) if snapshot.signed_in => root
                .child(self.render_signed_in(&snapshot, cx))
                .child(
                    Button::new("vibedev-sign-out", "Sign Out")
                        .label_size(LabelSize::Small)
                        .on_click(cx.listener(|this, _event, _window, cx| {
                            this.sign_out(cx);
                        })),
                ),
            AccountStatus::Loaded(snapshot) => {
                let hint = snapshot
                    .reason
                    .unwrap_or_else(|| "Sign in to view your balance.".into());
                root.child(Label::new(hint).color(Color::Muted)).child(
                    Button::new("vibedev-sign-in", "Sign In")
                        .end_icon(Icon::new(IconName::ArrowUpRight).size(IconSize::Small))
                        .full_width()
                        .on_click(cx.listener(|this, _event, window, cx| {
                            this.sign_in(window, cx);
                        })),
                )
            }
        };

        // Footer actions: "Refresh" always pulls a fresh account snapshot;
        // "Refresh Models" additionally invalidates the model catalog cache
        // so a partial model list can be repaired without restarting the app.
        let signed_in = matches!(
            &self.status,
            AccountStatus::Loaded(snapshot) if snapshot.signed_in
        );
        root.child(
            h_flex()
                .gap_2()
                .child(
                    Button::new("vibedev-account-refresh", "Refresh")
                        .start_icon(Icon::new(IconName::ArrowCircle).size(IconSize::Small))
                        .label_size(LabelSize::Small)
                        .on_click(cx.listener(|this, _event, _window, cx| {
                            this.refresh_now(cx);
                        })),
                )
                .when(signed_in, |this| {
                    this.child(
                        Button::new("vibedev-refresh-models", "🔄 Refresh Models")
                            .label_size(LabelSize::Small)
                            .on_click(cx.listener(|this, _event, _window, cx| {
                                this.refresh_models(cx);
                            })),
                    )
                }),
        )
    }
}

impl Panel for VibedevAccountPanel {
    fn persistent_name() -> &'static str {
        "Vibedev Account Panel"
    }

    fn panel_key() -> &'static str {
        VIBEDEV_ACCOUNT_PANEL_KEY
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
        px(320.)
    }

    fn icon(&self, _window: &Window, _cx: &App) -> Option<ui::IconName> {
        Some(ui::IconName::Person)
    }

    fn icon_tooltip(&self, _window: &Window, _cx: &App) -> Option<&'static str> {
        Some("VibeDev Account")
    }

    fn toggle_action(&self) -> Box<dyn gpui::Action> {
        Box::new(zed_actions::vibedev::ToggleAccountFocus)
    }

    fn activation_priority(&self) -> u32 {
        9
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

    // Mirrors VibeDev VS's patch72 lesson (vibedevCloudRegistration.test.ts):
    // assert the panel is actually registered into the workspace, guarding
    // against "compiles but never wired into the dock". `VibedevAccountPanel::load`
    // is a thin `update_in` wrapper around `new`, so constructing via `new` +
    // `add_panel` exercises the same registration path that `initialize_panels`
    // drives (matching the existing project_panel test scaffolding).
    #[gpui::test]
    async fn account_panel_loads_and_registers(cx: &mut TestAppContext) {
        init_test(cx);

        let fs = FakeFs::new(cx.executor());
        let project = Project::test(fs, [], cx).await;
        let (workspace, cx) =
            cx.add_window_view(|window, cx| Workspace::test_new(project.clone(), window, cx));

        workspace.update_in(cx, |workspace, window, cx| {
            let panel = VibedevAccountPanel::new(workspace, window, cx);
            workspace.add_panel(panel, window, cx);
        });

        workspace.read_with(cx, |workspace, cx| {
            assert!(
                workspace.panel::<VibedevAccountPanel>(cx).is_some(),
                "VibedevAccountPanel should be registered into the workspace"
            );
        });
    }
}
