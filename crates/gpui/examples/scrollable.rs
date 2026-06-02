#![cfg_attr(target_family = "wasm", no_main)]

use gpui::{App, Bounds, Context, Window, WindowBounds, WindowOptions, div, prelude::*, px, size};
use gpui_platform::application;

struct Scrollable {}

impl Render for Scrollable {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .id("vertical")
            .p_4()
            .overflow_scroll()
            .bg(gpui::white())
            .child("嵌套布局中双向滚动测试示例")
            .child(
                div()
                    .h(px(5000.))
                    .border_1()
                    .border_color(gpui::blue())
                    .bg(gpui::blue().opacity(0.05))
                    .p_4()
                    .child(
                        div()
                            .mb_5()
                            .w_full()
                            .id("horizontal")
                            .overflow_scroll()
                            .child(
                                div()
                                    .w(px(2000.))
                                    .h(px(150.))
                                    .bg(gpui::green().opacity(0.1))
                                    .hover(|this| this.bg(gpui::green().opacity(0.2)))
                                    .border_1()
                                    .border_color(gpui::green())
                                    .p_4()
                                    .child("水平滚动"),
                            ),
                    )
                    .child("垂直滚动"),
            )
    }
}

fn run_example() {
    application().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(500.), px(500.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(|_| Scrollable {}),
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
