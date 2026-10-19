use crate::subscription::PiAction;
use cosmic::iced::Length;
use cosmic::widget;
use cosmic::Element;

/// Collapsible thinking / reasoning accordion widget.
pub struct ThinkingBlock;

impl ThinkingBlock {
    /// Builds a native libcosmic collapsible thinking element.
    pub fn view<'a>(
        message_index: usize,
        block_index: usize,
        text: &'a str,
        collapsed: bool,
        is_streaming: bool,
    ) -> Element<'a, PiAction> {
        let arrow = if collapsed { "▶" } else { "▼" };
        let count_str = if is_streaming {
            " (thinking...)".to_string()
        } else {
            format!(" ({} chars)", text.len())
        };

        let header_label = format!("{arrow} 🧠 Reasoning{count_str}");

        let header_btn = widget::button::text(header_label)
            .description(if collapsed {
                "Expand reasoning block"
            } else {
                "Collapse reasoning block"
            })
            .on_press(PiAction::ToggleThinking {
                message_index,
                block_index,
            });

        let mut col = widget::column::with_capacity(2).push(header_btn).spacing(4);

        if !collapsed {
            let body_text = if text.is_empty() && is_streaming {
                "..."
            } else {
                text
            };

            let body_widget = widget::text::caption(body_text);
            let scrollable_body = widget::scrollable(body_widget);

            let body_container = widget::container(scrollable_body)
                .max_height(260.0)
                .padding([4, 8])
                .width(Length::Fill)
                .class(cosmic::theme::Container::Card);

            col = col.push(body_container);
        }

        let container = widget::container(col)
            .padding(4)
            .width(Length::Fill)
            .class(cosmic::theme::Container::Transparent);

        container.into()
    }
}
