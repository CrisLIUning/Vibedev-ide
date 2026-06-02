#![cfg_attr(target_family = "wasm", no_main)]

use gpui::{
    App, Bounds, BoxShadow, Context, Div, SharedString, Window, WindowBounds, WindowOptions, div,
    hsla, point, prelude::*, px, relative, rgb, size,
};
use gpui_platform::application;

struct Shadow {}

impl Shadow {
    fn base() -> Div {
        div()
            .size_16()
            .bg(rgb(0xffffff))
            .rounded_full()
            .border_1()
            .border_color(hsla(0.0, 0.0, 0.0, 0.1))
    }

    fn square() -> Div {
        div()
            .size_16()
            .bg(rgb(0xffffff))
            .border_1()
            .border_color(hsla(0.0, 0.0, 0.0, 0.1))
    }

    fn rounded_small() -> Div {
        div()
            .size_16()
            .bg(rgb(0xffffff))
            .rounded(px(4.))
            .border_1()
            .border_color(hsla(0.0, 0.0, 0.0, 0.1))
    }

    fn rounded_medium() -> Div {
        div()
            .size_16()
            .bg(rgb(0xffffff))
            .rounded(px(8.))
            .border_1()
            .border_color(hsla(0.0, 0.0, 0.0, 0.1))
    }

    fn rounded_large() -> Div {
        div()
            .size_16()
            .bg(rgb(0xffffff))
            .rounded(px(12.))
            .border_1()
            .border_color(hsla(0.0, 0.0, 0.0, 0.1))
    }
}

fn example(label: impl Into<SharedString>, example: impl IntoElement) -> impl IntoElement {
    let label = label.into();

    div()
        .flex()
        .flex_col()
        .justify_center()
        .items_center()
        .w(relative(1. / 6.))
        .border_r_1()
        .border_color(hsla(0.0, 0.0, 0.0, 1.0))
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .flex_1()
                .py_12()
                .child(example),
        )
        .child(
            div()
                .w_full()
                .border_t_1()
                .border_color(hsla(0.0, 0.0, 0.0, 1.0))
                .p_1()
                .flex()
                .items_center()
                .child(label),
        )
}

impl Render for Shadow {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("shadow-example")
            .overflow_y_scroll()
            .bg(rgb(0xffffff))
            .size_full()
            .text_xs()
            .child(div().flex().flex_col().w_full().children(vec![
                div()
                    .border_b_1()
                    .border_color(hsla(0.0, 0.0, 0.0, 1.0))
                    .flex()
                    .flex_row()
                    .children(vec![
                        example(
                            "方形",
                            Shadow::square()
                                .shadow(vec![BoxShadow {
                                    color: hsla(0.0, 0.5, 0.5, 0.3),
                                    offset: point(px(0.), px(8.)),
                                    blur_radius: px(8.),
                                    spread_radius: px(0.),
                                }]),
                        ),
                        example(
                            "圆角 4",
                            Shadow::rounded_small()
                                .shadow(vec![BoxShadow {
                                    color: hsla(0.0, 0.5, 0.5, 0.3),
                                    offset: point(px(0.), px(8.)),
                                    blur_radius: px(8.),
                                    spread_radius: px(0.),
                                }]),
                        ),
                        example(
                            "圆角 8",
                            Shadow::rounded_medium()
                                .shadow(vec![BoxShadow {
                                    color: hsla(0.0, 0.5, 0.5, 0.3),
                                    offset: point(px(0.), px(8.)),
                                    blur_radius: px(8.),
                                    spread_radius: px(0.),
                                }]),
                        ),
                        example(
                            "圆角 16",
                            Shadow::rounded_large()
                                .shadow(vec![BoxShadow {
                                    color: hsla(0.0, 0.5, 0.5, 0.3),
                                    offset: point(px(0.), px(8.)),
                                    blur_radius: px(8.),
                                    spread_radius: px(0.),
                                }]),
                        ),
                        example(
                            "圆形",
                            Shadow::base()
                                .shadow(vec![BoxShadow {
                                    color: hsla(0.0, 0.5, 0.5, 0.3),
                                    offset: point(px(0.), px(8.)),
                                    blur_radius: px(8.),
                                    spread_radius: px(0.),
                                }]),
                        ),
                    ]),
                div()
                    .border_b_1()
                    .border_color(hsla(0.0, 0.0, 0.0, 1.0))
                    .flex()
                    .w_full()
                    .children(vec![
                        example("无", Shadow::base()),
                        // 2Xsmall shadow
                        example("2X 小", Shadow::base().shadow_2xs()),
                        // Xsmall shadow
                        example("超小", Shadow::base().shadow_xs()),
                        // Small shadow
                        example("小", Shadow::base().shadow_sm()),
                        // Medium shadow
                        example("中", Shadow::base().shadow_md()),
                        // Large shadow
                        example("大", Shadow::base().shadow_lg()),
                        example("超大", Shadow::base().shadow_xl()),
                        example("2X 大", Shadow::base().shadow_2xl()),
                    ]),
                // Horizontal list of increasing blur radii
                div()
                    .border_b_1()
                    .border_color(hsla(0.0, 0.0, 0.0, 1.0))
                    .flex()
                    .children(vec![
                        example(
                            "模糊 0",
                            Shadow::base().shadow(vec![BoxShadow {
                                color: hsla(0.0, 0.0, 0.0, 0.3),
                                offset: point(px(0.), px(8.)),
                                blur_radius: px(0.),
                                spread_radius: px(0.),
                            }]),
                        ),
                        example(
                            "模糊 2",
                            Shadow::base().shadow(vec![BoxShadow {
                                color: hsla(0.0, 0.0, 0.0, 0.3),
                                offset: point(px(0.), px(8.)),
                                blur_radius: px(2.),
                                spread_radius: px(0.),
                            }]),
                        ),
                        example(
                            "模糊 4",
                            Shadow::base().shadow(vec![BoxShadow {
                                color: hsla(0.0, 0.0, 0.0, 0.3),
                                offset: point(px(0.), px(8.)),
                                blur_radius: px(4.),
                                spread_radius: px(0.),
                            }]),
                        ),
                        example(
                            "模糊 8",
                            Shadow::base().shadow(vec![BoxShadow {
                                color: hsla(0.0, 0.0, 0.0, 0.3),
                                offset: point(px(0.), px(8.)),
                                blur_radius: px(8.),
                                spread_radius: px(0.),
                            }]),
                        ),
                        example(
                            "模糊 16",
                            Shadow::base().shadow(vec![BoxShadow {
                                color: hsla(0.0, 0.0, 0.0, 0.3),
                                offset: point(px(0.), px(8.)),
                                blur_radius: px(16.),
                                spread_radius: px(0.),
                            }]),
                        ),
                    ]),
                // Horizontal list of increasing spread radii
                div()
                    .border_b_1()
                    .border_color(hsla(0.0, 0.0, 0.0, 1.0))
                    .flex()
                    .children(vec![
                        example(
                            "扩展 0",
                            Shadow::base().shadow(vec![BoxShadow {
                                color: hsla(0.0, 0.0, 0.0, 0.3),
                                offset: point(px(0.), px(8.)),
                                blur_radius: px(8.),
                                spread_radius: px(0.),
                            }]),
                        ),
                        example(
                            "扩展 2",
                            Shadow::base().shadow(vec![BoxShadow {
                                color: hsla(0.0, 0.0, 0.0, 0.3),
                                offset: point(px(0.), px(8.)),
                                blur_radius: px(8.),
                                spread_radius: px(2.),
                            }]),
                        ),
                        example(
                            "扩展 4",
                            Shadow::base().shadow(vec![BoxShadow {
                                color: hsla(0.0, 0.0, 0.0, 0.3),
                                offset: point(px(0.), px(8.)),
                                blur_radius: px(8.),
                                spread_radius: px(4.),
                            }]),
                        ),
                        example(
                            "扩展 8",
                            Shadow::base().shadow(vec![BoxShadow {
                                color: hsla(0.0, 0.0, 0.0, 0.3),
                                offset: point(px(0.), px(8.)),
                                blur_radius: px(8.),
                                spread_radius: px(8.),
                            }]),
                        ),
                        example(
                            "扩展 16",
                            Shadow::base().shadow(vec![BoxShadow {
                                color: hsla(0.0, 0.0, 0.0, 0.3),
                                offset: point(px(0.), px(8.)),
                                blur_radius: px(8.),
                                spread_radius: px(16.),
                            }]),
                        ),
                    ]),
                // Square spread examples
                div()
                    .border_b_1()
                    .border_color(hsla(0.0, 0.0, 0.0, 1.0))
                    .flex()
                    .children(vec![
                        example(
                            "方形 扩展 0",
                            Shadow::square().shadow(vec![BoxShadow {
                                color: hsla(0.0, 0.0, 0.0, 0.3),
                                offset: point(px(0.), px(8.)),
                                blur_radius: px(8.),
                                spread_radius: px(0.),
                            }]),
                        ),
                        example(
                            "方形 扩展 8",
                            Shadow::square().shadow(vec![BoxShadow {
                                color: hsla(0.0, 0.0, 0.0, 0.3),
                                offset: point(px(0.), px(8.)),
                                blur_radius: px(8.),
                                spread_radius: px(8.),
                            }]),
                        ),
                        example(
                            "方形 扩展 16",
                            Shadow::square().shadow(vec![BoxShadow {
                                color: hsla(0.0, 0.0, 0.0, 0.3),
                                offset: point(px(0.), px(8.)),
                                blur_radius: px(8.),
                                spread_radius: px(16.),
                            }]),
                        ),
                    ]),
                // Rounded large spread examples
                div()
                    .border_b_1()
                    .border_color(hsla(0.0, 0.0, 0.0, 1.0))
                    .flex()
                    .children(vec![
                        example(
                            "大圆角 扩展 0",
                            Shadow::rounded_large().shadow(vec![BoxShadow {
                                color: hsla(0.0, 0.0, 0.0, 0.3),
                                offset: point(px(0.), px(8.)),
                                blur_radius: px(8.),
                                spread_radius: px(0.),
                            }]),
                        ),
                        example(
                            "大圆角 扩展 8",
                            Shadow::rounded_large().shadow(vec![BoxShadow {
                                color: hsla(0.0, 0.0, 0.0, 0.3),
                                offset: point(px(0.), px(8.)),
                                blur_radius: px(8.),
                                spread_radius: px(8.),
                            }]),
                        ),
                        example(
                            "大圆角 扩展 16",
                            Shadow::rounded_large().shadow(vec![BoxShadow {
                                color: hsla(0.0, 0.0, 0.0, 0.3),
                                offset: point(px(0.), px(8.)),
                                blur_radius: px(8.),
                                spread_radius: px(16.),
                            }]),
                        ),
                    ]),
                // Directional shadows
                div()
                    .border_b_1()
                    .border_color(hsla(0.0, 0.0, 0.0, 1.0))
                    .flex()
                    .children(vec![
                        example(
                            "左",
                            Shadow::base().shadow(vec![BoxShadow {
                                color: hsla(0.0, 0.5, 0.5, 0.3),
                                offset: point(px(-8.), px(0.)),
                                blur_radius: px(8.),
                                spread_radius: px(0.),
                            }]),
                        ),
                        example(
                            "右",
                            Shadow::base().shadow(vec![BoxShadow {
                                color: hsla(0.0, 0.5, 0.5, 0.3),
                                offset: point(px(8.), px(0.)),
                                blur_radius: px(8.),
                                spread_radius: px(0.),
                            }]),
                        ),
                        example(
                            "顶部",
                            Shadow::base().shadow(vec![BoxShadow {
                                color: hsla(0.0, 0.5, 0.5, 0.3),
                                offset: point(px(0.), px(-8.)),
                                blur_radius: px(8.),
                                spread_radius: px(0.),
                            }]),
                        ),
                        example(
                            "下",
                            Shadow::base().shadow(vec![BoxShadow {
                                color: hsla(0.0, 0.5, 0.5, 0.3),
                                offset: point(px(0.), px(8.)),
                                blur_radius: px(8.),
                                spread_radius: px(0.),
                            }]),
                        ),
                    ]),
                // Square directional shadows
                div()
                    .border_b_1()
                    .border_color(hsla(0.0, 0.0, 0.0, 1.0))
                    .flex()
                    .children(vec![
                        example(
                            "方形 左",
                            Shadow::square().shadow(vec![BoxShadow {
                                color: hsla(0.0, 0.5, 0.5, 0.3),
                                offset: point(px(-8.), px(0.)),
                                blur_radius: px(8.),
                                spread_radius: px(0.),
                            }]),
                        ),
                        example(
                            "方形 右",
                            Shadow::square().shadow(vec![BoxShadow {
                                color: hsla(0.0, 0.5, 0.5, 0.3),
                                offset: point(px(8.), px(0.)),
                                blur_radius: px(8.),
                                spread_radius: px(0.),
                            }]),
                        ),
                        example(
                            "方形 上",
                            Shadow::square().shadow(vec![BoxShadow {
                                color: hsla(0.0, 0.5, 0.5, 0.3),
                                offset: point(px(0.), px(-8.)),
                                blur_radius: px(8.),
                                spread_radius: px(0.),
                            }]),
                        ),
                        example(
                            "方形 下",
                            Shadow::square().shadow(vec![BoxShadow {
                                color: hsla(0.0, 0.5, 0.5, 0.3),
                                offset: point(px(0.), px(8.)),
                                blur_radius: px(8.),
                                spread_radius: px(0.),
                            }]),
                        ),
                    ]),
                // Rounded large directional shadows
                div()
                    .border_b_1()
                    .border_color(hsla(0.0, 0.0, 0.0, 1.0))
                    .flex()
                    .children(vec![
                        example(
                            "大圆角 左",
                            Shadow::rounded_large().shadow(vec![BoxShadow {
                                color: hsla(0.0, 0.5, 0.5, 0.3),
                                offset: point(px(-8.), px(0.)),
                                blur_radius: px(8.),
                                spread_radius: px(0.),
                            }]),
                        ),
                        example(
                            "大圆角 右",
                            Shadow::rounded_large().shadow(vec![BoxShadow {
                                color: hsla(0.0, 0.5, 0.5, 0.3),
                                offset: point(px(8.), px(0.)),
                                blur_radius: px(8.),
                                spread_radius: px(0.),
                            }]),
                        ),
                        example(
                            "大圆角 上",
                            Shadow::rounded_large().shadow(vec![BoxShadow {
                                color: hsla(0.0, 0.5, 0.5, 0.3),
                                offset: point(px(0.), px(-8.)),
                                blur_radius: px(8.),
                                spread_radius: px(0.),
                            }]),
                        ),
                        example(
                            "大圆角 下",
                            Shadow::rounded_large().shadow(vec![BoxShadow {
                                color: hsla(0.0, 0.5, 0.5, 0.3),
                                offset: point(px(0.), px(8.)),
                                blur_radius: px(8.),
                                spread_radius: px(0.),
                            }]),
                        ),
                    ]),
                // Multiple shadows for different shapes
                div()
                    .border_b_1()
                    .border_color(hsla(0.0, 0.0, 0.0, 1.0))
                    .flex()
                    .children(vec![
                        example(
                            "圆形 多重",
                            Shadow::base().shadow(vec![
                                BoxShadow {
                                    color: hsla(0.0 / 360., 1.0, 0.5, 0.3), // Red
                                    offset: point(px(0.), px(-12.)),
                                    blur_radius: px(8.),
                                    spread_radius: px(2.),
                                },
                                BoxShadow {
                                    color: hsla(60.0 / 360., 1.0, 0.5, 0.3), // Yellow
                                    offset: point(px(12.), px(0.)),
                                    blur_radius: px(8.),
                                    spread_radius: px(2.),
                                },
                                BoxShadow {
                                    color: hsla(120.0 / 360., 1.0, 0.5, 0.3), // Green
                                    offset: point(px(0.), px(12.)),
                                    blur_radius: px(8.),
                                    spread_radius: px(2.),
                                },
                                BoxShadow {
                                    color: hsla(240.0 / 360., 1.0, 0.5, 0.3), // Blue
                                    offset: point(px(-12.), px(0.)),
                                    blur_radius: px(8.),
                                    spread_radius: px(2.),
                                },
                            ]),
                        ),
                        example(
                            "方形 多重",
                            Shadow::square().shadow(vec![
                                BoxShadow {
                                    color: hsla(0.0 / 360., 1.0, 0.5, 0.3), // Red
                                    offset: point(px(0.), px(-12.)),
                                    blur_radius: px(8.),
                                    spread_radius: px(2.),
                                },
                                BoxShadow {
                                    color: hsla(60.0 / 360., 1.0, 0.5, 0.3), // Yellow
                                    offset: point(px(12.), px(0.)),
                                    blur_radius: px(8.),
                                    spread_radius: px(2.),
                                },
                                BoxShadow {
                                    color: hsla(120.0 / 360., 1.0, 0.5, 0.3), // Green
                                    offset: point(px(0.), px(12.)),
                                    blur_radius: px(8.),
                                    spread_radius: px(2.),
                                },
                                BoxShadow {
                                    color: hsla(240.0 / 360., 1.0, 0.5, 0.3), // Blue
                                    offset: point(px(-12.), px(0.)),
                                    blur_radius: px(8.),
                                    spread_radius: px(2.),
                                },
                            ]),
                        ),
                        example(
                            "大圆角 多重",
                            Shadow::rounded_large().shadow(vec![
                                BoxShadow {
                                    color: hsla(0.0 / 360., 1.0, 0.5, 0.3), // Red
                                    offset: point(px(0.), px(-12.)),
                                    blur_radius: px(8.),
                                    spread_radius: px(2.),
                                },
                                BoxShadow {
                                    color: hsla(60.0 / 360., 1.0, 0.5, 0.3), // Yellow
                                    offset: point(px(12.), px(0.)),
                                    blur_radius: px(8.),
                                    spread_radius: px(2.),
                                },
                                BoxShadow {
                                    color: hsla(120.0 / 360., 1.0, 0.5, 0.3), // Green
                                    offset: point(px(0.), px(12.)),
                                    blur_radius: px(8.),
                                    spread_radius: px(2.),
                                },
                                BoxShadow {
                                    color: hsla(240.0 / 360., 1.0, 0.5, 0.3), // Blue
                                    offset: point(px(-12.), px(0.)),
                                    blur_radius: px(8.),
                                    spread_radius: px(2.),
                                },
                            ]),
                        ),
                    ]),
            ]))
    }
}

fn run_example() {
    application().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1000.0), px(800.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(|_| Shadow {}),
        )
        .unwrap();

        cx.activate(true);
    });
}

#[cfg(not(target_family = "wasm"))]
fn main() {
    run_example();
}

#[cfg(target_family = "wasm")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
    gpui_platform::web_init();
    run_example();
}
