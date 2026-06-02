#![cfg_attr(target_family = "wasm", no_main)]

use gpui::{
    App, Bounds, Context, KeyBinding, PromptButton, PromptLevel, Window, WindowBounds, WindowKind,
    WindowOptions, actions, div, prelude::*, px, rgb, size,
};
use gpui_platform::application;

struct SubWindow {
    custom_titlebar: bool,
    is_dialog: bool,
}

fn button(text: &str, on_click: impl Fn(&mut Window, &mut App) + 'static) -> impl IntoElement {
    div()
        .id(text.to_string())
        .flex_none()
        .px_2()
        .bg(rgb(0xf7f7f7))
        .active(|this| this.opacity(0.85))
        .border_1()
        .border_color(rgb(0xe0e0e0))
        .rounded_sm()
        .cursor_pointer()
        .child(text.to_string())
        .on_click(move |_, window, cx| on_click(window, cx))
}

impl Render for SubWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let window_bounds =
            WindowBounds::Windowed(Bounds::centered(None, size(px(250.0), px(200.0)), cx));

        div()
            .flex()
            .flex_col()
            .bg(rgb(0xffffff))
            .size_full()
            .gap_2()
            .when(self.custom_titlebar, |cx| {
                cx.child(
                    div()
                        .flex()
                        .h(px(32.))
                        .px_4()
                        .bg(gpui::blue())
                        .text_color(gpui::white())
                        .w_full()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_center()
                                .size_full()
                                .child("自定义标题栏"),
                        ),
                )
            })
            .child(
                div()
                    .p_8()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child("子窗口")
                    .when(self.is_dialog, |div| {
                        div.child(button("打开嵌套对话框", move |_, cx| {
                            cx.open_window(
                                WindowOptions {
                                    window_bounds: Some(window_bounds),
                                    kind: WindowKind::Dialog,
                                    ..Default::default()
                                },
                                |_, cx| {
                                    cx.new(|_| SubWindow {
                                        custom_titlebar: false,
                                        is_dialog: true,
                                    })
                                },
                            )
                            .unwrap();
                        }))
                    })
                    .child(button("关闭", |window, _| {
                        window.remove_window();
                    })),
            )
    }
}

struct WindowDemo {}

impl Render for WindowDemo {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let window_bounds =
            WindowBounds::Windowed(Bounds::centered(None, size(px(300.0), px(300.0)), cx));

        div()
            .p_4()
            .flex()
            .flex_wrap()
            .bg(rgb(0xffffff))
            .size_full()
            .justify_center()
            .content_center()
            .gap_2()
            .child(button("普通", move |_, cx| {
                cx.open_window(
                    WindowOptions {
                        window_bounds: Some(window_bounds),
                        ..Default::default()
                    },
                    |_, cx| {
                        cx.new(|_| SubWindow {
                            custom_titlebar: false,
                            is_dialog: false,
                        })
                    },
                )
                .unwrap();
            }))
            .child(button("弹出窗口", move |_, cx| {
                cx.open_window(
                    WindowOptions {
                        window_bounds: Some(window_bounds),
                        kind: WindowKind::PopUp,
                        ..Default::default()
                    },
                    |_, cx| {
                        cx.new(|_| SubWindow {
                            custom_titlebar: false,
                            is_dialog: false,
                        })
                    },
                )
                .unwrap();
            }))
            .child(button("浮动窗口", move |_, cx| {
                cx.open_window(
                    WindowOptions {
                        window_bounds: Some(window_bounds),
                        kind: WindowKind::Floating,
                        ..Default::default()
                    },
                    |_, cx| {
                        cx.new(|_| SubWindow {
                            custom_titlebar: false,
                            is_dialog: false,
                        })
                    },
                )
                .unwrap();
            }))
            .child(button("对话框", move |_, cx| {
                cx.open_window(
                    WindowOptions {
                        window_bounds: Some(window_bounds),
                        kind: WindowKind::Dialog,
                        ..Default::default()
                    },
                    |_, cx| {
                        cx.new(|_| SubWindow {
                            custom_titlebar: false,
                            is_dialog: true,
                        })
                    },
                )
                .unwrap();
            }))
            .child(button("自定义标题栏", move |_, cx| {
                cx.open_window(
                    WindowOptions {
                        titlebar: None,
                        window_bounds: Some(window_bounds),
                        ..Default::default()
                    },
                    |_, cx| {
                        cx.new(|_| SubWindow {
                            custom_titlebar: true,
                            is_dialog: false,
                        })
                    },
                )
                .unwrap();
            }))
            .child(button("不可见", move |_, cx| {
                cx.open_window(
                    WindowOptions {
                        show: false,
                        window_bounds: Some(window_bounds),
                        ..Default::default()
                    },
                    |_, cx| {
                        cx.new(|_| SubWindow {
                            custom_titlebar: false,
                            is_dialog: false,
                        })
                    },
                )
                .unwrap();
            }))
            .child(button("不可移动", move |_, cx| {
                cx.open_window(
                    WindowOptions {
                        is_movable: false,
                        titlebar: None,
                        window_bounds: Some(window_bounds),
                        ..Default::default()
                    },
                    |_, cx| {
                        cx.new(|_| SubWindow {
                            custom_titlebar: false,
                            is_dialog: false,
                        })
                    },
                )
                .unwrap();
            }))
            .child(button("不可调整大小", move |_, cx| {
                cx.open_window(
                    WindowOptions {
                        is_resizable: false,
                        window_bounds: Some(window_bounds),
                        ..Default::default()
                    },
                    |_, cx| {
                        cx.new(|_| SubWindow {
                            custom_titlebar: false,
                            is_dialog: false,
                        })
                    },
                )
                .unwrap();
            }))
            .child(button("不可最小化", move |_, cx| {
                cx.open_window(
                    WindowOptions {
                        is_minimizable: false,
                        window_bounds: Some(window_bounds),
                        ..Default::default()
                    },
                    |_, cx| {
                        cx.new(|_| SubWindow {
                            custom_titlebar: false,
                            is_dialog: false,
                        })
                    },
                )
                .unwrap();
            }))
            .child(button("隐藏应用", |window, cx| {
                cx.hide();

                // Restore the application after 3 seconds
                window
                    .spawn(cx, async move |cx| {
                        cx.background_executor()
                            .timer(std::time::Duration::from_secs(3))
                            .await;
                        cx.update(|_, cx| {
                            cx.activate(false);
                        })
                    })
                    .detach();
            }))
            .child(button("调整大小", |window, _| {
                let content_size = window.bounds().size;
                window.resize(size(content_size.height, content_size.width));
            }))
            .child(button("提示", |window, cx| {
                let answer = window.prompt(
                    PromptLevel::Info,
                    "您确定吗?",
                    None,
                    &["确定", "取消"],
                    cx,
                );

                cx.spawn(async move |_| {
                    if answer.await.unwrap() == 0 {
                        println!("你点击了确定");
                    } else {
                        println!("你点击了取消");
                    }
                })
                .detach();
            }))
            .child(button("提示 (非英语)", |window, cx| {
                let answer = window.prompt(
                    PromptLevel::Info,
                    "您确定吗?",
                    None,
                    &[PromptButton::ok("确定"), PromptButton::cancel("取消")],
                    cx,
                );

                cx.spawn(async move |_| {
                    if answer.await.unwrap() == 0 {
                        println!("你点击了确定");
                    } else {
                        println!("你点击了取消");
                    }
                })
                .detach();
            }))
    }
}

actions!(window, [Quit]);

fn run_example() {
    application().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(800.0), px(600.0)), cx);

        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |window, cx| {
                cx.new(|cx| {
                    cx.observe_window_bounds(window, move |_, window, _| {
                        println!("窗口边界已更改: {:?}", window.bounds());
                    })
                    .detach();

                    WindowDemo {}
                })
            },
        )
        .unwrap();

        cx.activate(true);
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
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
