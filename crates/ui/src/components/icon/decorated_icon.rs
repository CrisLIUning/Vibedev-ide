use gpui::{AnyElement, IntoElement, Point};

use crate::{IconDecoration, IconDecorationKind, prelude::*};

#[derive(IntoElement, RegisterComponent)]
pub struct DecoratedIcon {
    icon: Icon,
    decoration: Option<IconDecoration>,
}

impl DecoratedIcon {
    pub fn new(icon: Icon, decoration: Option<IconDecoration>) -> Self {
        Self { icon, decoration }
    }
}

impl RenderOnce for DecoratedIcon {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .relative()
            .size(self.icon.size)
            .child(self.icon)
            .children(self.decoration)
    }
}

impl Component for DecoratedIcon {
    fn scope() -> ComponentScope {
        ComponentScope::Images
    }

    fn description() -> Option<&'static str> {
        Some(
            "带有可选装饰覆盖层(如 X、三角形或圆点)的图标,该覆盖层可相对于图标定位",
        )
    }

    fn preview(_window: &mut Window, cx: &mut App) -> Option<AnyElement> {
        let decoration_x = IconDecoration::new(
            IconDecorationKind::X,
            cx.theme().colors().surface_background,
            cx,
        )
        .color(cx.theme().status().error)
        .position(Point {
            x: px(-2.),
            y: px(-2.),
        });

        let decoration_triangle = IconDecoration::new(
            IconDecorationKind::Triangle,
            cx.theme().colors().surface_background,
            cx,
        )
        .color(cx.theme().status().error)
        .position(Point {
            x: px(-2.),
            y: px(-2.),
        });

        let decoration_dot = IconDecoration::new(
            IconDecorationKind::Dot,
            cx.theme().colors().surface_background,
            cx,
        )
        .color(cx.theme().status().error)
        .position(Point {
            x: px(-2.),
            y: px(-2.),
        });

        Some(
            v_flex()
                .gap_6()
                .children(vec![example_group_with_title(
                    "装饰",
                    vec![
                        single_example(
                            "无装饰",
                            DecoratedIcon::new(Icon::new(IconName::FileDoc), None)
                                .into_any_element(),
                        ),
                        single_example(
                            "X 装饰",
                            DecoratedIcon::new(Icon::new(IconName::FileDoc), Some(decoration_x))
                                .into_any_element(),
                        ),
                        single_example(
                            "三角形装饰",
                            DecoratedIcon::new(
                                Icon::new(IconName::FileDoc),
                                Some(decoration_triangle),
                            )
                            .into_any_element(),
                        ),
                        single_example(
                            "圆点装饰",
                            DecoratedIcon::new(Icon::new(IconName::FileDoc), Some(decoration_dot))
                                .into_any_element(),
                        ),
                    ],
                )])
                .into_any_element(),
        )
    }
}
