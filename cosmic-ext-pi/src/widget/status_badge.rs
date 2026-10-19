use crate::controller::AgentStatus;
use crate::subscription::PiAction;
use cosmic::widget;
use cosmic::Element;

/// Status indicator badge displaying agent status with full accessibility.
pub struct StatusBadge;

impl StatusBadge {
    /// Builds a native libcosmic status badge element.
    pub fn view<'a>(status: &'a AgentStatus) -> Element<'a, PiAction> {
        let (dot, label, is_active) = match status {
            AgentStatus::Idle => ("●", "Ready", false),
            AgentStatus::Thinking => ("◐", "Thinking...", true),
            AgentStatus::ExecutingTool { name, .. } => ("⚙", name.as_str(), true),
            AgentStatus::AwaitingUser { .. } => ("⚠", "Awaiting confirmation", true),
            AgentStatus::Error(err) => ("✖", err.as_str(), false),
        };

        let badge_text = match status {
            AgentStatus::ExecutingTool { name, .. } => format!("⚙ Running {name}"),
            _ => format!("{dot} {label}"),
        };

        let text_widget = widget::text::caption(badge_text);

        let content = widget::row::with_capacity(1)
            .push(text_widget)
            .spacing(6)
            .align_y(cosmic::iced::Alignment::Center);

        let container = widget::container(content)
            .padding([4, 8])
            .class(if is_active {
                cosmic::theme::Container::Primary
            } else {
                cosmic::theme::Container::Card
            });

        container.into()
    }
}
