use std::sync::Arc;

use ai_onboarding::{AgentPanelOnboardingCard, PlanDefinitions};
use gpui::{AnyElement, App, IntoElement, RenderOnce, Window};
use ui::{Divider, Tooltip, prelude::*};

#[derive(IntoElement, RegisterComponent)]
pub struct EndTrialUpsell {
    dismiss_upsell: Arc<dyn Fn(&mut Window, &mut App)>,
}

impl EndTrialUpsell {
    // VIBEDEV: rendering path (`should_render_trial_end_upsell` in agent_panel) was neutered;
    // kept for upstream-rebase ease.
    #[allow(dead_code)]
    pub fn new(dismiss_upsell: Arc<dyn Fn(&mut Window, &mut App)>) -> Self {
        Self { dismiss_upsell }
    }
}

impl RenderOnce for EndTrialUpsell {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let pro_section = v_flex()
            .gap_1()
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Label::new("专业版")
                            .size(LabelSize::Small)
                            .color(Color::Accent)
                            .buffer_font(cx),
                    )
                    .child(Divider::horizontal()),
            )
            .child(PlanDefinitions.pro_plan())
            .child(
                Button::new("cta-button", "升级到 VibeDev Pro")
                    .full_width()
                    .style(ButtonStyle::Tinted(ui::TintColor::Accent))
                    .on_click(move |_, _window, _cx| {
                        telemetry::event!("Upgrade To Pro Clicked", state = "end-of-trial");
                        { /* VIBEDEV: was zed.dev/account/upgrade; render_trial_end_upsell never mounts this component anyway */ }
                    }),
            );

        let free_section = v_flex()
            .mt_1p5()
            .gap_1()
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Label::new("免费")
                            .size(LabelSize::Small)
                            .color(Color::Muted)
                            .buffer_font(cx),
                    )
                    .child(
                        Label::new("(当前方案)")
                            .size(LabelSize::Small)
                            .color(Color::Custom(cx.theme().colors().text_muted.opacity(0.6)))
                            .buffer_font(cx),
                    )
                    .child(Divider::horizontal()),
            )
            .child(PlanDefinitions.free_plan());

        AgentPanelOnboardingCard::new()
            .child(Headline::new("您的 VibeDev Pro 试用已过期"))
            .child(
                Label::new("您已被自动重置为 Free 方案。")
                    .color(Color::Muted)
                    .mb_2(),
            )
            .child(pro_section)
            .child(free_section)
            .child(
                h_flex().absolute().top_4().right_4().child(
                    IconButton::new("dismiss_onboarding", IconName::Close)
                        .icon_size(IconSize::Small)
                        .tooltip(Tooltip::text("关闭"))
                        .on_click({
                            let callback = self.dismiss_upsell.clone();
                            move |_, window, cx| {
                                telemetry::event!("Banner Dismissed", source = "AI Onboarding");
                                callback(window, cx)
                            }
                        }),
                ),
            )
    }
}

impl Component for EndTrialUpsell {
    fn scope() -> ComponentScope {
        ComponentScope::Onboarding
    }

    fn name() -> &'static str {
        "End of Trial Upsell Banner"
    }

    fn sort_name() -> &'static str {
        "End of Trial Upsell Banner"
    }

    fn preview(_window: &mut Window, _cx: &mut App) -> Option<AnyElement> {
        Some(
            v_flex()
                .child(EndTrialUpsell {
                    dismiss_upsell: Arc::new(|_, _| {}),
                })
                .into_any_element(),
        )
    }
}
