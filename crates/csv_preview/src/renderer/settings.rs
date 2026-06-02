use ui::{
    ActiveTheme as _, AnyElement, ButtonSize, Context, ContextMenu, DropdownMenu, ElementId,
    IntoElement as _, ParentElement as _, Styled as _, Tooltip, Window, div, h_flex,
};

use crate::{CsvPreviewView, settings::VerticalAlignment};

///// Settings related /////
impl CsvPreviewView {
    /// Render settings panel above the table
    pub(crate) fn render_settings_panel(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let current_alignment_text = match self.settings.vertical_alignment {
            VerticalAlignment::Top => "顶部",
            VerticalAlignment::Center => "居中",
        };

        let view = cx.entity();
        let alignment_dropdown_menu = ContextMenu::build(window, cx, |menu, _window, _cx| {
            menu.entry("顶部", None, {
                let view = view.clone();
                move |_window, cx| {
                    view.update(cx, |this, cx| {
                        this.settings.vertical_alignment = VerticalAlignment::Top;
                        cx.notify();
                    });
                }
            })
            .entry("居中", None, {
                let view = view.clone();
                move |_window, cx| {
                    view.update(cx, |this, cx| {
                        this.settings.vertical_alignment = VerticalAlignment::Center;
                        cx.notify();
                    });
                }
            })
        });

        let panel = h_flex()
            .gap_4()
            .p_2()
            .bg(cx.theme().colors().surface_background)
            .border_b_1()
            .border_color(cx.theme().colors().border)
            .flex_wrap()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().colors().text_muted)
                            .child("文本对齐:"),
                    )
                    .child(
                        DropdownMenu::new(
                            ElementId::Name("vertical-alignment-dropdown".into()),
                            current_alignment_text,
                            alignment_dropdown_menu,
                        )
                        .trigger_size(ButtonSize::Compact)
                        .trigger_tooltip(Tooltip::text(
                            "选择单元格内文本的垂直对齐方式",
                        )),
                    ),
            );

        #[cfg(feature = "dev-tools")]
        let panel = panel.child(
            h_flex()
                .gap_2()
                .items_center()
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().colors().text_muted)
                        .child("仅开发:"),
                )
                .child(create_dev_only_popover_menu(cx)),
        );

        panel.into_any_element()
    }
}

#[cfg(feature = "dev-tools")]
fn create_dev_only_popover_menu(
    cx: &mut Context<'_, CsvPreviewView>,
) -> ui::PopoverMenu<ContextMenu> {
    use crate::settings::RowRenderMechanism;
    use ui::{IconButton, IconName, IconPosition, IconSize, PopoverMenu};

    PopoverMenu::new("debug-options-menu")
        .trigger_with_tooltip(
            IconButton::new("debug-options-trigger", IconName::Settings).icon_size(IconSize::Small),
            Tooltip::text(
                "仅用于调试的部分。将在 CSV 功能公开发布时移除"
            ),
        )
        .menu({
            let view_entity = cx.entity();
            move |window, cx| {
                let view = view_entity.read(cx);
                let settings = view.settings.clone();
                Some(ContextMenu::build(window, cx, |menu, _, _| {
                    menu.header("渲染模式")
                        .toggleable_entry(
                            "可变高度",
                            settings.rendering_with == RowRenderMechanism::VariableList,
                            IconPosition::Start,
                            None,
                            {
                                let view_entity = view_entity.clone();
                                move |_w, cx| {
                                    view_entity.update(cx, |view, cx| {
                                        view.settings.rendering_with =
                                            RowRenderMechanism::VariableList;
                                        view.settings.multiline_cells_enabled = true;
                                        cx.notify();
                                    })
                                }
                            },
                        )
                        .toggleable_entry(
                            "统一高度",
                            settings.rendering_with == RowRenderMechanism::UniformList,
                            IconPosition::Start,
                            None,
                            {
                                let view_entity = view_entity.clone();
                                move |_w, cx| {
                                    view_entity.update(cx, |view, cx| {
                                        view.settings.rendering_with =
                                            RowRenderMechanism::UniformList;
                                        view.settings.multiline_cells_enabled = false;
                                        cx.notify();
                                    })
                                }
                            },
                        )
                        .separator()
                        .toggleable_entry(
                            "显示性能指标",
                            settings.show_perf_metrics_overlay,
                            IconPosition::Start,
                            None,
                            {
                                let view_entity = view_entity.clone();
                                move |_w, cx| {
                                    view_entity.update(cx, |view, cx| {
                                        view.settings.show_perf_metrics_overlay =
                                            !view.settings.show_perf_metrics_overlay;
                                        cx.notify();
                                    })
                                }
                            },
                        )
                        .toggleable_entry(
                            "显示单元格位置",
                            settings.show_debug_info,
                            IconPosition::Start,
                            None,
                            {
                                let view_entity = view_entity.clone();
                                move |_, cx| {
                                    view_entity.update(cx, |view, cx| {
                                        view.settings.show_debug_info =
                                            !view.settings.show_debug_info;
                                        cx.notify();
                                    })
                                }
                            },
                        )
                }))
            }
        })
}
