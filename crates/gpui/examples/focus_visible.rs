#![cfg_attr(target_family = "wasm", no_main)]

use gpui::{
    App, Bounds, Context, Div, ElementId, FocusHandle, KeyBinding, SharedString, Stateful, Window,
    WindowBounds, WindowOptions, actions, div, prelude::*, px, size,
};
use gpui_platform::application;

actions!(example, [Tab, TabPrev, Quit]);

struct Example {
    focus_handle: FocusHandle,
    items: Vec<(FocusHandle, &'static str)>,
    message: SharedString,
}

impl Example {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let items = vec![
            (
                cx.focus_handle().tab_index(1).tab_stop(true),
                "使用 .focus() 的按钮 - 获得焦点时始终显示边框",
            ),
            (
                cx.focus_handle().tab_index(2).tab_stop(true),
                "使用 .focus_visible() 的按钮 - 仅在使用键盘时显示边框",
            ),
            (
                cx.focus_handle().tab_index(3).tab_stop(true),
                "同时使用 .focus() 和 .focus_visible() 的按钮",
            ),
        ];

        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);

        Self {
            focus_handle,
            items,
            message: SharedString::from(
                "尝试点击或按 标签页 键!点击不显示边框,标签页 键显示边框。",
            ),
        }
    }

    fn on_tab(&mut self, _: &Tab, window: &mut Window, cx: &mut Context<Self>) {
        window.focus_next(cx);
        self.message = SharedString::from("Pressed Tab - focus-visible border should appear!");
    }

    fn on_tab_prev(&mut self, _: &TabPrev, window: &mut Window, cx: &mut Context<Self>) {
        window.focus_prev(cx);
        self.message =
            SharedString::from("Pressed Shift-Tab - focus-visible border should appear!");
    }

    fn on_quit(&mut self, _: &Quit, _window: &mut Window, cx: &mut Context<Self>) {
        cx.quit();
    }
}

impl Render for Example {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        fn button_base(id: impl Into<ElementId>, label: &'static str) -> Stateful<Div> {
            div()
                .id(id)
                .h_16()
                .w_full()
                .flex()
                .justify_center()
                .items_center()
                .bg(gpui::rgb(0x2563eb))
                .text_color(gpui::white())
                .rounded_md()
                .cursor_pointer()
                .hover(|style| style.bg(gpui::rgb(0x1d4ed8)))
                .child(label)
        }

        div()
            .id("app")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_tab))
            .on_action(cx.listener(Self::on_tab_prev))
            .on_action(cx.listener(Self::on_quit))
            .size_full()
            .flex()
            .flex_col()
            .p_8()
            .gap_6()
            .bg(gpui::rgb(0xf3f4f6))
            .child(
                div()
                    .text_2xl()
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_color(gpui::rgb(0x111827))
                    .child("CSS focus-visible 演示"),
            )
            .child(
                div()
                    .p_4()
                    .rounded_md()
                    .bg(gpui::rgb(0xdbeafe))
                    .text_color(gpui::rgb(0x1e3a8a))
                    .child(self.message.clone()),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(gpui::rgb(0x374151))
                                    .child("1. 常规 .focus() - 始终可见:"),
                            )
                            .child(
                                button_base("button1", self.items[0].1)
                                    .track_focus(&self.items[0].0)
                                    .focus(|style| {
                                        style.border_4().border_color(gpui::rgb(0xfbbf24))
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.message =
                                            "点击了按钮 1 - 焦点边框可见!".into();
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(gpui::rgb(0x374151))
                                    .child("2. 新的 .focus_visible() - 仅键盘:"),
                            )
                            .child(
                                button_base("button2", self.items[1].1)
                                    .track_focus(&self.items[1].0)
                                    .focus_visible(|style| {
                                        style.border_4().border_color(gpui::rgb(0x10b981))
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.message =
                                            "点击了按钮 2 - 无边框!请尝试 标签页 键。".into();
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(gpui::rgb(0x374151))
                                    .child(
                                        "3. 同时使用 .focus()(黄色)和 .focus_visible()(绿色):",
                                    ),
                            )
                            .child(
                                button_base("button3", self.items[2].1)
                                    .track_focus(&self.items[2].0)
                                    .focus(|style| {
                                        style.border_4().border_color(gpui::rgb(0xfbbf24))
                                    })
                                    .focus_visible(|style| {
                                        style.border_4().border_color(gpui::rgb(0x10b981))
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.message =
                                            "点击了按钮 3 - 黄色边框。标签页 键显示绿色!"
                                                .into();
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
    }
}

fn run_example() {
    application().run(|cx: &mut App| {
        cx.bind_keys([
            KeyBinding::new("tab", Tab, None),
            KeyBinding::new("shift-tab", TabPrev, None),
            KeyBinding::new("cmd-q", Quit, None),
        ]);

        let bounds = Bounds::centered(None, size(px(800.), px(600.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| Example::new(window, cx)),
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
