use std::sync::Arc;

use gpui::{Task, div};
use http_client::HttpClient;
use ui::{Label, prelude::*};
use vibedev_account::ACCOUNT_POLL_INTERVAL;
use workspace::{HideStatusItem, StatusItemView, item::ItemHandle};

/// Status-bar entry showing the account balance. The number comes straight from
/// the sidecar's sub2api account snapshot (the authoritative balance) — never a
/// locally computed ACP cost estimate (Plan 5 audit #12). Hidden until a
/// signed-in balance is available, mirroring how `EditPredictionButton` renders
/// an empty `div` when it has nothing to show.
pub struct VibedevCostStatusItem {
    balance_text: Option<String>,
    _poll: Task<()>,
}

impl VibedevCostStatusItem {
    pub fn new(http: Arc<dyn HttpClient>, cx: &mut Context<Self>) -> Self {
        let poll = cx.spawn(async move |this, cx| {
            loop {
                let snapshot = vibedev_account::fetch_account(http.clone()).await;
                let alive = this
                    .update(cx, |this, cx| {
                        this.balance_text = match snapshot {
                            Ok(snapshot) if snapshot.signed_in => {
                                snapshot.balance.map(|balance| format!("Balance {balance:.2}"))
                            }
                            _ => None,
                        };
                        cx.notify();
                    })
                    .is_ok();
                if !alive {
                    break;
                }
                cx.background_executor().timer(ACCOUNT_POLL_INTERVAL).await;
            }
        });
        Self {
            balance_text: None,
            _poll: poll,
        }
    }
}

impl Render for VibedevCostStatusItem {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        match self.balance_text.clone() {
            Some(text) => Label::new(text).size(LabelSize::Small).into_any_element(),
            None => div().into_any_element(),
        }
    }
}

impl StatusItemView for VibedevCostStatusItem {
    fn set_active_pane_item(
        &mut self,
        _active_pane_item: Option<&dyn ItemHandle>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        // Balance is independent of the active editor pane.
    }

    fn hide_setting(&self, _cx: &App) -> Option<HideStatusItem> {
        None
    }
}
