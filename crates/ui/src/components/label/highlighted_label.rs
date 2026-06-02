use std::ops::Range;

use gpui::{FontWeight, HighlightStyle, StyleRefinement, StyledText};
use gpui_util::debug_panic;

use crate::{LabelCommon, LabelLike, LabelSize, LineHeightStyle, prelude::*};

#[derive(IntoElement, RegisterComponent)]
pub struct HighlightedLabel {
    base: LabelLike,
    label: SharedString,
    highlight_indices: Vec<usize>,
}

impl HighlightedLabel {
    /// Constructs a label with the given characters highlighted.
    /// Characters are identified by UTF-8 byte position.
    #[track_caller]
    pub fn new(label: impl Into<SharedString>, mut highlight_indices: Vec<usize>) -> Self {
        let label = label.into();

        if let Some(index) = highlight_indices
            .iter()
            .find(|&i| !label.is_char_boundary(*i))
        {
            let location = std::panic::Location::caller();
            debug_panic!(
                "高亮索引 {index} 不是有效的 UTF-8 边界(调用位置 {location})"
            );
            highlight_indices.clear();
        }

        Self {
            base: LabelLike::new(),
            label,
            highlight_indices,
        }
    }

    /// Constructs a label with the given byte ranges highlighted.
    /// Assumes that the highlight ranges are valid UTF-8 byte positions.
    pub fn from_ranges(
        label: impl Into<SharedString>,
        highlight_ranges: Vec<Range<usize>>,
    ) -> Self {
        let label = label.into();
        let highlight_indices = highlight_ranges
            .iter()
            .flat_map(|range| {
                let mut indices = Vec::new();
                let mut index = range.start;
                while index < range.end {
                    indices.push(index);
                    index += label[index..].chars().next().map_or(0, |c| c.len_utf8());
                }
                indices
            })
            .collect();

        Self {
            base: LabelLike::new(),
            label,
            highlight_indices,
        }
    }

    pub fn text(&self) -> &str {
        self.label.as_str()
    }

    pub fn highlight_indices(&self) -> &[usize] {
        &self.highlight_indices
    }
}

impl HighlightedLabel {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.base.style()
    }

    pub fn flex_1(mut self) -> Self {
        self.style().flex_grow = Some(1.);
        self.style().flex_shrink = Some(1.);
        self.style().flex_basis = Some(gpui::relative(0.).into());
        self
    }

    pub fn flex_none(mut self) -> Self {
        self.style().flex_grow = Some(0.);
        self.style().flex_shrink = Some(0.);
        self
    }

    pub fn flex_grow(mut self) -> Self {
        self.style().flex_grow = Some(1.);
        self
    }

    pub fn flex_shrink(mut self) -> Self {
        self.style().flex_shrink = Some(1.);
        self
    }

    pub fn flex_shrink_0(mut self) -> Self {
        self.style().flex_shrink = Some(0.);
        self
    }
}

impl LabelCommon for HighlightedLabel {
    fn size(mut self, size: LabelSize) -> Self {
        self.base = self.base.size(size);
        self
    }

    fn weight(mut self, weight: FontWeight) -> Self {
        self.base = self.base.weight(weight);
        self
    }

    fn line_height_style(mut self, line_height_style: LineHeightStyle) -> Self {
        self.base = self.base.line_height_style(line_height_style);
        self
    }

    fn color(mut self, color: Color) -> Self {
        self.base = self.base.color(color);
        self
    }

    fn strikethrough(mut self) -> Self {
        self.base = self.base.strikethrough();
        self
    }

    fn italic(mut self) -> Self {
        self.base = self.base.italic();
        self
    }

    fn alpha(mut self, alpha: f32) -> Self {
        self.base = self.base.alpha(alpha);
        self
    }

    fn underline(mut self) -> Self {
        self.base = self.base.underline();
        self
    }

    fn truncate(mut self) -> Self {
        self.base = self.base.truncate();
        self
    }

    fn single_line(mut self) -> Self {
        self.base = self.base.single_line();
        self
    }

    fn buffer_font(mut self, cx: &App) -> Self {
        self.base = self.base.buffer_font(cx);
        self
    }

    fn inline_code(mut self, cx: &App) -> Self {
        self.base = self.base.inline_code(cx);
        self
    }
}

pub fn highlight_ranges(
    text: &str,
    indices: &[usize],
    style: HighlightStyle,
) -> Vec<(Range<usize>, HighlightStyle)> {
    let mut highlight_indices = indices.iter().copied().peekable();
    let mut highlights: Vec<(Range<usize>, HighlightStyle)> = Vec::new();

    while let Some(start_ix) = highlight_indices.next() {
        let mut end_ix = start_ix;

        loop {
            end_ix += text[end_ix..].chars().next().map_or(0, |c| c.len_utf8());
            if highlight_indices.next_if(|&ix| ix == end_ix).is_none() {
                break;
            }
        }

        highlights.push((start_ix..end_ix, style));
    }

    highlights
}

impl RenderOnce for HighlightedLabel {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let highlight_color = cx.theme().colors().text_accent;

        let highlights = highlight_ranges(
            &self.label,
            &self.highlight_indices,
            HighlightStyle {
                color: Some(highlight_color),
                ..Default::default()
            },
        );

        let mut text_style = window.text_style();
        text_style.color = self.base.color.color(cx);

        self.base
            .child(StyledText::new(self.label).with_default_highlights(&text_style, highlights))
    }
}

impl Component for HighlightedLabel {
    fn scope() -> ComponentScope {
        ComponentScope::Typography
    }

    fn name() -> &'static str {
        "HighlightedLabel"
    }

    fn description() -> Option<&'static str> {
        Some("根据指定索引高亮字符的标签。")
    }

    fn preview(_window: &mut Window, _cx: &mut App) -> Option<AnyElement> {
        Some(
            v_flex()
                .gap_6()
                .children(vec![
                    example_group_with_title(
                        "基本用法",
                        vec![
                            single_example(
                                "默认",
                                HighlightedLabel::new("高亮文本", vec![0, 1, 2, 3]).into_any_element(),
                            ),
                            single_example(
                                "自定义颜色",
                                HighlightedLabel::new("彩色高亮", vec![0, 1, 7, 8, 9])
                                    .color(Color::Accent)
                                    .into_any_element(),
                            ),
                        ],
                    ),
                    example_group_with_title(
                        "样式",
                        vec![
                            single_example(
                                "粗体",
                                HighlightedLabel::new("粗体高亮", vec![0, 1, 2, 3])
                                    .weight(FontWeight::BOLD)
                                    .into_any_element(),
                            ),
                            single_example(
                                "斜体",
                                HighlightedLabel::new("斜体高亮", vec![0, 1, 6, 7, 8])
                                    .italic()
                                    .into_any_element(),
                            ),
                            single_example(
                                "下划线",
                                HighlightedLabel::new("下划线高亮", vec![0, 1, 10, 11, 12])
                                    .underline()
                                    .into_any_element(),
                            ),
                        ],
                    ),
                    example_group_with_title(
                        "尺寸",
                        vec![
                            single_example(
                                "小",
                                HighlightedLabel::new("小号高亮", vec![0, 1, 5, 6, 7])
                                    .size(LabelSize::Small)
                                    .into_any_element(),
                            ),
                            single_example(
                                "大",
                                HighlightedLabel::new("大号高亮", vec![0, 1, 5, 6, 7])
                                    .size(LabelSize::Large)
                                    .into_any_element(),
                            ),
                        ],
                    ),
                    example_group_with_title(
                        "特殊情况",
                        vec![
                            single_example(
                                "单行",
                                HighlightedLabel::new("单行高亮\n带换行符", vec![0, 1, 7, 8, 9])
                                    .single_line()
                                    .into_any_element(),
                            ),
                            single_example(
                                "截断",
                                HighlightedLabel::new("这是一段带有高亮效果的非常长的文本,应该被截断", vec![0, 1, 2, 3, 4, 5])
                                    .truncate()
                                    .into_any_element(),
                            ),
                        ],
                    ),
                ])
                .into_any_element()
        )
    }
}
