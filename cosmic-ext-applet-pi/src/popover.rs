// SPDX-License-Identifier: EUPL-1.2

use cosmic::Element;
use cosmic::iced::Length;
use cosmic::widget;
use cosmic_ext_pi::PiSessionController;
use cosmic_ext_pi::controller::AgentStatus;
use pi_rpc::protocol::ThinkingLevel;

use crate::app::Message;
use crate::config::Config;
use crate::fl;

/// Embedded vector icon assets for branding and popover starting page.
pub const PI_LOGO_COLOR_SVG: &[u8] = include_bytes!("../resources/icons/pi-logo-color.svg");
pub const PLUS_SYMBOLIC_SVG: &[u8] = include_bytes!("../resources/icons/plus-symbolic.svg");
pub const COSMIC_SYMBOLIC_SVG: &[u8] = include_bytes!("../resources/icons/cosmic-symbolic.svg");

/// Renders the small Card container shown on the starting page:
/// colored Pi icon + plus icon + official COSMIC computer logo (white or black depending on background).
pub fn view_starting_card<'a>() -> Element<'a, Message> {
    let pi_icon = widget::icon::icon(widget::icon::from_svg_bytes(PI_LOGO_COLOR_SVG)).size(64);

    let plus_icon =
        widget::icon::icon(widget::icon::from_svg_bytes(PLUS_SYMBOLIC_SVG).symbolic(true)).size(32);

    let cosmic_icon =
        widget::icon::icon(widget::icon::from_svg_bytes(COSMIC_SYMBOLIC_SVG).symbolic(true))
            .size(64);

    let card_row = widget::row::with_capacity(3)
        .push(pi_icon)
        .push(plus_icon)
        .push(cosmic_icon)
        .spacing(12)
        .align_y(cosmic::iced::Alignment::Center);

    widget::container(card_row)
        .class(cosmic::theme::Container::Card)
        .padding([8, 16])
        .into()
}

/// Fixed popover dimensions to ensure persistent layout and prevent reaching screen edges.
pub const POPOVER_WIDTH: f32 = 420.0;
pub const POPOVER_HEIGHT: f32 = 580.0;

/// Builds the complete popover window content.
///
/// Ensures the bottom section (two dividers, prompt input, status, send/steer/abort,
/// and model/context/thinking footer) is persistently anchored at the bottom,
/// the body area scrolls in between, and the popover has a fixed bounding size
/// so it never reaches the bottom of the screen.
pub fn view_popover<'a>(
    config: &'a Config,
    controller: Option<&'a PiSessionController>,
    is_spawning: bool,
    spawn_error: Option<&'a str>,
    prompt_input: &'a str,
    current_model: Option<&'a str>,
    show_settings: bool,
) -> Element<'a, Message> {
    let header = view_header(config, show_settings);
    let divider1 = widget::divider::horizontal::default();

    let content = if show_settings {
        let settings_body = widget::container(view_settings(config))
            .width(Length::Fill)
            .height(Length::Fill);

        widget::column::with_capacity(3)
            .push(header)
            .push(divider1)
            .push(settings_body)
            .spacing(8)
            .padding([8, 12])
            .width(Length::Fill)
            .height(Length::Fill)
    } else {
        let body = widget::container(view_body(controller, is_spawning, spawn_error, config))
            .width(Length::Fill)
            .height(Length::Fill);
        let bottom_section = view_bottom_section(prompt_input, controller, config, current_model);

        widget::column::with_capacity(4)
            .push(header)
            .push(divider1)
            .push(body)
            .push(bottom_section)
            .spacing(6)
            .padding([8, 12])
            .width(Length::Fill)
            .height(Length::Fill)
    };

    widget::container(content)
        .width(Length::Fixed(POPOVER_WIDTH))
        .height(Length::Fixed(POPOVER_HEIGHT))
        .into()
}

/// Renders the persistent bottom section: divider, prompt input & status action row,
/// divider, and model/context/thinking footer.
fn view_bottom_section<'a>(
    prompt_input: &'a str,
    controller: Option<&'a PiSessionController>,
    config: &'a Config,
    current_model: Option<&'a str>,
) -> Element<'a, Message> {
    let divider2 = widget::divider::horizontal::default();
    let input_bar = view_input_bar(prompt_input, controller, config);
    let divider3 = widget::divider::horizontal::default();
    let footer = view_footer(config, controller, current_model);

    widget::column::with_capacity(4)
        .push(divider2)
        .push(input_bar)
        .push(divider3)
        .push(footer)
        .spacing(6)
        .width(Length::Fill)
        .into()
}

/// Renders the popover top header bar with project selector, browse and recent icons,
/// and session controls (new session, settings).
fn view_header<'a>(config: &'a Config, show_settings: bool) -> Element<'a, Message> {
    if show_settings {
        let back_btn = widget::button::icon(widget::icon::from_name("go-previous-symbolic"))
            .class(cosmic::theme::Button::Icon)
            .tooltip(fl!("back"))
            .on_press(Message::ToggleSettings);

        let title = widget::text::title4(fl!("settings"));

        let settings_btn =
            widget::button::icon(widget::icon::from_name("preferences-system-symbolic"))
                .class(cosmic::theme::Button::Icon)
                .tooltip(fl!("close"))
                .on_press(Message::ToggleSettings);

        return widget::row::with_capacity(4)
            .push(back_btn)
            .push(title)
            .push(widget::space::horizontal())
            .push(settings_btn)
            .spacing(8)
            .align_y(cosmic::iced::Alignment::Center)
            .width(Length::Fill)
            .into();
    }

    let effective_cwd = config.effective_working_dir();
    let full_path = effective_cwd.display().to_string();
    let folder_name = effective_cwd
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(".")
        .to_string();

    let display_folder = if folder_name.chars().count() > 22 {
        let truncated: String = folder_name.chars().take(20).collect();
        format!("{truncated}…")
    } else {
        folder_name
    };

    let folder_icon = widget::icon::from_name("folder").size(16);
    let folder_text = widget::text::title4(display_folder);

    let project_label = widget::tooltip(
        widget::row::with_capacity(2)
            .push(folder_icon)
            .push(folder_text)
            .spacing(6)
            .align_y(cosmic::iced::Alignment::Center),
        widget::text(full_path).class(cosmic::theme::Text::Accent),
        widget::tooltip::Position::Bottom,
    );

    let browse_btn = widget::button::icon(widget::icon::from_name("folder-open-symbolic"))
        .tooltip(fl!("browse"))
        .class(cosmic::theme::Button::Icon)
        .on_press(Message::PickProjectDirectory);

    let recent_but = if let Some(recent) = config
        .recent_projects
        .iter()
        .find(|p| p.is_dir() && p.as_path() != effective_cwd.as_path())
    {
        let name = recent
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("recent");
        Some(
            widget::button::icon(widget::icon::from_name("folder-recent"))
                .tooltip(format!("{}: {name}", fl!("recent-projects")))
                .on_press(Message::SelectRecentProject(recent.clone())),
        )
    } else {
        None
    };

    let mut header_row = widget::row::with_capacity(8)
        .push(project_label)
        .push(browse_btn)
        .push_maybe(recent_but);

    let settings_btn = widget::button::icon(widget::icon::from_name("preferences-system-symbolic"))
        .tooltip(fl!("settings"))
        .on_press(Message::ToggleSettings);

    header_row = header_row
        .push(widget::space::horizontal())
        .push(settings_btn)
        .spacing(6)
        .align_y(cosmic::iced::Alignment::Center)
        .width(Length::Fill);

    header_row.into()
}

/// Renders the settings view inside the popover.
pub fn view_settings<'a>(config: &'a Config) -> Element<'a, Message> {
    let default_dir = config.effective_default_folder();
    let default_dir_str = default_dir.display().to_string();

    let choose_btn =
        widget::button::text(fl!("choose-folder")).on_press(Message::PickDefaultFolder);

    let reset_btn = widget::button::icon(widget::icon::from_name("edit-undo-symbolic"))
        .tooltip(fl!("reset-to-home"))
        .on_press(Message::ResetDefaultFolder);

    let folder_buttons = widget::row::with_capacity(2)
        .push(choose_btn)
        .push(reset_btn)
        .spacing(6)
        .align_y(cosmic::iced::Alignment::Center);

    let folder_item = widget::settings::item::builder(fl!("default-folder"))
        .description(default_dir_str)
        .control(folder_buttons);

    let workspace_section = widget::settings::section()
        .title(fl!("project"))
        .add(folder_item);

    let chat_section = widget::settings::section()
        .title(fl!("chat"))
        .add(widget::settings::item(
            fl!("newest-first"),
            widget::toggler(config.newest_first).on_toggle(Message::ToggleNewestFirst),
        ));

    let restart_btn = widget::settings::item(
        fl!("restart"),
        widget::button::destructive("")
            .leading_icon(widget::icon::from_name("view-refresh-symbolic"))
            .on_press(Message::RestartPi),
    );

    let daemon_section = widget::settings::section()
        .title(fl!("daemon"))
        .add(widget::settings::item(
            fl!("autostart-daemon"),
            widget::toggler(config.autostart_daemon).on_toggle(Message::ToggleAutostart),
        ))
        .add(restart_btn);

    let notifications_section = widget::settings::section()
        .title(fl!("notifications"))
        .add(widget::settings::item(
            fl!("notify-complete"),
            widget::toggler(config.notify_on_complete).on_toggle(Message::ToggleNotifyComplete),
        ))
        .add(widget::settings::item(
            fl!("notify-approval"),
            widget::toggler(config.notify_on_approval).on_toggle(Message::ToggleNotifyApproval),
        ));

    let settings_col = widget::settings::view_column(vec![
        workspace_section.into(),
        chat_section.into(),
        daemon_section.into(),
        notifications_section.into(),
    ]);

    widget::scrollable(settings_col)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

/// Renders the central popover body: active dialog overlay, chat message history,
/// empty state, or connection/error banners.
fn view_body<'a>(
    controller: Option<&'a PiSessionController>,
    is_spawning: bool,
    spawn_error: Option<&'a str>,
    config: &'a Config,
) -> Element<'a, Message> {
    if let Some(err) = spawn_error {
        let err_box = widget::column::with_capacity(3)
            .push(widget::text::title3("Connection Error"))
            .push(
                widget::text::body(format!("Failed to connect to Pi:\n{err}")).width(Length::Fill),
            )
            .push(widget::button::suggested(fl!("restart")).on_press(Message::RestartPi))
            .spacing(12)
            .padding(16)
            .width(Length::Fill);

        let container = widget::container(err_box)
            .class(cosmic::theme::Container::Card)
            .center(Length::Fill);

        return container.into();
    }

    if is_spawning {
        let spawning_box = widget::column::with_capacity(2)
            .push(widget::text::title3("Connecting to Pi..."))
            .push(widget::text::caption(
                "Spawning background agent process in RPC mode",
            ))
            .spacing(8)
            .align_x(cosmic::iced::Alignment::Center);

        let container = widget::container(spawning_box).center(Length::Fill);
        return container.into();
    }

    let controller = match controller {
        Some(c) => c,
        None => {
            let start_box = widget::column::with_capacity(3)
                .push(widget::text::title3("Pi Agent Idle"))
                .push(widget::text::body(
                    "The background Pi process is not currently active.",
                ))
                .push(widget::button::suggested("Start Pi").on_press(Message::RestartPi))
                .spacing(12)
                .align_x(cosmic::iced::Alignment::Center);

            let container = widget::container(start_box).center(Length::Fill);
            return container.into();
        }
    };

    if let Some(dialog) = controller.view_active_dialog() {
        let dialog_mapped = dialog.map(Message::PiAction);
        let dialog_container = widget::container(dialog_mapped)
            .padding(12)
            .class(cosmic::theme::Container::Primary)
            .center(Length::Fill);

        return dialog_container.into();
    }

    if controller.history().is_empty() {
        let welcome_box = widget::column::with_capacity(3)
            .push(view_starting_card())
            .push(
                widget::text::body(fl!("no-messages"))
                    .class(cosmic::theme::Text::Accent)
                    .font(cosmic::font::light()),
            )
            .push(widget::text::caption(fl!("tip")))
            .spacing(12)
            .padding(16)
            .align_x(cosmic::iced::Alignment::Center);

        let container = widget::container(welcome_box).center(Length::Fill);
        return container.into();
    }

    controller
        .view_chat_ordered(config.newest_first)
        .map(Message::PiAction)
}

/// Renders the bottom input bar: prompt input field, Send, Steer, Follow-up, Abort icon buttons,
/// attach file button, and live status label.
fn view_input_bar<'a>(
    prompt_input: &'a str,
    controller: Option<&'a PiSessionController>,
    config: &'a Config,
) -> Element<'a, Message> {
    let status = controller.map(|c| c.status()).unwrap_or(&AgentStatus::Idle);
    let is_busy = status.is_busy();

    let input = widget::text_input(fl!("prompt-placeholder"), prompt_input)
        .on_input(Message::PromptInputChanged)
        .on_submit(move |_| {
            if is_busy {
                Message::SubmitSteer
            } else {
                Message::SubmitPrompt
            }
        })
        .width(Length::Fill);

    let (status_dot, status_str, full_status_str) = match status {
        AgentStatus::Idle => ("●", fl!("status-ready"), fl!("status-ready")),
        AgentStatus::Thinking => ("◐", fl!("status-thinking"), fl!("status-thinking")),
        AgentStatus::ExecutingTool { name, .. } => {
            let tool_display = if name.chars().count() > 14 {
                let s: String = name.chars().take(12).collect();
                format!("{s}…")
            } else {
                name.clone()
            };
            (
                "⚙",
                fl!("status-running-tool", tool = tool_display.as_str()),
                fl!("status-running-tool", tool = name.as_str()),
            )
        }
        AgentStatus::AwaitingUser { .. } => (
            "⚠",
            fl!("status-awaiting-user"),
            fl!("status-awaiting-user"),
        ),
        AgentStatus::Error(e) => {
            let err_display = if e.chars().count() > 16 {
                let s: String = e.chars().take(14).collect();
                format!("{s}…")
            } else {
                e.clone()
            };
            (
                "✖",
                fl!("status-error", error = err_display.as_str()),
                fl!("status-error", error = e.as_str()),
            )
        }
    };

    let caption_text = widget::text::caption(format!("{status_dot} {status_str}"))
        .class(cosmic::theme::Text::Accent);

    let status_caption: Element<'a, Message> = if status_str != full_status_str {
        widget::tooltip(
            caption_text,
            widget::text(full_status_str),
            widget::tooltip::Position::Top,
        )
        .into()
    } else {
        caption_text.into()
    };

    let attach_btn = widget::button::icon(widget::icon::from_name("mail-attachment-symbolic"))
        .tooltip(fl!("attach-file"))
        .class(cosmic::theme::Button::Image)
        .on_press(Message::PickFileAttachment);

    let mut action_row = widget::row::with_capacity(6)
        .push(status_caption)
        .push(widget::space::horizontal())
        .push(attach_btn)
        .spacing(6)
        .align_y(cosmic::iced::Alignment::Center);

    if is_busy {
        let mut steer_btn = widget::button::icon(widget::icon::from_name("go-next-symbolic"))
            .tooltip(fl!("steer"))
            .class(cosmic::theme::Button::Suggested);

        let mut follow_up_btn =
            widget::button::icon(widget::icon::from_name("media-skip-forward-symbolic"))
                .tooltip(fl!("follow-up"))
                .class(cosmic::theme::Button::Suggested);

        if !prompt_input.trim().is_empty() {
            steer_btn = steer_btn.on_press(Message::SubmitSteer);
            follow_up_btn = follow_up_btn.on_press(Message::SubmitFollowUp);
        }

        let abort_btn = widget::button::icon(widget::icon::from_name("process-stop-symbolic"))
            .tooltip(fl!("abort"))
            .class(cosmic::theme::Button::Destructive)
            .on_press(Message::Abort);

        action_row = action_row
            .push(steer_btn)
            .push(follow_up_btn)
            .push(abort_btn);
    } else {
        let mut send_btn = widget::button::icon(widget::icon::from_name("document-send-symbolic"))
            .tooltip(fl!("send"))
            .class(cosmic::theme::Button::Suggested);

        if !prompt_input.trim().is_empty() {
            send_btn = send_btn.on_press(Message::SubmitPrompt);
        }
        action_row = action_row.push(send_btn);
    }

    let mut input_column = widget::column::with_capacity(4)
        .push(input)
        .spacing(6)
        .width(Length::Fill);

    if let Some(ctrl) = controller {
        let steer_q = ctrl.steering_queue();
        let follow_q = ctrl.follow_up_queue();
        if !steer_q.is_empty() || !follow_q.is_empty() {
            let mut queue_col =
                widget::column::with_capacity(steer_q.len() + follow_q.len()).spacing(2);
            for item in steer_q {
                let display = if item.chars().count() > 40 {
                    let s: String = item.chars().take(38).collect();
                    format!("{s}…")
                } else {
                    item.clone()
                };
                queue_col = queue_col.push(
                    widget::text::caption(format!("↪ {}: {}", fl!("steer"), display))
                        .class(cosmic::theme::Text::Accent),
                );
            }
            for item in follow_q {
                let display = if item.chars().count() > 40 {
                    let s: String = item.chars().take(38).collect();
                    format!("{s}…")
                } else {
                    item.clone()
                };
                queue_col = queue_col.push(
                    widget::text::caption(format!("⏭ {}: {}", fl!("follow-up"), display))
                        .class(cosmic::theme::Text::Accent),
                );
            }
            input_column = input_column.push(queue_col);
        }
    }

    // Show suggestion chips when user types a slash command
    if prompt_input.starts_with('/') {
        let prefix = prompt_input.trim_start();
        let query = &prefix[1..].to_lowercase();

        let available = if let Some(c) = controller {
            c.available_commands()
        } else {
            vec![
                cosmic_ext_pi::SlashCommandInfo {
                    name: "help".to_string(),
                    description: "Show available commands".to_string(),
                    is_builtin: true,
                },
                cosmic_ext_pi::SlashCommandInfo {
                    name: "compact".to_string(),
                    description: "Compact session context".to_string(),
                    is_builtin: true,
                },
                cosmic_ext_pi::SlashCommandInfo {
                    name: "new".to_string(),
                    description: "Start a fresh session".to_string(),
                    is_builtin: true,
                },
            ]
        };

        let matches: Vec<_> = available
            .into_iter()
            .filter(|cmd| {
                if query.is_empty() {
                    true
                } else {
                    cmd.name.to_lowercase().contains(query)
                }
            })
            .take(6)
            .collect();

        if !matches.is_empty() {
            let mut suggestions = widget::row::with_capacity(matches.len()).spacing(4);
            for cmd in matches {
                let full_cmd = format!("/{}", cmd.name);
                let label = format!("/{}", cmd.name);
                let tooltip_text = if cmd.description.is_empty() {
                    label.clone()
                } else {
                    format!("{label} — {}", cmd.description)
                };
                let chip = widget::tooltip(
                    widget::button::text(label)
                        .class(cosmic::theme::Button::Text)
                        .on_press(Message::PromptInputChanged(format!("{full_cmd} "))),
                    widget::text(tooltip_text),
                    widget::tooltip::Position::Top,
                );
                suggestions = suggestions.push(chip);
            }
            let suggestions_scroll = widget::scrollable(suggestions).direction(
                cosmic::iced::widget::scrollable::Direction::Horizontal(
                    cosmic::iced::widget::scrollable::Scrollbar::default(),
                ),
            );
            input_column = input_column.push(suggestions_scroll);
        }
    } else if let Some((_before, query)) = crate::file_reference::extract_at_query(prompt_input) {
        let effective_cwd = config.effective_working_dir();
        let matches = crate::file_reference::find_matching_files(&effective_cwd, query, 6);

        if !matches.is_empty() {
            let mut suggestions = widget::row::with_capacity(matches.len()).spacing(4);
            for file_path in matches {
                let label = format!("@{file_path}");
                let next_input =
                    crate::file_reference::apply_at_completion(prompt_input, &file_path);
                let chip = widget::tooltip(
                    widget::button::text(label)
                        .class(cosmic::theme::Button::Text)
                        .on_press(Message::PromptInputChanged(next_input)),
                    widget::text(format!("{}: {file_path}", fl!("project"))),
                    widget::tooltip::Position::Top,
                );
                suggestions = suggestions.push(chip);
            }
            let suggestions_scroll = widget::scrollable(suggestions).direction(
                cosmic::iced::widget::scrollable::Direction::Horizontal(
                    cosmic::iced::widget::scrollable::Scrollbar::default(),
                ),
            );
            input_column = input_column.push(suggestions_scroll);
        }
    }

    input_column = input_column.push(action_row);

    input_column.into()
}

/// Helper to determine expected model context window length when not explicitly reported.
pub fn context_window_for_model(model: Option<&str>) -> u64 {
    match model {
        Some(m) if m.contains("claude") => 200_000,
        Some(m) if m.contains("gemini") => 1_000_000,
        Some(m) if m.contains("o1") || m.contains("o3") => 200_000,
        Some(m) if m.contains("gpt-4") => 128_000,
        Some(m) if m.contains("deepseek") => 64_000,
        _ => 128_000,
    }
}

/// Renders the popover bottom footer: current model selection, context window circular progress,
/// and thinking level.
/// Uses a thinner font (Weight::Light) than the Ready status indicator, with tooltips
/// and interactive buttons to cycle selections.
fn view_footer<'a>(
    config: &'a Config,
    controller: Option<&'a PiSessionController>,
    current_model: Option<&'a str>,
) -> Element<'a, Message> {
    // Current model display
    let model_raw = current_model
        .or_else(|| controller.and_then(|c| c.model()))
        .or(config.default_model.as_deref())
        .unwrap_or("default");

    // Extract short name for display to prevent overflowing (e.g. "anthropic/claude-3-7-sonnet" -> "claude-3-7-sonnet")
    let model_short = if let Some((_, model_name)) = model_raw.split_once('/') {
        model_name
    } else {
        model_raw
    };

    let model_display = if model_short.chars().count() > 18 {
        let truncated: String = model_short.chars().take(16).collect();
        format!("{truncated}…")
    } else {
        model_short.to_string()
    };

    let model_label = format!("{}: {}", fl!("model"), model_display);
    let model_tooltip = format!("{}: {} ({})", fl!("model"), model_raw, fl!("cycle-model"));

    let model_btn = widget::button::text(model_label)
        .font_weight(cosmic::iced::font::Weight::Light)
        .font_size(12)
        .padding([2, 4])
        .tooltip(model_tooltip)
        .on_press(Message::CycleModel);

    // Context window circular progress display
    let tokens_in = controller.map(|c| c.tokens_in()).unwrap_or(0);

    let model_context_window_length = controller
        .and_then(|c| c.model_context_window())
        .unwrap_or_else(|| context_window_for_model(Some(model_raw)));

    let progress = if model_context_window_length > 0 {
        (tokens_in as f32 / model_context_window_length as f32).clamp(0.0, 1.0)
    } else {
        0.0
    };

    let context_tooltip = fl!(
        "context-window",
        tokens_in = tokens_in,
        model_context_window_length = model_context_window_length
    );

    let context_progress_widget = widget::tooltip(
        widget::determinate_circular(progress).size(16.0),
        widget::text(context_tooltip),
        widget::tooltip::Position::Top,
    );

    // Thinking level display
    let thinking_str = match controller.and_then(|c| c.thinking_level()) {
        Some(ThinkingLevel::Off) => fl!("level-off"),
        Some(ThinkingLevel::Low) | Some(ThinkingLevel::Minimal) => fl!("level-low"),
        Some(ThinkingLevel::Medium) => fl!("level-medium"),
        Some(ThinkingLevel::High) | Some(ThinkingLevel::Xhigh) | Some(ThinkingLevel::Max) => {
            fl!("level-high")
        }
        None => match config.thinking_level.as_deref() {
            Some("off") => fl!("level-off"),
            Some("low") | Some("minimal") => fl!("level-low"),
            Some("medium") => fl!("level-medium"),
            Some("high") | Some("xhigh") | Some("max") => fl!("level-high"),
            Some(s) => s.to_string(),
            None => fl!("level-medium"),
        },
    };

    let thinking_label = format!("{}: {}", fl!("thinking-level"), thinking_str);
    let thinking_tooltip = format!("{thinking_label} ({})", fl!("cycle-thinking"));

    let thinking_btn = widget::button::text(thinking_label)
        .font_weight(cosmic::iced::font::Weight::Light)
        .font_size(12)
        .padding([2, 4])
        .tooltip(thinking_tooltip)
        .on_press(Message::CycleThinkingLevel);

    let footer_row = widget::row::with_capacity(5)
        .push(model_btn)
        .push(widget::space::horizontal())
        .push(context_progress_widget)
        .push(widget::space::horizontal())
        .push(thinking_btn)
        .spacing(6)
        .align_y(cosmic::iced::Alignment::Center)
        .width(Length::Fill);

    footer_row.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_window_for_model() {
        assert_eq!(
            context_window_for_model(Some("anthropic/claude-3-7-sonnet")),
            200_000
        );
        assert_eq!(context_window_for_model(Some("claude-3-5-sonnet")), 200_000);
        assert_eq!(context_window_for_model(Some("openai/gpt-4o")), 128_000);
        assert_eq!(
            context_window_for_model(Some("google/gemini-2.5-pro")),
            1_000_000
        );
        assert_eq!(
            context_window_for_model(Some("deepseek/deepseek-chat")),
            64_000
        );
        assert_eq!(context_window_for_model(Some("openai/o1")), 200_000);
        assert_eq!(context_window_for_model(Some("custom-model")), 128_000);
        assert_eq!(context_window_for_model(None), 128_000);
    }

    #[test]
    fn test_context_window_tooltip_localization() {
        let tooltip = fl!(
            "context-window",
            tokens_in = 1200,
            model_context_window_length = 200_000
        );
        assert!(tooltip.contains("Context window:"));
        assert!(tooltip.contains("1200") || tooltip.contains("1,200"));
        assert!(tooltip.contains("200000") || tooltip.contains("200,000"));
    }

    #[test]
    fn test_popover_dimensions_safety() {
        // Popover height must stay <= 600.0 so it never reaches the bottom of the screen
        // even on scaled or lower resolution displays (e.g. 720p / 768p / 1002p logical).
        assert!(POPOVER_HEIGHT <= 600.0);
        assert!(POPOVER_HEIGHT >= 450.0);
        assert!(POPOVER_WIDTH >= 360.0);
        assert!(POPOVER_WIDTH <= 500.0);
    }

    #[test]
    fn test_bottom_section_rendering() {
        let config = Config::default();
        let _ = view_bottom_section("hello world", None, &config, None);
        // Test rendering when prompt_input is a slash command (shows suggestion chips)
        let _ = view_bottom_section("/", None, &config, None);
        let _ = view_bottom_section("/h", None, &config, None);
        // Test rendering when prompt_input is an @ file reference (shows file chips)
        let _ = view_bottom_section("@", None, &config, None);
        let _ = view_bottom_section("@src", None, &config, None);
    }

    #[test]
    fn test_settings_and_header_rendering() {
        let config = Config::default();
        let _ = view_settings(&config);
        let _ = view_header(&config, false);
        let _ = view_header(&config, true);
    }

    #[test]
    fn test_starting_card_rendering() {
        let card = view_starting_card();
        let _ = card;
        assert!(!PI_LOGO_COLOR_SVG.is_empty());
        assert!(!PLUS_SYMBOLIC_SVG.is_empty());
        assert!(!COSMIC_SYMBOLIC_SVG.is_empty());
    }
}
