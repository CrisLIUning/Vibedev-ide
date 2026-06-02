use crate::{Tooltip, prelude::*};
use gpui::{ClickEvent, IntoElement, ParentElement, SharedString};

#[derive(IntoElement, RegisterComponent)]
pub struct ConfiguredApiCard {
    label: SharedString,
    button_label: Option<SharedString>,
    button_tab_index: Option<isize>,
    tooltip_label: Option<SharedString>,
    disabled: bool,
    on_click: Option<Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>>,
}

impl ConfiguredApiCard {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            button_label: None,
            button_tab_index: None,
            tooltip_label: None,
            disabled: false,
            on_click: None,
        }
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }

    pub fn button_label(mut self, button_label: impl Into<SharedString>) -> Self {
        self.button_label = Some(button_label.into());
        self
    }

    pub fn tooltip_label(mut self, tooltip_label: impl Into<SharedString>) -> Self {
        self.tooltip_label = Some(tooltip_label.into());
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn button_tab_index(mut self, tab_index: isize) -> Self {
        self.button_tab_index = Some(tab_index);
        self
    }
}

impl Component for ConfiguredApiCard {
    fn scope() -> ComponentScope {
        ComponentScope::Agent
    }

    fn preview(_window: &mut Window, cx: &mut App) -> Option<AnyElement> {
        let container = || {
            v_flex()
                .w_72()
                .p_2()
                .gap_2()
                .border_1()
                .border_color(cx.theme().colors().border_variant)
                .bg(cx.theme().colors().panel_background)
        };

        let examples = vec![
            single_example(
                "默认",
                container()
                    .child(ConfiguredApiCard::new("API 密钥已配置"))
                    .into_any_element(),
            ),
            single_example(
                "自定义按钮标签",
                container()
                    .child(
                        ConfiguredApiCard::new("OpenAI API 密钥已配置")
                            .button_label("移除密钥"),
                    )
                    .into_any_element(),
            ),
            single_example(
                "带工具提示",
                container()
                    .child(
                        ConfiguredApiCard::new("Anthropic API 密钥已配置")
                            .tooltip_label("点击重置 API 密钥"),
                    )
                    .into_any_element(),
            ),
            single_example(
                "已禁用",
                container()
                    .child(ConfiguredApiCard::new("API 密钥已配置").disabled(true))
                    .into_any_element(),
            ),
        ];

        Some(example_group(examples).into_any_element())
    }
}

impl RenderOnce for ConfiguredApiCard {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let button_label = self.button_label.unwrap_or("重置密钥".into());
        let button_id = SharedString::new(format!("id-{}", button_label));

        h_flex()
            .min_w_0()
            .mt_0p5()
            .p_1()
            .justify_between()
            .rounded_md()
            .flex_wrap()
            .border_1()
            .border_color(cx.theme().colors().border)
            .bg(cx.theme().colors().background)
            .child(
                h_flex()
                    .min_w_0()
                    .gap_1()
                    .child(Icon::new(IconName::Check).color(Color::Success))
                    .child(Label::new(self.label)),
            )
            .child(
                Button::new(button_id, button_label)
                    .when_some(self.button_tab_index, |elem, tab_index| {
                        elem.tab_index(tab_index)
                    })
                    .label_size(LabelSize::Small)
                    .start_icon(
                        Icon::new(IconName::Undo)
                            .size(IconSize::Small)
                            .color(Color::Muted),
                    )
                    .disabled(self.disabled)
                    .when_some(self.tooltip_label, |this, label| {
                        this.tooltip(Tooltip::text(label))
                    })
                    .when_some(
                        self.on_click.filter(|_| !self.disabled),
                        |this, on_click| this.on_click(on_click),
                    ),
            )
    }
}
