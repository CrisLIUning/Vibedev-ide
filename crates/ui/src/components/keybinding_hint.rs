use crate::KeyBinding;
use crate::prelude::*;
use gpui::{AnyElement, App, BoxShadow, FontStyle, Hsla, IntoElement, Window, point};
use theme::Appearance;

/// Represents a hint for a keybinding, optionally with a prefix and suffix.
///
/// This struct allows for the creation and customization of a keybinding hint,
/// which can be used to display keyboard shortcuts or commands in a user interface.
///
/// # Examples
///
/// ```no_run
/// use gpui::{App, Hsla, KeybindingKeystroke, Keystroke};
/// use ui::prelude::*;
/// use ui::{KeyBinding, KeybindingHint};
///
/// # fn example(cx: &App) {
/// let hint = KeybindingHint::new(
///     KeyBinding::from_keystrokes(vec![KeybindingKeystroke::from_keystroke(Keystroke::parse("ctrl-s").unwrap())].into(), false),
///     Hsla::black()
/// )
///     .prefix("保存:")
///     .size(Pixels::from(14.0));
/// # }
/// ```
#[derive(Debug, IntoElement, RegisterComponent)]
pub struct KeybindingHint {
    prefix: Option<SharedString>,
    suffix: Option<SharedString>,
    keybinding: KeyBinding,
    size: Option<Pixels>,
    background_color: Hsla,
}

impl KeybindingHint {
    /// Creates a new `KeybindingHint` with the specified keybinding.
    ///
    /// This method initializes a new `KeybindingHint` instance with the given keybinding,
    /// setting all other fields to their default values.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use gpui::{App, Hsla, KeybindingKeystroke, Keystroke};
    /// use ui::prelude::*;
    /// use ui::{KeyBinding, KeybindingHint};
    ///
    /// # fn example(cx: &App) {
    /// let hint = KeybindingHint::new(
    ///     KeyBinding::from_keystrokes(vec![KeybindingKeystroke::from_keystroke(Keystroke::parse("ctrl-c").unwrap())].into(), false),
    ///     Hsla::black()
    /// );
    /// # }
    /// ```
    pub fn new(keybinding: KeyBinding, background_color: Hsla) -> Self {
        Self {
            prefix: None,
            suffix: None,
            keybinding,
            size: None,
            background_color,
        }
    }

    /// Creates a new `KeybindingHint` with a prefix and keybinding.
    ///
    /// This method initializes a new `KeybindingHint` instance with the given prefix and keybinding,
    /// setting all other fields to their default values.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use gpui::{App, Hsla, KeybindingKeystroke, Keystroke};
    /// use ui::prelude::*;
    /// use ui::{KeyBinding, KeybindingHint};
    ///
    /// # fn example(cx: &App) {
    /// let hint = KeybindingHint::with_prefix(
    ///     "复制:",
    ///     KeyBinding::from_keystrokes(vec![KeybindingKeystroke::from_keystroke(Keystroke::parse("ctrl-c").unwrap())].into(), false),
    ///     Hsla::black()
    /// );
    /// # }
    /// ```
    pub fn with_prefix(
        prefix: impl Into<SharedString>,
        keybinding: KeyBinding,
        background_color: Hsla,
    ) -> Self {
        Self {
            prefix: Some(prefix.into()),
            suffix: None,
            keybinding,
            size: None,
            background_color,
        }
    }

    /// Creates a new `KeybindingHint` with a keybinding and suffix.
    ///
    /// This method initializes a new `KeybindingHint` instance with the given keybinding and suffix,
    /// setting all other fields to their default values.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use gpui::{App, Hsla, KeybindingKeystroke, Keystroke};
    /// use ui::prelude::*;
    /// use ui::{KeyBinding, KeybindingHint};
    ///
    /// # fn example(cx: &App) {
    /// let hint = KeybindingHint::with_suffix(
    ///     KeyBinding::from_keystrokes(vec![KeybindingKeystroke::from_keystroke(Keystroke::parse("ctrl-v").unwrap())].into(), false),
    ///     "粘贴",
    ///     Hsla::black()
    /// );
    /// # }
    /// ```
    pub fn with_suffix(
        keybinding: KeyBinding,
        suffix: impl Into<SharedString>,
        background_color: Hsla,
    ) -> Self {
        Self {
            prefix: None,
            suffix: Some(suffix.into()),
            keybinding,
            size: None,
            background_color,
        }
    }

    /// Sets the prefix for the keybinding hint.
    ///
    /// This method allows adding or changing the prefix text that appears before the keybinding.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use gpui::{App, Hsla, KeybindingKeystroke, Keystroke};
    /// use ui::prelude::*;
    /// use ui::{KeyBinding, KeybindingHint};
    ///
    /// # fn example(cx: &App) {
    /// let hint = KeybindingHint::new(
    ///     KeyBinding::from_keystrokes(vec![KeybindingKeystroke::from_keystroke(Keystroke::parse("ctrl-x").unwrap())].into(), false),
    ///     Hsla::black()
    /// )
    ///     .prefix("剪切:");
    /// # }
    /// ```
    pub fn prefix(mut self, prefix: impl Into<SharedString>) -> Self {
        self.prefix = Some(prefix.into());
        self
    }

    /// Sets the suffix for the keybinding hint.
    ///
    /// This method allows adding or changing the suffix text that appears after the keybinding.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use gpui::{App, Hsla, KeybindingKeystroke, Keystroke};
    /// use ui::prelude::*;
    /// use ui::{KeyBinding, KeybindingHint};
    ///
    /// # fn example(cx: &App) {
    /// let hint = KeybindingHint::new(
    ///     KeyBinding::from_keystrokes(vec![KeybindingKeystroke::from_keystroke(Keystroke::parse("ctrl-f").unwrap())].into(), false),
    ///     Hsla::black()
    /// )
    ///     .suffix("查找");
    /// # }
    /// ```
    pub fn suffix(mut self, suffix: impl Into<SharedString>) -> Self {
        self.suffix = Some(suffix.into());
        self
    }

    /// Sets the size of the keybinding hint.
    ///
    /// This method allows specifying the size of the keybinding hint in pixels.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use gpui::{App, Hsla, KeybindingKeystroke, Keystroke};
    /// use ui::prelude::*;
    /// use ui::{KeyBinding, KeybindingHint};
    ///
    /// # fn example(cx: &App) {
    /// let hint = KeybindingHint::new(
    ///     KeyBinding::from_keystrokes(vec![KeybindingKeystroke::from_keystroke(Keystroke::parse("ctrl-z").unwrap())].into(), false),
    ///     Hsla::black()
    /// )
    ///     .size(Pixels::from(16.0));
    /// # }
    /// ```
    pub fn size(mut self, size: impl Into<Option<Pixels>>) -> Self {
        self.size = size.into();
        self
    }
}

impl RenderOnce for KeybindingHint {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = cx.theme().colors();
        let is_light = cx.theme().appearance() == Appearance::Light;

        let border_color =
            self.background_color
                .blend(colors.text.alpha(if is_light { 0.08 } else { 0.16 }));

        let bg_color = self
            .background_color
            .blend(colors.text_accent.alpha(if is_light { 0.05 } else { 0.1 }));

        let shadow_color = colors.text.alpha(if is_light { 0.04 } else { 0.08 });

        let size = self
            .size
            .unwrap_or(TextSize::Small.rems(cx).to_pixels(window.rem_size()));

        let kb_size = size - px(2.0);

        let mut base = h_flex();

        base.text_style().font_style = Some(FontStyle::Italic);

        base.gap_1()
            .font_buffer(cx)
            .text_size(size)
            .text_color(colors.text_disabled)
            .children(self.prefix)
            .child(
                h_flex()
                    .rounded_sm()
                    .px_0p5()
                    .mr_0p5()
                    .border_1()
                    .border_color(border_color)
                    .bg(bg_color)
                    .shadow(vec![BoxShadow {
                        color: shadow_color,
                        offset: point(px(0.), px(1.)),
                        blur_radius: px(0.),
                        spread_radius: px(0.),
                    }])
                    .child(self.keybinding.size(rems_from_px(kb_size))),
            )
            .children(self.suffix)
    }
}

impl Component for KeybindingHint {
    fn scope() -> ComponentScope {
        ComponentScope::DataDisplay
    }

    fn description() -> Option<&'static str> {
        Some("显示带有可选前缀和后缀文本的键盘快捷键提示")
    }

    fn preview(_window: &mut Window, cx: &mut App) -> Option<AnyElement> {
        let enter = KeyBinding::for_action(&menu::Confirm, cx);

        let bg_color = cx.theme().colors().surface_background;

        Some(
            v_flex()
                .gap_6()
                .children(vec![
                    example_group_with_title(
                        "基础",
                        vec![
                            single_example(
                                "带前缀",
                                KeybindingHint::with_prefix(
                                    "跳转到开头:",
                                    enter.clone(),
                                    bg_color,
                                )
                                .into_any_element(),
                            ),
                            single_example(
                                "带后缀",
                                KeybindingHint::with_suffix(enter.clone(), "跳转到结尾", bg_color)
                                    .into_any_element(),
                            ),
                            single_example(
                                "带前缀和后缀",
                                KeybindingHint::new(enter.clone(), bg_color)
                                    .prefix("确认:")
                                    .suffix("执行选中的操作")
                                    .into_any_element(),
                            ),
                        ],
                    ),
                    example_group_with_title(
                        "尺寸",
                        vec![
                            single_example(
                                "小",
                                KeybindingHint::new(enter.clone(), bg_color)
                                    .size(Pixels::from(12.0))
                                    .prefix("小:")
                                    .into_any_element(),
                            ),
                            single_example(
                                "中",
                                KeybindingHint::new(enter.clone(), bg_color)
                                    .size(Pixels::from(16.0))
                                    .suffix("中")
                                    .into_any_element(),
                            ),
                            single_example(
                                "大",
                                KeybindingHint::new(enter, bg_color)
                                    .size(Pixels::from(20.0))
                                    .prefix("大:")
                                    .suffix("尺寸")
                                    .into_any_element(),
                            ),
                        ],
                    ),
                ])
                .into_any_element(),
        )
    }
}
