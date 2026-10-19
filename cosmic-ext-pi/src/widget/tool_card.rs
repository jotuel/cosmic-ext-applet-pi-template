use crate::subscription::PiAction;
use cosmic::iced::Length;
use cosmic::widget;
use cosmic::Element;

/// Collapsible tool execution card widget.
pub struct ToolCard;

impl ToolCard {
    /// Builds a native libcosmic collapsible tool card element.
    #[allow(clippy::too_many_arguments)]
    pub fn view<'a>(
        tool_call_id: &'a str,
        tool_name: &'a str,
        args: &'a serde_json::Value,
        args_raw: &'a str,
        partial_result: Option<&'a serde_json::Value>,
        result: Option<&'a serde_json::Value>,
        is_error: bool,
        duration_ms: Option<f64>,
        collapsed: bool,
        is_executing: bool,
    ) -> Element<'a, PiAction> {
        let (status_icon, status_label) = if is_executing {
            ("⏳", "Running")
        } else if is_error {
            ("✖", "Failed")
        } else {
            ("✔", "Done")
        };

        let duration_text = duration_ms
            .map(|d| {
                if d >= 1000.0 {
                    format!(" ({:.2}s)", d / 1000.0)
                } else {
                    format!(" ({:.0}ms)", d)
                }
            })
            .unwrap_or_default();

        let arrow = if collapsed { "▶" } else { "▼" };
        let header_title =
            format!("{arrow} {status_icon} [{status_label}] {tool_name}{duration_text}");

        let toggle_btn = widget::button::text(header_title)
            .description(format!(
                "Toggle tool execution details for {tool_name}: {status_label}"
            ))
            .on_press(PiAction::ToggleTool(tool_call_id.to_string()));

        let mut col = widget::column::with_capacity(3).push(toggle_btn).spacing(6);

        if !collapsed {
            // Arguments display
            let args_formatted = if !args.is_null() {
                serde_json::to_string_pretty(args).unwrap_or_else(|_| args.to_string())
            } else if !args_raw.is_empty() {
                args_raw.to_string()
            } else {
                "{}".to_string()
            };

            let args_col = widget::column::with_capacity(2)
                .push(widget::text::caption("Arguments:"))
                .push(widget::text::monotext(args_formatted))
                .spacing(2);

            let args_container = widget::container(args_col)
                .padding([4, 8])
                .width(Length::Fill)
                .class(cosmic::theme::Container::Card);

            col = col.push(args_container);

            // Output / Result display
            let active_result = result.or(partial_result);
            if let Some(res) = active_result {
                let res_text = match res {
                    serde_json::Value::String(s) => s.clone(),
                    other => {
                        serde_json::to_string_pretty(other).unwrap_or_else(|_| other.to_string())
                    }
                };

                let output_label = if is_error { "Error:" } else { "Output:" };
                let output_col = widget::column::with_capacity(2)
                    .push(widget::text::caption(output_label))
                    .push(widget::scrollable(widget::text::monotext(res_text)))
                    .spacing(2);

                let output_container = widget::container(output_col)
                    .max_height(260.0)
                    .padding([4, 8])
                    .width(Length::Fill)
                    .class(if is_error {
                        cosmic::theme::Container::Primary
                    } else {
                        cosmic::theme::Container::Card
                    });

                col = col.push(output_container);
            }
        }

        let main_container = widget::container(col)
            .padding(6)
            .width(Length::Fill)
            .class(cosmic::theme::Container::Card);

        main_container.into()
    }
}
