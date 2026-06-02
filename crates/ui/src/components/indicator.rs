use super::AnyIcon;
use crate::prelude::*;

#[derive(Default)]
enum IndicatorKind {
    #[default]
    Dot,
    Bar,
    Icon(AnyIcon),
}

#[derive(IntoElement, RegisterComponent)]
pub struct Indicator {
    kind: IndicatorKind,
    border_color: Option<Color>,
    pub color: Color,
}

impl Indicator {
    pub fn dot() -> Self {
        Self {
            kind: IndicatorKind::Dot,
            border_color: None,
            color: Color::Default,
        }
    }

    pub fn bar() -> Self {
        Self {
            kind: IndicatorKind::Bar,
            border_color: None,

            color: Color::Default,
        }
    }

    pub fn icon(icon: impl Into<AnyIcon>) -> Self {
        Self {
            kind: IndicatorKind::Icon(icon.into()),
            border_color: None,

            color: Color::Default,
        }
    }

    pub fn color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    pub fn border_color(mut self, color: Color) -> Self {
        self.border_color = Some(color);
        self
    }
}

impl RenderOnce for Indicator {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let container = div().flex_none();
        let container = if let Some(border_color) = self.border_color {
            if matches!(self.kind, IndicatorKind::Dot | IndicatorKind::Bar) {
                container.border_1().border_color(border_color.color(cx))
            } else {
                container
            }
        } else {
            container
        };

        match self.kind {
            IndicatorKind::Icon(icon) => container
                .child(icon.map(|icon| icon.custom_size(rems_from_px(8.)).color(self.color))),
            IndicatorKind::Dot => container
                .w_1p5()
                .h_1p5()
                .rounded_full()
                .bg(self.color.color(cx)),
            IndicatorKind::Bar => container
                .w_full()
                .h_1p5()
                .rounded_t_sm()
                .bg(self.color.color(cx)),
        }
    }
}

impl Component for Indicator {
    fn scope() -> ComponentScope {
        ComponentScope::Status
    }

    fn description() -> Option<&'static str> {
        Some(
            "用于表示状态、通知或引起对特定元素注意的视觉指示器。",
        )
    }

    fn preview(_window: &mut Window, _cx: &mut App) -> Option<AnyElement> {
        Some(
            v_flex()
                .gap_6()
                .children(vec![
                    example_group_with_title(
                        "圆点指示器",
                        vec![
                            single_example("默认", Indicator::dot().into_any_element()),
                            single_example(
                                "成功",
                                Indicator::dot().color(Color::Success).into_any_element(),
                            ),
                            single_example(
                                "警告",
                                Indicator::dot().color(Color::Warning).into_any_element(),
                            ),
                            single_example(
                                "错误",
                                Indicator::dot().color(Color::Error).into_any_element(),
                            ),
                            single_example(
                                "带边框",
                                Indicator::dot()
                                    .color(Color::Accent)
                                    .border_color(Color::Default)
                                    .into_any_element(),
                            ),
                        ],
                    ),
                    example_group_with_title(
                        "条形指示器",
                        vec![
                            single_example("默认", Indicator::bar().into_any_element()),
                            single_example(
                                "成功",
                                Indicator::bar().color(Color::Success).into_any_element(),
                            ),
                            single_example(
                                "警告",
                                Indicator::bar().color(Color::Warning).into_any_element(),
                            ),
                            single_example(
                                "错误",
                                Indicator::bar().color(Color::Error).into_any_element(),
                            ),
                        ],
                    ),
                    example_group_with_title(
                        "图标指示器",
                        vec![
                            single_example(
                                "默认",
                                Indicator::icon(Icon::new(IconName::Circle)).into_any_element(),
                            ),
                            single_example(
                                "成功",
                                Indicator::icon(Icon::new(IconName::Check))
                                    .color(Color::Success)
                                    .into_any_element(),
                            ),
                            single_example(
                                "警告",
                                Indicator::icon(Icon::new(IconName::Warning))
                                    .color(Color::Warning)
                                    .into_any_element(),
                            ),
                            single_example(
                                "错误",
                                Indicator::icon(Icon::new(IconName::Close))
                                    .color(Color::Error)
                                    .into_any_element(),
                            ),
                        ],
                    ),
                ])
                .into_any_element(),
        )
    }
}
