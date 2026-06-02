use gpui::{App, IntoElement, Modifiers, RenderOnce, Window};
use ui::{prelude::*, render_modifiers};

#[derive(IntoElement)]
pub struct HoldForDefault {
    is_default: bool,
    more_content: bool,
}

impl HoldForDefault {
    pub fn new(is_default: bool) -> Self {
        Self {
            is_default,
            more_content: true,
        }
    }

    #[allow(dead_code)]
    pub fn more_content(mut self, more_content: bool) -> Self {
        self.more_content = more_content;
        self
    }
}

impl RenderOnce for HoldForDefault {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        h_flex()
            .when(self.more_content, |this| {
                this.pt_1()
                    .border_t_1()
                    .border_color(cx.theme().colors().border_variant)
            })
            .gap_0p5()
            .text_sm()
            .text_color(Color::Muted.color(cx))
            .child("按住")
            .child(h_flex().flex_shrink_0().children(render_modifiers(
                &Modifiers::secondary_key(),
                PlatformStyle::platform(),
                None,
                Some(TextSize::Default.rems(cx).into()),
                false,
            )))
            .child(div().map(|this| {
                if self.is_default {
                    this.child("取消默认设置")
                } else {
                    this.child("设为默认")
                }
            }))
    }
}
