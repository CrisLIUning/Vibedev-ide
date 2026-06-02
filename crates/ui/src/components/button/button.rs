use crate::component_prelude::*;
use gpui::{AnyElement, AnyView, DefiniteLength};
use ui_macros::RegisterComponent;

use crate::traits::animation_ext::CommonAnimationExt;
use crate::{ButtonCommon, ButtonLike, ButtonSize, ButtonStyle, Icon, Label};
use crate::{
    Color, DynamicSpacing, ElevationIndex, KeyBinding, KeybindingPosition, TintColor, prelude::*,
};

/// An element that creates a button with a label and optional icons.
///
/// Common buttons:
/// - Label, Icon + Label: [`Button`] (this component)
/// - Icon only: [`IconButton`]
/// - Custom: [`ButtonLike`]
///
/// To create a more complex button than what the [`Button`] or [`IconButton`] components provide, use
/// [`ButtonLike`] directly.
///
/// # Examples
///
/// **A button with a label**, is typically used in scenarios such as a form, where the button's label
/// indicates what action will be performed when the button is clicked.
///
/// ```
/// use ui::prelude::*;
///
/// Button::new("button_id", "点击我!")
///     .on_click(|event, window, cx| {
///         // Handle click event
///     });
/// ```
///
/// **A toggleable button**, is typically used in scenarios such as a toolbar,
/// where the button's state indicates whether a feature is enabled or not, or
/// a trigger for a popover menu, where clicking the button toggles the visibility of the menu.
///
/// ```
/// use ui::prelude::*;
///
/// Button::new("button_id", "点击我!")
///     .start_icon(Icon::new(IconName::Check))
///     .toggle_state(true)
///     .on_click(|event, window, cx| {
///         // Handle click event
///     });
/// ```
///
/// To change the style of the button when it is selected use the [`selected_style`][Button::selected_style] method.
///
/// ```
/// use ui::prelude::*;
/// use ui::TintColor;
///
/// Button::new("button_id", "点击我!")
///     .toggle_state(true)
///     .selected_style(ButtonStyle::Tinted(TintColor::Accent))
///     .on_click(|event, window, cx| {
///         // Handle click event
///     });
/// ```
/// This will create a button with a blue tinted background when selected.
///
/// **A full-width button**, is typically used in scenarios such as the bottom of a modal or form, where it occupies the entire width of its container.
/// The button's content, including text and icons, is centered by default.
///
/// ```
/// use ui::prelude::*;
///
/// let button = Button::new("button_id", "点击我!")
///     .full_width()
///     .on_click(|event, window, cx| {
///         // Handle click event
///     });
/// ```
///
#[derive(IntoElement, Documented, RegisterComponent)]
pub struct Button {
    base: ButtonLike,
    label: SharedString,
    label_color: Option<Color>,
    label_size: Option<LabelSize>,
    selected_label: Option<SharedString>,
    selected_label_color: Option<Color>,
    start_icon: Option<Icon>,
    end_icon: Option<Icon>,
    key_binding: Option<KeyBinding>,
    key_binding_position: KeybindingPosition,
    alpha: Option<f32>,
    truncate: bool,
    loading: bool,
}

impl Button {
    /// Creates a new [`Button`] with a specified identifier and label.
    ///
    /// This is the primary constructor for a [`Button`] component. It initializes
    /// the button with the provided identifier and label text, setting all other
    /// properties to their default values, which can be customized using the
    /// builder pattern methods provided by this struct.
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            base: ButtonLike::new(id),
            label: label.into(),
            label_color: None,
            label_size: None,
            selected_label: None,
            selected_label_color: None,
            start_icon: None,
            end_icon: None,
            key_binding: None,
            key_binding_position: KeybindingPosition::default(),
            alpha: None,
            truncate: false,
            loading: false,
        }
    }

    /// Sets the color of the button's label.
    pub fn color(mut self, label_color: impl Into<Option<Color>>) -> Self {
        self.label_color = label_color.into();
        self
    }

    /// Defines the size of the button's label.
    pub fn label_size(mut self, label_size: impl Into<Option<LabelSize>>) -> Self {
        self.label_size = label_size.into();
        self
    }

    /// Sets the label used when the button is in a selected state.
    pub fn selected_label<L: Into<SharedString>>(mut self, label: impl Into<Option<L>>) -> Self {
        self.selected_label = label.into().map(Into::into);
        self
    }

    /// Sets the label color used when the button is in a selected state.
    pub fn selected_label_color(mut self, color: impl Into<Option<Color>>) -> Self {
        self.selected_label_color = color.into();
        self
    }

    /// Sets an icon to display at the start (left) of the button label.
    ///
    /// The icon's color will be overridden to `Color::Disabled` when the button is disabled.
    pub fn start_icon(mut self, icon: impl Into<Option<Icon>>) -> Self {
        self.start_icon = icon.into();
        self
    }

    /// Sets an icon to display at the end (right) of the button label.
    ///
    /// The icon's color will be overridden to `Color::Disabled` when the button is disabled.
    pub fn end_icon(mut self, icon: impl Into<Option<Icon>>) -> Self {
        self.end_icon = icon.into();
        self
    }

    /// Display the keybinding that triggers the button action.
    pub fn key_binding(mut self, key_binding: impl Into<Option<KeyBinding>>) -> Self {
        self.key_binding = key_binding.into();
        self
    }

    /// Sets the position of the keybinding relative to the button label.
    ///
    /// This method allows you to specify where the keybinding should be displayed
    /// in relation to the button's label.
    pub fn key_binding_position(mut self, position: KeybindingPosition) -> Self {
        self.key_binding_position = position;
        self
    }

    /// Sets the alpha property of the color of label.
    pub fn alpha(mut self, alpha: f32) -> Self {
        self.alpha = Some(alpha);
        self
    }

    /// Truncates overflowing labels with an ellipsis (`…`) if needed.
    ///
    /// Buttons with static labels should _never_ be truncated, ensure
    /// this is only used when the label is dynamic and may overflow.
    pub fn truncate(mut self, truncate: bool) -> Self {
        self.truncate = truncate;
        self
    }

    /// Displays a rotating loading spinner in place of the `start_icon`.
    ///
    /// When `loading` is `true`, any `start_icon` is ignored. and a rotating
    /// loading spinner is shown instead.
    pub fn loading(mut self, loading: bool) -> Self {
        self.loading = loading;
        self
    }
}

impl Toggleable for Button {
    /// Sets the selected state of the button.
    ///
    /// # Examples
    ///
    /// Create a toggleable button that changes appearance when selected:
    ///
    /// ```
    /// use ui::prelude::*;
    /// use ui::TintColor;
    ///
    /// let selected = true;
    ///
    /// Button::new("toggle_button", "切换我")
    ///     .start_icon(Icon::new(IconName::Check))
    ///     .toggle_state(selected)
    ///     .selected_style(ButtonStyle::Tinted(TintColor::Accent))
    ///     .on_click(|event, window, cx| {
    ///         // Toggle the selected state
    ///     });
    /// ```
    fn toggle_state(mut self, selected: bool) -> Self {
        self.base = self.base.toggle_state(selected);
        self
    }
}

impl SelectableButton for Button {
    /// Sets the style for the button in a selected state.
    ///
    /// # Examples
    ///
    /// Customize the selected appearance of a button:
    ///
    /// ```
    /// use ui::prelude::*;
    /// use ui::TintColor;
    ///
    /// Button::new("styled_button", "样式化按钮")
    ///     .toggle_state(true)
    ///     .selected_style(ButtonStyle::Tinted(TintColor::Accent));
    /// ```
    fn selected_style(mut self, style: ButtonStyle) -> Self {
        self.base = self.base.selected_style(style);
        self
    }
}

impl Disableable for Button {
    /// Disables the button, preventing interaction and changing its appearance.
    ///
    /// When disabled, the button's icon and label will use `Color::Disabled`.
    ///
    /// # Examples
    ///
    /// Create a disabled button:
    ///
    /// ```
    /// use ui::prelude::*;
    ///
    /// Button::new("disabled_button", "无法点击我")
    ///     .disabled(true);
    /// ```
    fn disabled(mut self, disabled: bool) -> Self {
        self.base = self.base.disabled(disabled);
        self
    }
}

impl Clickable for Button {
    fn on_click(
        mut self,
        handler: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.base = self.base.on_click(handler);
        self
    }

    fn cursor_style(mut self, cursor_style: gpui::CursorStyle) -> Self {
        self.base = self.base.cursor_style(cursor_style);
        self
    }
}

impl FixedWidth for Button {
    /// Sets a fixed width for the button.
    ///
    /// # Examples
    ///
    /// Create a button with a fixed width of 100 pixels:
    ///
    /// ```
    /// use ui::prelude::*;
    ///
    /// Button::new("fixed_width_button", "固定宽度")
    ///     .width(px(100.0));
    /// ```
    fn width(mut self, width: impl Into<DefiniteLength>) -> Self {
        self.base = self.base.width(width);
        self
    }

    /// Makes the button take up the full width of its container.
    ///
    /// # Examples
    ///
    /// Create a button that takes up the full width of its container:
    ///
    /// ```
    /// use ui::prelude::*;
    ///
    /// Button::new("full_width_button", "全宽")
    ///     .full_width();
    /// ```
    fn full_width(mut self) -> Self {
        self.base = self.base.full_width();
        self
    }
}

impl ButtonCommon for Button {
    fn id(&self) -> &ElementId {
        self.base.id()
    }

    /// Sets the visual style of the button.
    fn style(mut self, style: ButtonStyle) -> Self {
        self.base = self.base.style(style);
        self
    }

    /// Sets the size of the button.
    fn size(mut self, size: ButtonSize) -> Self {
        self.base = self.base.size(size);
        self
    }

    /// Sets a tooltip that appears on hover.
    ///
    /// # Examples
    ///
    /// Add a tooltip to a button:
    ///
    /// ```
    /// use ui::{Tooltip, prelude::*};
    ///
    /// Button::new("tooltip_button", "悬停在我上")
    ///     .tooltip(Tooltip::text("这是一个工具提示"));
    /// ```
    fn tooltip(mut self, tooltip: impl Fn(&mut Window, &mut App) -> AnyView + 'static) -> Self {
        self.base = self.base.tooltip(tooltip);
        self
    }

    fn tab_index(mut self, tab_index: impl Into<isize>) -> Self {
        self.base = self.base.tab_index(tab_index);
        self
    }

    fn layer(mut self, elevation: ElevationIndex) -> Self {
        self.base = self.base.layer(elevation);
        self
    }

    fn track_focus(mut self, focus_handle: &gpui::FocusHandle) -> Self {
        self.base = self.base.track_focus(focus_handle);
        self
    }
}

impl RenderOnce for Button {
    #[allow(refining_impl_trait)]
    fn render(self, _window: &mut Window, cx: &mut App) -> ButtonLike {
        let is_disabled = self.base.disabled;
        let is_selected = self.base.selected;

        let label = self
            .selected_label
            .filter(|_| is_selected)
            .unwrap_or(self.label);

        let label_color = if is_disabled {
            Color::Disabled
        } else if is_selected {
            self.selected_label_color.unwrap_or(Color::Selected)
        } else {
            self.label_color.unwrap_or_default()
        };

        self.base.child(
            h_flex()
                .when(self.truncate, |this| this.min_w_0().overflow_hidden())
                .gap(DynamicSpacing::Base04.rems(cx))
                .when_else(
                    self.loading,
                    |this| {
                        this.child(
                            Icon::new(IconName::LoadCircle)
                                .size(IconSize::Small)
                                .color(Color::Muted)
                                .with_rotate_animation(2),
                        )
                    },
                    |this| {
                        this.when_some(self.start_icon, |this, icon| {
                            this.child(if is_disabled {
                                icon.color(Color::Disabled)
                            } else {
                                icon
                            })
                        })
                    },
                )
                .child(
                    h_flex()
                        .when(self.truncate, |this| this.min_w_0().overflow_hidden())
                        .when(
                            self.key_binding_position == KeybindingPosition::Start,
                            |this| this.flex_row_reverse(),
                        )
                        .gap(DynamicSpacing::Base06.rems(cx))
                        .justify_between()
                        .child(
                            Label::new(label)
                                .color(label_color)
                                .size(self.label_size.unwrap_or_default())
                                .when_some(self.alpha, |this, alpha| this.alpha(alpha))
                                .when(self.truncate, |this| this.truncate()),
                        )
                        .children(self.key_binding),
                )
                .when_some(self.end_icon, |this, icon| {
                    this.child(if is_disabled {
                        icon.color(Color::Disabled)
                    } else {
                        icon
                    })
                }),
        )
    }
}

impl Component for Button {
    fn scope() -> ComponentScope {
        ComponentScope::Input
    }

    fn sort_name() -> &'static str {
        "ButtonA"
    }

    fn description() -> Option<&'static str> {
        Some("按钮用于触发事件或操作。")
    }

    fn preview(_window: &mut Window, _cx: &mut App) -> Option<AnyElement> {
        Some(
            v_flex()
                .gap_6()
                .children(vec![
                    example_group_with_title(
                        "按钮样式",
                        vec![
                            single_example(
                                "默认",
                                Button::new("default", "默认").into_any_element(),
                            ),
                            single_example(
                                "填充",
                                Button::new("filled", "填充")
                                    .style(ButtonStyle::Filled)
                                    .into_any_element(),
                            ),
                            single_example(
                                "淡雅",
                                Button::new("outline", "淡雅")
                                    .style(ButtonStyle::Subtle)
                                    .into_any_element(),
                            ),
                            single_example(
                                "着色",
                                Button::new("tinted_accent_style", "强调")
                                    .style(ButtonStyle::Tinted(TintColor::Accent))
                                    .into_any_element(),
                            ),
                            single_example(
                                "透明",
                                Button::new("transparent", "透明")
                                    .style(ButtonStyle::Transparent)
                                    .into_any_element(),
                            ),
                        ],
                    ),
                    example_group_with_title(
                        "色调样式",
                        vec![
                            single_example(
                                "强调",
                                Button::new("tinted_accent", "强调")
                                    .style(ButtonStyle::Tinted(TintColor::Accent))
                                    .into_any_element(),
                            ),
                            single_example(
                                "错误",
                                Button::new("tinted_negative", "错误")
                                    .style(ButtonStyle::Tinted(TintColor::Error))
                                    .into_any_element(),
                            ),
                            single_example(
                                "警告",
                                Button::new("tinted_warning", "警告")
                                    .style(ButtonStyle::Tinted(TintColor::Warning))
                                    .into_any_element(),
                            ),
                            single_example(
                                "成功",
                                Button::new("tinted_positive", "成功")
                                    .style(ButtonStyle::Tinted(TintColor::Success))
                                    .into_any_element(),
                            ),
                        ],
                    ),
                    example_group_with_title(
                        "特殊状态",
                        vec![
                            single_example(
                                "默认",
                                Button::new("default_state", "默认").into_any_element(),
                            ),
                            single_example(
                                "已禁用",
                                Button::new("disabled", "已禁用")
                                    .disabled(true)
                                    .into_any_element(),
                            ),
                            single_example(
                                "已选中",
                                Button::new("selected", "已选中")
                                    .toggle_state(true)
                                    .into_any_element(),
                            ),
                        ],
                    ),
                    example_group_with_title(
                        "带图标的按钮",
                        vec![
                            single_example(
                                "起始图标",
                                Button::new("icon_start", "起始图标")
                                    .start_icon(Icon::new(IconName::Check))
                                    .into_any_element(),
                            ),
                            single_example(
                                "结束图标",
                                Button::new("icon_end", "结束图标")
                                    .end_icon(Icon::new(IconName::Check))
                                    .into_any_element(),
                            ),
                            single_example(
                                "双图标",
                                Button::new("both_icons", "双图标")
                                    .start_icon(Icon::new(IconName::Check))
                                    .end_icon(Icon::new(IconName::ChevronDown))
                                    .into_any_element(),
                            ),
                            single_example(
                                "图标颜色",
                                Button::new("icon_color", "图标颜色")
                                    .start_icon(Icon::new(IconName::Check).color(Color::Accent))
                                    .into_any_element(),
                            ),
                        ],
                    ),
                ])
                .into_any_element(),
        )
    }
}
