use crate::controller::{ChatMessage, MessageBlock};
use crate::subscription::PiAction;
use crate::widget::thinking::ThinkingBlock;
use crate::widget::tool_card::ToolCard;
use cosmic::iced::Length;
use cosmic::widget;
use cosmic::Element;

/// Streaming chat view widget for conversational interactions.
pub struct ChatView;

impl ChatView {
    /// Builds a native libcosmic chat view from message history with default chronological order.
    pub fn view<'a>(history: &'a [ChatMessage]) -> Element<'a, PiAction> {
        Self::view_ordered(history, false)
    }

    /// Builds a native libcosmic chat view from message history, optionally reversing
    /// message order so newest messages appear at the top.
    pub fn view_ordered<'a>(history: &'a [ChatMessage], newest_first: bool) -> Element<'a, PiAction> {
        let mut msg_column = widget::column::with_capacity(history.len()).spacing(16);

        if newest_first {
            for (msg_idx, msg) in history.iter().enumerate().rev() {
                let rendered_msg = Self::render_message(msg_idx, msg);
                msg_column = msg_column.push(rendered_msg);
            }
        } else {
            for (msg_idx, msg) in history.iter().enumerate() {
                let rendered_msg = Self::render_message(msg_idx, msg);
                msg_column = msg_column.push(rendered_msg);
            }
        }

        let scrollable_chat = widget::scrollable(msg_column)
            .height(Length::Fill)
            .width(Length::Fill);

        let container = widget::container(scrollable_chat)
            .padding(12)
            .height(Length::Fill)
            .width(Length::Fill);

        container.into()
    }

    fn render_message<'a>(msg_idx: usize, msg: &'a ChatMessage) -> Element<'a, PiAction> {
        match msg.role.as_str() {
            "user" => Self::render_user_message(msg),
            "assistant" => {
                if msg.blocks.is_empty() && !msg.is_streaming {
                    widget::column::with_capacity(0).into()
                } else {
                    Self::render_assistant_message(msg_idx, msg)
                }
            }
            _ => Self::render_system_message(msg),
        }
    }

    fn render_user_message<'a>(msg: &'a ChatMessage) -> Element<'a, PiAction> {
        let mut content_col = widget::column::with_capacity(2).spacing(4);
        content_col = content_col.push(widget::text::caption_heading("You"));

        for block in &msg.blocks {
            if let MessageBlock::Text { text, .. } = block {
                content_col = content_col.push(widget::text::body(text.as_str()));
            }
        }

        let bubble = widget::container(content_col)
            .padding([8, 12])
            .class(cosmic::theme::Container::Primary);

        let row = widget::row::with_capacity(2)
            .push(widget::space::horizontal())
            .push(bubble)
            .width(Length::Fill);

        row.into()
    }

    fn render_assistant_message<'a>(msg_idx: usize, msg: &'a ChatMessage) -> Element<'a, PiAction> {
        let mut content_col = widget::column::with_capacity(msg.blocks.len() + 2).spacing(8);
        content_col = content_col.push(widget::text::caption_heading("Pi Assistant"));

        for (blk_idx, block) in msg.blocks.iter().enumerate() {
            match block {
                MessageBlock::Thinking {
                    text,
                    collapsed,
                    is_streaming,
                } => {
                    let thinking_el = ThinkingBlock::view(
                        msg_idx,
                        blk_idx,
                        text.as_str(),
                        *collapsed,
                        *is_streaming,
                    );
                    content_col = content_col.push(thinking_el);
                }
                MessageBlock::ToolCall(tool) => {
                    let tool_el = ToolCard::view(
                        tool.id.as_str(),
                        tool.name.as_str(),
                        &tool.args,
                        tool.args_raw.as_str(),
                        tool.partial_result.as_ref(),
                        tool.result.as_ref(),
                        tool.is_error,
                        tool.duration_ms,
                        tool.collapsed,
                        tool.is_executing,
                    );
                    content_col = content_col.push(tool_el);
                }
                MessageBlock::Text { text, parsed } => {
                    if !parsed.is_empty() {
                        let md = cosmic::iced::widget::markdown::view(
                            parsed,
                            cosmic::iced::widget::markdown::Settings::with_style(
                                cosmic::iced::theme::Palette::DARK,
                            ),
                        )
                        .map(|_| PiAction::Noop);
                        content_col = content_col.push(md);
                    } else if !text.is_empty() {
                        content_col = content_col.push(widget::text::body(text.as_str()));
                    }
                }
            }
        }

        if msg.is_streaming {
            content_col = content_col.push(widget::text::caption("●"));
        }

        let container = widget::container(content_col)
            .padding([8, 12])
            .width(Length::Fill)
            .class(cosmic::theme::Container::Card);

        container.into()
    }

    fn render_system_message<'a>(msg: &'a ChatMessage) -> Element<'a, PiAction> {
        let mut content_col = widget::column::with_capacity(msg.blocks.len() + 1).spacing(4);
        content_col = content_col.push(widget::text::caption_heading("System"));

        for block in &msg.blocks {
            if let MessageBlock::Text { text, parsed } = block {
                if !parsed.is_empty() {
                    let md = cosmic::iced::widget::markdown::view(
                        parsed,
                        cosmic::iced::widget::markdown::Settings::with_style(
                            cosmic::iced::theme::Palette::DARK,
                        ),
                    )
                    .map(|_| PiAction::Noop);
                    content_col = content_col.push(md);
                } else if !text.is_empty() {
                    content_col = content_col.push(widget::text::body(text.as_str()));
                }
            }
        }

        let container = widget::container(content_col)
            .padding([8, 12])
            .width(Length::Fill)
            .class(cosmic::theme::Container::Card);

        container.into()
    }
}
