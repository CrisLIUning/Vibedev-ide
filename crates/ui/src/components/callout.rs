use gpui::AnyElement;

use crate::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorderPosition {
    Top,
    Bottom,
}

/// A callout component for displaying important information that requires user attention.
///
/// # Usage Example
///
/// ```
/// use ui::prelude::*;
/// use ui::{Button, Callout, IconName, Label, Severity};
///
/// let callout = Callout::new()
///     .severity(Severity::Warning)
///     .icon(IconName::Warning)
///     .title("请注意您的订阅!")
///     .description("您的订阅即将到期。请立即续订!")
///     .actions_slot(Button::new("renew", "立即续订"));
/// ```
///
#[derive(IntoElement, RegisterComponent)]
pub struct Callout {
    severity: Severity,
    icon: Option<IconName>,
    title: Option<SharedString>,
    description: Option<SharedString>,
    description_slot: Option<AnyElement>,
    actions_slot: Option<AnyElement>,
    dismiss_action: Option<AnyElement>,
    line_height: Option<Pixels>,
    border_position: BorderPosition,
}

impl Callout {
    /// Creates a new `Callout` component with default styling.
    pub fn new() -> Self {
        Self {
            severity: Severity::Info,
            icon: None,
            title: None,
            description: None,
            description_slot: None,
            actions_slot: None,
            dismiss_action: None,
            line_height: None,
            border_position: BorderPosition::Top,
        }
    }

    /// Sets the severity of the callout.
    pub fn severity(mut self, severity: Severity) -> Self {
        self.severity = severity;
        self
    }

    /// Sets the icon to display in the callout.
    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Sets the title of the callout.
    pub fn title(mut self, title: impl Into<SharedString>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Sets the description of the callout.
    /// The description can be single or multi-line text.
    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Allows for any element—like markdown elements—to fill the description slot of the callout.
    /// This method wins over `description` if both happen to be set.
    pub fn description_slot(mut self, description: impl IntoElement) -> Self {
        self.description_slot = Some(description.into_any_element());
        self
    }

    /// Sets the primary call-to-action button.
    pub fn actions_slot(mut self, action: impl IntoElement) -> Self {
        self.actions_slot = Some(action.into_any_element());
        self
    }

    /// Sets an optional dismiss button, which is usually an icon button with a close icon.
    /// This button is always rendered as the last one to the far right.
    pub fn dismiss_action(mut self, action: impl IntoElement) -> Self {
        self.dismiss_action = Some(action.into_any_element());
        self
    }

    /// Sets a custom line height for the callout content.
    pub fn line_height(mut self, line_height: Pixels) -> Self {
        self.line_height = Some(line_height);
        self
    }

    /// Sets the border position in the callout.
    pub fn border_position(mut self, border_position: BorderPosition) -> Self {
        self.border_position = border_position;
        self
    }
}

impl RenderOnce for Callout {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let line_height = self.line_height.unwrap_or(window.line_height());

        let has_actions = self.actions_slot.is_some() || self.dismiss_action.is_some();

        let (icon, icon_color, bg_color) = match self.severity {
            Severity::Info => (
                IconName::Info,
                Color::Muted,
                cx.theme().status().info_background.opacity(0.1),
            ),
            Severity::Success => (
                IconName::Check,
                Color::Success,
                cx.theme().status().success.opacity(0.1),
            ),
            Severity::Warning => (
                IconName::Warning,
                Color::Warning,
                cx.theme().status().warning_background.opacity(0.2),
            ),
            Severity::Error => (
                IconName::XCircle,
                Color::Error,
                cx.theme().status().error.opacity(0.08),
            ),
        };

        h_flex()
            .min_w_0()
            .w_full()
            .p_2()
            .gap_2()
            .items_start()
            .map(|this| match self.border_position {
                BorderPosition::Top => this.border_t_1(),
                BorderPosition::Bottom => this.border_b_1(),
            })
            .border_color(cx.theme().colors().border)
            .bg(bg_color)
            .overflow_x_hidden()
            .when(self.icon.is_some(), |this| {
                this.child(
                    h_flex()
                        .h(line_height)
                        .justify_center()
                        .child(Icon::new(icon).size(IconSize::Small).color(icon_color)),
                )
            })
            .child(
                v_flex()
                    .min_w_0()
                    .min_h_0()
                    .w_full()
                    .child(
                        h_flex()
                            .min_h(line_height)
                            .w_full()
                            .gap_1()
                            .justify_between()
                            .flex_wrap()
                            .when_some(self.title, |this, title| {
                                this.child(
                                    div()
                                        .min_w_0()
                                        .flex_1()
                                        .child(Label::new(title).size(LabelSize::Small)),
                                )
                            })
                            .when(has_actions, |this| {
                                this.child(
                                    h_flex()
                                        .gap_0p5()
                                        .when_some(self.actions_slot, |this, action| {
                                            this.child(action)
                                        })
                                        .when_some(self.dismiss_action, |this, action| {
                                            this.child(action)
                                        }),
                                )
                            }),
                    )
                    .map(|this| {
                        let base_desc_container = div()
                            .id("callout-description-slot")
                            .w_full()
                            .max_h_32()
                            .flex_1()
                            .overflow_y_scroll()
                            .text_ui_sm(cx);

                        if let Some(description_slot) = self.description_slot {
                            this.child(base_desc_container.child(description_slot))
                        } else if let Some(description) = self.description {
                            this.child(
                                base_desc_container
                                    .text_color(cx.theme().colors().text_muted)
                                    .child(description),
                            )
                        } else {
                            this
                        }
                    }),
            )
    }
}

impl Component for Callout {
    fn scope() -> ComponentScope {
        ComponentScope::DataDisplay
    }

    fn description() -> Option<&'static str> {
        Some(
            "用于在用户需要了解某些信息并可能需要做出决定的情况下显示标注。例如对话线程令牌耗尽,或计划内的提示词用尽需要升级。",
        )
    }

    fn preview(_window: &mut Window, _cx: &mut App) -> Option<AnyElement> {
        let single_action = || Button::new("got-it", "知道了").label_size(LabelSize::Small);
        let multiple_actions = || {
            h_flex()
                .gap_0p5()
                .child(Button::new("update", "备份并更新").label_size(LabelSize::Small))
                .child(Button::new("dismiss", "关闭").label_size(LabelSize::Small))
        };

        let basic_examples = vec![
            single_example(
                "仅包含标题的简单样式",
                Callout::new()
                    .icon(IconName::Info)
                    .title("系统维护定于今晚进行")
                    .actions_slot(single_action())
                    .into_any_element(),
            )
            .width(px(580.)),
            single_example(
                "包含标题和描述",
                Callout::new()
                    .icon(IconName::Warning)
                    .title("您的设置包含已弃用的值")
                    .description(
                        "我们将备份您当前的设置并将其更新为新格式。",
                    )
                    .actions_slot(single_action())
                    .into_any_element(),
            )
            .width(px(580.)),
            single_example(
                "包含多个操作的错误",
                Callout::new()
                    .icon(IconName::Close)
                    .title("对话线程已达到 Token 上限")
                    .description("从摘要启动新对话线程以继续对话。")
                    .actions_slot(multiple_actions())
                    .into_any_element(),
            )
            .width(px(580.)),
            single_example(
                "多行描述",
                Callout::new()
                    .icon(IconName::Sparkle)
                    .title("升级到 Pro")
                    .description("• 无限对话线程\n• 优先支持\n• 高级分析")
                    .actions_slot(multiple_actions())
                    .into_any_element(),
            )
            .width(px(580.)),
            single_example(
                "可滚动的长描述",
                Callout::new()
                    .severity(Severity::Error)
                    .icon(IconName::XCircle)
                    .title("非常长的 API 错误描述")
                    .description_slot(
                        v_flex().gap_1().children(
                            [
                                "您已超出当前配额。",
                                "欲了解更多信息,请访问文档。",
                                "错误详情:",
                                "• 指标超出配额",
                                "• 限制:0",
                                "• 模型: gemini-3.1-pro",
                                "请在 26.33 秒后重试。",
                                "附加详情:",
                                "- 请求 ID:abc123def456",
                                "- 时间戳:2024-01-15T10:30:00Z",
                                "- 区域:us-central1",
                                "- 服务:generativelanguage.googleapis.com",
                                "- 错误代码:RESOURCE_EXHAUSTED",
                                "- 重试时间:26s",
                                "当您超出 API 配额时会发生此错误。",
                            ]
                            .into_iter()
                            .map(|t| Label::new(t).size(LabelSize::Small).color(Color::Muted)),
                        ),
                    )
                    .actions_slot(single_action())
                    .into_any_element(),
            )
            .width(px(580.)),
        ];

        let severity_examples = vec![
            single_example(
                "信息",
                Callout::new()
                    .icon(IconName::Info)
                    .title("系统维护定于今晚进行")
                    .actions_slot(single_action())
                    .into_any_element(),
            ),
            single_example(
                "警告",
                Callout::new()
                    .severity(Severity::Warning)
                    .icon(IconName::Triangle)
                    .title("系统维护定于今晚进行")
                    .actions_slot(single_action())
                    .into_any_element(),
            ),
            single_example(
                "错误",
                Callout::new()
                    .severity(Severity::Error)
                    .icon(IconName::XCircle)
                    .title("系统维护定于今晚进行")
                    .actions_slot(single_action())
                    .into_any_element(),
            ),
            single_example(
                "成功",
                Callout::new()
                    .severity(Severity::Success)
                    .icon(IconName::Check)
                    .title("系统维护定于今晚进行")
                    .actions_slot(single_action())
                    .into_any_element(),
            ),
        ];

        Some(
            v_flex()
                .gap_4()
                .child(example_group(basic_examples).vertical())
                .child(example_group_with_title("严重程度", severity_examples).vertical())
                .into_any_element(),
        )
    }
}
