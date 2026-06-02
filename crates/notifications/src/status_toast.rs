use std::rc::Rc;

use gpui::{DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, IntoElement};
use ui::{Tooltip, prelude::*};
use workspace::{ToastAction, ToastView};
use zed_actions::toast;

#[derive(RegisterComponent)]
pub struct StatusToast {
    icon: Option<Icon>,
    text: SharedString,
    action: Option<ToastAction>,
    show_dismiss: bool,
    auto_dismiss: bool,
    this_handle: Entity<Self>,
    focus_handle: FocusHandle,
}

impl StatusToast {
    pub fn new(
        text: impl Into<SharedString>,
        cx: &mut App,
        f: impl FnOnce(Self, &mut Context<Self>) -> Self,
    ) -> Entity<Self> {
        cx.new(|cx| {
            let focus_handle = cx.focus_handle();

            f(
                Self {
                    text: text.into(),
                    icon: None,
                    action: None,
                    show_dismiss: false,
                    auto_dismiss: true,
                    this_handle: cx.entity(),
                    focus_handle,
                },
                cx,
            )
        })
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn auto_dismiss(mut self, auto_dismiss: bool) -> Self {
        self.auto_dismiss = auto_dismiss;
        self
    }

    pub fn action(
        mut self,
        label: impl Into<SharedString>,
        f: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        let this_handle = self.this_handle.clone();
        self.action = Some(ToastAction::new(
            label.into(),
            Some(Rc::new(move |window, cx| {
                this_handle.update(cx, |_, cx| {
                    cx.emit(DismissEvent);
                });
                f(window, cx);
            })),
        ));
        self
    }

    pub fn dismiss_button(mut self, show: bool) -> Self {
        self.show_dismiss = show;
        self
    }
}

impl Render for StatusToast {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let has_action_or_dismiss = self.action.is_some() || self.show_dismiss;

        h_flex()
            .id("status-toast")
            .elevation_3(cx)
            .gap_2()
            .py_1p5()
            .pl_2p5()
            .map(|this| {
                if has_action_or_dismiss {
                    this.pr_1p5()
                } else {
                    this.pr_2p5()
                }
            })
            .flex_none()
            .bg(cx.theme().colors().surface_background)
            .shadow_lg()
            .when_some(self.icon.clone(), |this, icon| this.child(icon))
            .child(Label::new(self.text.clone()).color(Color::Default))
            .when_some(self.action.as_ref(), |this, action| {
                this.child(
                    Button::new(action.id.clone(), action.label.clone())
                        .tooltip(Tooltip::for_action_title(
                            action.label.clone(),
                            &toast::RunAction,
                        ))
                        .color(Color::Muted)
                        .when_some(action.on_click.clone(), |el, handler| {
                            el.on_click(move |_click_event, window, cx| handler(window, cx))
                        }),
                )
            })
            .when(self.show_dismiss, |this| {
                let handle = self.this_handle.clone();
                this.child(
                    IconButton::new("dismiss", IconName::Close)
                        .shape(ui::IconButtonShape::Square)
                        .icon_size(IconSize::Small)
                        .icon_color(Color::Muted)
                        .tooltip(Tooltip::text("关闭"))
                        .on_click(move |_click_event, _window, cx| {
                            handle.update(cx, |_, cx| {
                                cx.emit(DismissEvent);
                            });
                        }),
                )
            })
    }
}

impl ToastView for StatusToast {
    fn action(&self) -> Option<ToastAction> {
        self.action.clone()
    }

    fn auto_dismiss(&self) -> bool {
        self.auto_dismiss
    }
}

impl Focusable for StatusToast {
    fn focus_handle(&self, _cx: &App) -> gpui::FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<DismissEvent> for StatusToast {}

impl Component for StatusToast {
    fn scope() -> ComponentScope {
        ComponentScope::Notification
    }

    fn preview(_window: &mut Window, cx: &mut App) -> Option<AnyElement> {
        let text_example = StatusToast::new("操作已完成", cx, |this, _| this);

        let action_example = StatusToast::new("更新已准备好安装", cx, |this, _cx| {
            this.action("重启", |_, _| {})
        });

        let dismiss_button_example =
            StatusToast::new("关闭按钮", cx, |this, _| this.dismiss_button(true));

        let icon_example = StatusToast::new(
            "Nathan Sobo 接受了您的好友请求",
            cx,
            |this, _| {
                this.icon(
                    Icon::new(IconName::Check)
                        .size(IconSize::Small)
                        .color(Color::Muted),
                )
            },
        );

        let success_example = StatusToast::new("已推送 4 个更改到 `zed/main`", cx, |this, _| {
            this.icon(
                Icon::new(IconName::Check)
                    .size(IconSize::Small)
                    .color(Color::Success),
            )
        });

        let error_example = StatusToast::new(
            "git push: 无法找到远程源 `iamnbutler/zed`",
            cx,
            |this, _cx| {
                this.icon(
                    Icon::new(IconName::XCircle)
                        .size(IconSize::Small)
                        .color(Color::Error),
                )
                .action("更多信息", |_, _| {})
            },
        );

        let warning_example = StatusToast::new("您的设置已过时", cx, |this, _cx| {
            this.icon(
                Icon::new(IconName::Warning)
                    .size(IconSize::Small)
                    .color(Color::Warning),
            )
            .action("更多信息", |_, _| {})
        });

        let pr_example =
            StatusToast::new("`zed/new-notification-system` 已创建!", cx, |this, _cx| {
                this.icon(
                    Icon::new(IconName::GitBranch)
                        .size(IconSize::Small)
                        .color(Color::Muted),
                )
                .action("打开 Pull Request", |_, cx| {
                    cx.open_url("https://github.com/")
                })
            });

        Some(
            v_flex()
                .gap_6()
                .p_4()
                .children(vec![
                    example_group_with_title(
                        "基本提示",
                        vec![
                            single_example("文本", div().child(text_example).into_any_element()),
                            single_example(
                                "操作",
                                div().child(action_example).into_any_element(),
                            ),
                            single_example("图标", div().child(icon_example).into_any_element()),
                            single_example(
                                "关闭按钮",
                                div().child(dismiss_button_example).into_any_element(),
                            ),
                        ],
                    ),
                    example_group_with_title(
                        "示例",
                        vec![
                            single_example(
                                "成功",
                                div().child(success_example).into_any_element(),
                            ),
                            single_example("错误", div().child(error_example).into_any_element()),
                            single_example(
                                "警告",
                                div().child(warning_example).into_any_element(),
                            ),
                            single_example("创建 PR", div().child(pr_example).into_any_element()),
                        ],
                    )
                    .vertical(),
                ])
                .into_any_element(),
        )
    }
}
