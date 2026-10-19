// SPDX-License-Identifier: EUPL-1.2

use std::path::PathBuf;

use cosmic::Application;
use cosmic_ext_applet_pi::app::{AppModel, Message};
use cosmic_ext_applet_pi::config::Config;
use cosmic_ext_applet_pi::popover;
use cosmic_ext_pi::controller::AgentStatus;

#[test]
fn test_integration_config_flow() {
    let mut config = Config::default();
    assert!(config.autostart_daemon);
    assert_eq!(
        config.effective_default_folder(),
        cosmic_ext_applet_pi::config::home_dir()
    );

    let temp_base = std::env::temp_dir().join("cosmic_pi_test_flow");
    let p1 = temp_base.join("alpha");
    let p2 = temp_base.join("beta");
    let p3 = temp_base.join("gamma");
    let _ = std::fs::create_dir_all(&p1);
    let _ = std::fs::create_dir_all(&p2);
    let _ = std::fs::create_dir_all(&p3);

    config.add_recent_project(p1.clone());
    config.add_recent_project(p2.clone());

    assert_eq!(config.effective_working_dir(), p2);

    // Explicit working dir overrides recent
    config.working_dir = Some(p3.clone());
    assert_eq!(config.effective_working_dir(), p3);

    let _ = std::fs::remove_dir_all(&temp_base);
}

#[tokio::test]
async fn test_integration_popover_rendering_variants() {
    let mut config = Config::default();
    config.add_recent_project(PathBuf::from("/home/user/project1"));
    config.add_recent_project(PathBuf::from("/home/user/project2"));

    // Render with no controller (idle / not started)
    let _ = popover::view_popover(&config, None, false, None, "", None, false);

    // Render in spawning state
    let _ = popover::view_popover(
        &config,
        None,
        true,
        None,
        "Awaiting connection",
        None,
        false,
    );

    // Render in error state
    let _ = popover::view_popover(
        &config,
        None,
        false,
        Some("Command failed: pi not found in $PATH"),
        "",
        None,
        false,
    );

    // Render settings view
    let _ = popover::view_popover(&config, None, false, None, "", None, true);

    // Render with mock controller having active token usage
    let (client_stdin, _) = tokio::io::duplex(1024);
    let (_, client_stdout) = tokio::io::duplex(1024);
    let client = std::sync::Arc::new(pi_rpc::PiClient::from_io(
        Box::new(client_stdin),
        Box::new(client_stdout),
        None,
    ));
    let mut controller = cosmic_ext_pi::PiSessionController::new(client);
    controller.set_model(Some("anthropic/claude-3-7-sonnet".into()));
    controller.set_model_context_window(Some(200_000));
    let _ = controller.update(cosmic_ext_pi::subscription::PiAction::Event(
        pi_rpc::protocol::PiEvent::MessageUpdate {
            usage: Some(pi_rpc::protocol::Usage {
                input: 42_000,
                output: 1_200,
                cache_read: 8_000,
                cache_write: 0,
                cache_write1h: None,
                reasoning: None,
                total_tokens: 51_200,
                cost: None,
            }),
            assistant_message_event: pi_rpc::protocol::AssistantMessageEvent::Start,
        },
    ));

    assert_eq!(controller.tokens_in(), 50_000);
    assert_eq!(controller.model_context_window(), Some(200_000));

    let _ = popover::view_popover(
        &config,
        Some(&controller),
        false,
        None,
        "How are you?",
        Some("anthropic/claude-3-7-sonnet"),
        false,
    );

    // Render with newest_first explicitly disabled
    config.newest_first = false;
    let _ = popover::view_popover(
        &config,
        Some(&controller),
        false,
        None,
        "How are you?",
        Some("anthropic/claude-3-7-sonnet"),
        false,
    );

    // Render busy state with prompt input and queued steering/follow-up messages
    let _ = controller.update(cosmic_ext_pi::subscription::PiAction::Event(
        pi_rpc::protocol::PiEvent::MessageUpdate {
            usage: None,
            assistant_message_event: pi_rpc::protocol::AssistantMessageEvent::ThinkingStart {
                content_index: 0,
            },
        },
    ));
    assert!(controller.status().is_busy());

    let _ = controller.update(cosmic_ext_pi::subscription::PiAction::Event(
        pi_rpc::protocol::PiEvent::QueueUpdate {
            steering: vec!["Change algorithm".to_string()],
            follow_up: vec!["Check performance afterwards".to_string()],
        },
    ));

    let _ = popover::view_popover(
        &config,
        Some(&controller),
        false,
        None,
        "New input while busy",
        Some("anthropic/claude-3-7-sonnet"),
        false,
    );
}

#[test]
fn test_integration_app_model_lifecycle() {
    let tmp_config = std::env::temp_dir().join("cosmic_applet_test_config");
    let _ = std::fs::create_dir_all(&tmp_config);
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", &tmp_config);
    }

    let mut app = AppModel::default();
    assert_eq!(app.status(), AgentStatus::Idle);

    // Prompt change
    let _ = app.update(Message::PromptInputChanged("Refactor code".into()));
    assert_eq!(format!("{:?}", Message::SubmitPrompt), "SubmitPrompt");
    assert_eq!(format!("{:?}", Message::SubmitSteer), "SubmitSteer");
    assert_eq!(format!("{:?}", Message::SubmitFollowUp), "SubmitFollowUp");

    // SubmitSteer and SubmitFollowUp with empty vs non-empty input
    let _ = app.update(Message::SubmitSteer);
    let _ = app.update(Message::PromptInputChanged("Follow-up text".into()));
    let _ = app.update(Message::SubmitFollowUp);

    // Toggle settings
    let _ = app.update(Message::ToggleSettings);
    assert!(app.show_settings);
    let _ = app.update(Message::ToggleSettings);
    assert!(!app.show_settings);

    // RestartPi from settings resets show_settings
    let _ = app.update(Message::ToggleSettings);
    assert!(app.show_settings);
    let _ = app.update(Message::RestartPi);
    assert!(!app.show_settings);

    // Settings toggles
    let _ = app.update(Message::ToggleAutostart(false));
    assert!(!app.config().autostart_daemon);
    let _ = app.update(Message::ToggleNotifyComplete(false));
    assert!(!app.config().notify_on_complete);
    let _ = app.update(Message::ToggleNotifyApproval(false));
    assert!(!app.config().notify_on_approval);
    let _ = app.update(Message::ToggleNewestFirst(false));
    assert!(!app.config().newest_first);
    let _ = app.update(Message::ToggleNewestFirst(true));
    assert!(app.config().newest_first);

    // Default folder reset
    let _ = app.update(Message::ResetDefaultFolder);
    assert_eq!(
        app.config().default_folder,
        Some(cosmic_ext_applet_pi::config::home_dir())
    );

    // Project selection with valid temp dir
    let proj = std::env::temp_dir().join("cosmic-pi-test-dir");
    let _ = std::fs::create_dir_all(&proj);
    let _ = app.update(Message::SelectRecentProject(proj.clone()));
    assert_eq!(app.status(), AgentStatus::Idle);
    let _ = std::fs::remove_dir_all(&proj);
}
