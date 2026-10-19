use std::sync::Arc;
use tokio::io::duplex;

use cosmic_ext_pi::controller::{AgentStatus, MessageBlock, PiSessionController};
use cosmic_ext_pi::subscription::PiAction;
use pi_rpc::protocol::{
    AssistantMessageEvent, ExtensionUiRequest, ExtensionUiResponse, PiEvent, ThinkingLevel,
};
use pi_rpc::PiClient;
use serde_json::json;

fn create_mock_controller() -> (PiSessionController, Arc<PiClient>) {
    let (client_stdin, _server_stdin) = duplex(4096);
    let (_server_stdout, client_stdout) = duplex(4096);
    let client = Arc::new(PiClient::from_io(
        Box::new(client_stdin),
        Box::new(client_stdout),
        None,
    ));
    let controller = PiSessionController::new(Arc::clone(&client));
    (controller, client)
}

#[tokio::test]
async fn test_controller_initial_state() {
    let (controller, _) = create_mock_controller();
    assert_eq!(*controller.status(), AgentStatus::Idle);
    assert!(controller.history().is_empty());
    assert!(controller.session_id().is_none());
    assert!(controller.view_active_dialog().is_none());
}

#[tokio::test]
async fn test_controller_streaming_text_deltas() {
    let (mut controller, _) = create_mock_controller();

    // 1. Agent starts turn
    let _ = controller.update(PiAction::Event(PiEvent::TurnStart));
    assert_eq!(controller.history().len(), 1);
    assert_eq!(controller.history()[0].role, "assistant");
    assert!(controller.history()[0].is_streaming);

    // 2. Stream message text chunks
    let _ = controller.update(PiAction::Event(PiEvent::MessageUpdate {
        usage: None,
        assistant_message_event: AssistantMessageEvent::TextStart { content_index: 0 },
    }));

    let _ = controller.update(PiAction::Event(PiEvent::MessageUpdate {
        usage: None,
        assistant_message_event: AssistantMessageEvent::TextDelta {
            content_index: 0,
            delta: "Hello, ".to_string(),
        },
    }));

    let _ = controller.update(PiAction::Event(PiEvent::MessageUpdate {
        usage: None,
        assistant_message_event: AssistantMessageEvent::TextDelta {
            content_index: 0,
            delta: "world!".to_string(),
        },
    }));

    assert_eq!(controller.history()[0].text(), "Hello, world!");

    // 3. Text block end
    let _ = controller.update(PiAction::Event(PiEvent::MessageUpdate {
        usage: None,
        assistant_message_event: AssistantMessageEvent::TextEnd {
            content_index: 0,
            content: serde_json::Value::Null,
        },
    }));

    // Verify markdown was parsed and cached
    if let MessageBlock::Text { parsed, .. } = &controller.history()[0].blocks[0] {
        assert!(!parsed.is_empty());
    } else {
        panic!("Expected MessageBlock::Text");
    }

    // 4. Agent settles
    let _ = controller.update(PiAction::Event(PiEvent::AgentSettled { aborted: false }));
    assert_eq!(*controller.status(), AgentStatus::Idle);
    assert!(!controller.history()[0].is_streaming);
}

#[tokio::test]
async fn test_controller_thinking_and_collapse_toggle() {
    let (mut controller, _) = create_mock_controller();

    let _ = controller.update(PiAction::Event(PiEvent::TurnStart));

    // Thinking starts
    let _ = controller.update(PiAction::Event(PiEvent::MessageUpdate {
        usage: None,
        assistant_message_event: AssistantMessageEvent::ThinkingStart { content_index: 0 },
    }));
    assert_eq!(*controller.status(), AgentStatus::Thinking);

    // Stream thinking deltas
    let _ = controller.update(PiAction::Event(PiEvent::MessageUpdate {
        usage: None,
        assistant_message_event: AssistantMessageEvent::ThinkingDelta {
            content_index: 0,
            delta: "Let me consider ".to_string(),
        },
    }));
    let _ = controller.update(PiAction::Event(PiEvent::MessageUpdate {
        usage: None,
        assistant_message_event: AssistantMessageEvent::ThinkingDelta {
            content_index: 0,
            delta: "the options.".to_string(),
        },
    }));

    // Thinking ends
    let _ = controller.update(PiAction::Event(PiEvent::MessageUpdate {
        usage: None,
        assistant_message_event: AssistantMessageEvent::ThinkingEnd {
            content_index: 0,
            content: serde_json::Value::Null,
        },
    }));

    match &controller.history()[0].blocks[0] {
        MessageBlock::Thinking {
            text,
            collapsed,
            is_streaming,
        } => {
            assert_eq!(text, "Let me consider the options.");
            assert!(collapsed);
            assert!(!is_streaming);
        }
        other => panic!("Expected MessageBlock::Thinking, got {:?}", other),
    }

    // Toggle collapse on thinking block (first toggle expands it)
    let _ = controller.update(PiAction::ToggleThinking {
        message_index: 0,
        block_index: 0,
    });
    match &controller.history()[0].blocks[0] {
        MessageBlock::Thinking { collapsed, .. } => assert!(!collapsed),
        _ => panic!(),
    }

    // Second toggle collapses it again
    let _ = controller.update(PiAction::ToggleThinking {
        message_index: 0,
        block_index: 0,
    });
    match &controller.history()[0].blocks[0] {
        MessageBlock::Thinking { collapsed, .. } => assert!(collapsed),
        _ => panic!(),
    }
}

#[tokio::test]
async fn test_controller_tool_call_lifecycle() {
    let (mut controller, _) = create_mock_controller();

    let _ = controller.update(PiAction::Event(PiEvent::TurnStart));

    // 1. Tool call announced in message
    let _ = controller.update(PiAction::Event(PiEvent::MessageUpdate {
        usage: None,
        assistant_message_event: AssistantMessageEvent::ToolcallStart {
            content_index: 0,
            id: "call-1".to_string(),
            tool_name: "bash".to_string(),
        },
    }));

    let _ = controller.update(PiAction::Event(PiEvent::MessageUpdate {
        usage: None,
        assistant_message_event: AssistantMessageEvent::ToolcallDelta {
            content_index: 0,
            delta: r#"{"command": "cargo check"}"#.to_string(),
        },
    }));

    // 2. Tool execution starts
    let _ = controller.update(PiAction::Event(PiEvent::ToolExecutionStart {
        tool_call_id: "call-1".to_string(),
        tool_name: "bash".to_string(),
        args: json!({ "command": "cargo check" }),
    }));

    assert_eq!(
        *controller.status(),
        AgentStatus::ExecutingTool {
            name: "bash".to_string(),
            tool_call_id: "call-1".to_string(),
        }
    );

    // 3. Tool execution update
    let _ = controller.update(PiAction::Event(PiEvent::ToolExecutionUpdate {
        tool_call_id: "call-1".to_string(),
        tool_name: "bash".to_string(),
        args: json!({ "command": "cargo check" }),
        partial_result: Some(json!("Compiling...")),
    }));

    // 4. Tool execution end
    let _ = controller.update(PiAction::Event(PiEvent::ToolExecutionEnd {
        tool_call_id: "call-1".to_string(),
        tool_name: "bash".to_string(),
        result: Some(json!("Finished dev profile")),
        is_error: false,
        duration_ms: Some(150.0),
    }));

    assert_eq!(*controller.status(), AgentStatus::Thinking);

    // Verify tool block state
    if let MessageBlock::ToolCall(tool) = &controller.history()[0].blocks[0] {
        assert_eq!(tool.id, "call-1");
        assert_eq!(tool.name, "bash");
        assert_eq!(tool.result, Some(json!("Finished dev profile")));
        assert_eq!(tool.duration_ms, Some(150.0));
        assert!(!tool.is_error);
        assert!(!tool.is_executing);
        assert!(tool.collapsed);
    } else {
        panic!("Expected MessageBlock::ToolCall");
    }

    // Toggle collapse on tool call card (first toggle expands it)
    let _ = controller.update(PiAction::ToggleTool("call-1".to_string()));
    if let MessageBlock::ToolCall(tool) = &controller.history()[0].blocks[0] {
        assert!(!tool.collapsed);
    }
}

#[tokio::test]
async fn test_controller_extension_ui_confirm_and_cancel() {
    let (mut controller, _) = create_mock_controller();

    let req = ExtensionUiRequest::Confirm {
        id: "ui-101".to_string(),
        title: "Run command?".to_string(),
        message: "Allow bash execution?".to_string(),
        timeout: None,
    };

    let _ = controller.update(PiAction::UiRequest(req));

    assert_eq!(
        *controller.status(),
        AgentStatus::AwaitingUser {
            request_id: "ui-101".to_string(),
        }
    );
    assert_eq!(controller.pending_dialogs_count(), 1);
    assert!(controller.view_active_dialog().is_some());

    // Respond by confirming false (deny)
    let _ = controller.update(PiAction::DialogConfirm("ui-101".to_string(), false));

    assert_eq!(*controller.status(), AgentStatus::Idle);
    assert_eq!(controller.pending_dialogs_count(), 0);
    assert!(controller.view_active_dialog().is_none());
}

#[tokio::test]
async fn test_controller_extension_ui_input_and_select() {
    let (mut controller, _) = create_mock_controller();

    // 1. Select request
    let select_req = ExtensionUiRequest::Select {
        id: "ui-sel".to_string(),
        title: "Pick option".to_string(),
        options: vec!["A".to_string(), "B".to_string()],
        timeout: None,
    };
    let _ = controller.update(PiAction::UiRequest(select_req));

    assert_eq!(controller.pending_dialogs_count(), 1);
    let _ = controller.update(PiAction::DialogSelectOption("ui-sel".to_string(), 1));

    // Respond
    let _ = controller.update(PiAction::RespondUi(ExtensionUiResponse::value(
        "ui-sel", "B",
    )));
    assert_eq!(controller.pending_dialogs_count(), 0);

    // 2. Input request
    let input_req = ExtensionUiRequest::Input {
        id: "ui-inp".to_string(),
        title: "Enter name".to_string(),
        placeholder: Some("Name".to_string()),
        timeout: None,
    };
    let _ = controller.update(PiAction::UiRequest(input_req));

    let _ = controller.update(PiAction::DialogInputChanged(
        "ui-inp".to_string(),
        "Alice".to_string(),
    ));

    let _ = controller.update(PiAction::RespondUi(ExtensionUiResponse::value(
        "ui-inp", "Alice",
    )));
    assert_eq!(controller.pending_dialogs_count(), 0);
}

#[tokio::test]
async fn test_controller_thinking_level_and_session_info() {
    let (mut controller, _) = create_mock_controller();

    let _ = controller.update(PiAction::Event(PiEvent::SessionInfoChanged {
        name: Some("Cosmic Session".to_string()),
    }));
    assert_eq!(controller.session_name(), Some("Cosmic Session"));

    let _ = controller.update(PiAction::Event(PiEvent::ThinkingLevelChanged {
        level: ThinkingLevel::High,
    }));
    assert_eq!(controller.thinking_level(), Some(ThinkingLevel::High));

    let _ = controller.update(PiAction::Event(PiEvent::QueueUpdate {
        steering: vec!["Steer text".to_string()],
        follow_up: vec!["Follow up text".to_string()],
    }));
    assert_eq!(controller.steering_queue(), &["Steer text"]);
    assert_eq!(controller.follow_up_queue(), &["Follow up text"]);
}

#[tokio::test]
async fn test_controller_usage_tokens_in_and_context_window() {
    let (mut controller, _) = create_mock_controller();

    assert_eq!(controller.tokens_in(), 0);
    assert_eq!(controller.model_context_window(), None);

    controller.set_model_context_window(Some(200_000));
    assert_eq!(controller.model_context_window(), Some(200_000));

    let usage = pi_rpc::protocol::Usage {
        input: 1500,
        output: 300,
        cache_read: 500,
        cache_write: 0,
        cache_write1h: None,
        reasoning: None,
        total_tokens: 2300,
        cost: None,
    };

    let _ = controller.update(PiAction::Event(PiEvent::MessageUpdate {
        usage: Some(usage),
        assistant_message_event: AssistantMessageEvent::Start,
    }));

    // tokens_in should be input (1500) + cache_read (500) = 2000
    assert_eq!(controller.tokens_in(), 2000);
}

#[tokio::test]
async fn test_controller_slash_commands_help_and_unknown() {
    let (mut controller, _) = create_mock_controller();

    // 1. Submit /help
    let _ = controller.submit_prompt("/help");

    // History should have user "/help" and system response with help markdown
    assert_eq!(controller.history().len(), 2);
    assert_eq!(controller.history()[0].role, "user");
    assert_eq!(controller.history()[0].text(), "/help");

    assert_eq!(controller.history()[1].role, "system");
    let help_text = controller.history()[1].text();
    assert!(help_text.contains("### Available Commands"));
    assert!(help_text.contains("/help"));
    assert!(help_text.contains("/compact"));
    assert!(help_text.contains("/new"));
    assert_eq!(*controller.status(), AgentStatus::Idle);

    // 2. Submit unknown slash command
    let _ = controller.submit_prompt("/invalidcmd");
    assert_eq!(controller.history().len(), 4);
    assert_eq!(controller.history()[2].text(), "/invalidcmd");
    assert_eq!(controller.history()[3].role, "system");
    assert!(controller.history()[3].text().contains("Unknown command `/invalidcmd`"));
    assert_eq!(*controller.status(), AgentStatus::Idle);
}

#[tokio::test]
async fn test_controller_slash_commands_discovery() {
    let (mut controller, _) = create_mock_controller();

    let dummy_commands = vec![
        pi_rpc::protocol::RpcSlashCommand {
            name: "skill:but".to_string(),
            description: Some("GitButler version control".to_string()),
            source: "skill".to_string(),
            source_info: None,
        },
        pi_rpc::protocol::RpcSlashCommand {
            name: "antigravity.usage".to_string(),
            description: Some("Check usage quota".to_string()),
            source: "extension".to_string(),
            source_info: None,
        },
    ];

    let _ = controller.update(PiAction::SlashCommandsLoaded(dummy_commands));
    assert_eq!(controller.discovered_commands().len(), 2);

    let all = controller.available_commands();
    assert!(all.iter().any(|c| c.name == "help" && c.is_builtin));
    assert!(all.iter().any(|c| c.name == "skill:but" && !c.is_builtin));
    assert!(all.iter().any(|c| c.name == "antigravity.usage" && !c.is_builtin));

    let help = controller.format_help_markdown();
    assert!(help.contains("/skill:but"));
    assert!(help.contains("/antigravity.usage"));
}

#[tokio::test]
async fn test_controller_extension_ui_notify_and_prompt_handled() {
    let (mut controller, _) = create_mock_controller();

    // 1. Extension sends an info notification (e.g. from /mcp)
    let notify_req = ExtensionUiRequest::Notify {
        id: Some("notif-1".to_string()),
        message: "radius: connected, 123 tools (codemode)".to_string(),
        notify_type: Some(pi_rpc::protocol::NotifyType::Info),
    };
    let _ = controller.update(PiAction::UiRequest(notify_req));

    // Must be added to history as a system message
    assert_eq!(controller.history().len(), 1);
    assert_eq!(controller.history()[0].role, "system");
    assert_eq!(
        controller.history()[0].text(),
        "radius: connected, 123 tools (codemode)"
    );

    // Must NOT trigger AwaitingUser status or set active dialog
    assert_eq!(*controller.status(), AgentStatus::Idle);
    assert!(controller.view_active_dialog().is_none());
    assert_eq!(controller.pending_dialogs_count(), 0);

    // 2. Test slash command /mcp workflow:
    // User submits /mcp
    let _ = controller.submit_prompt("/mcp");
    assert_eq!(*controller.status(), AgentStatus::Thinking);

    // Prompt disposition Handled arrives (from synchronous CLI/extension command)
    let _ = controller.update(PiAction::PromptCompleted(Ok(
        pi_rpc::protocol::PromptDisposition::Handled,
    )));

    // Status must reset to Idle instead of remaining stuck in Thinking
    assert_eq!(*controller.status(), AgentStatus::Idle);
}

#[tokio::test]
async fn test_controller_steer_and_follow_up_lifecycle() {
    let (mut controller, _) = create_mock_controller();

    // 1. Initial prompt
    let _ = controller.submit_prompt("Build a web server");
    assert_eq!(controller.history().len(), 1);
    assert_eq!(controller.history()[0].role, "user");
    assert_eq!(controller.history()[0].text(), "Build a web server");

    // 2. Initial assistant turn
    let _ = controller.update(PiAction::Event(PiEvent::TurnStart));
    let _ = controller.update(PiAction::Event(PiEvent::MessageStart {
        message: pi_rpc::protocol::AgentMessage {
            role: "user".to_string(),
            content: serde_json::json!("Build a web server"),
            timestamp: Some(100),
            extra: Default::default(),
        },
    }));
    let _ = controller.update(PiAction::Event(PiEvent::MessageEnd {
        message: pi_rpc::protocol::AgentMessage {
            role: "user".to_string(),
            content: serde_json::json!("Build a web server"),
            timestamp: Some(101),
            extra: Default::default(),
        },
    }));
    // Should NOT duplicate initial prompt
    assert_eq!(controller.history().len(), 1);

    // Assistant response begins
    let _ = controller.update(PiAction::Event(PiEvent::MessageStart {
        message: pi_rpc::protocol::AgentMessage {
            role: "assistant".to_string(),
            content: serde_json::json!([]),
            timestamp: Some(102),
            extra: Default::default(),
        },
    }));
    let _ = controller.update(PiAction::Event(PiEvent::MessageUpdate {
        usage: None,
        assistant_message_event: AssistantMessageEvent::TextStart { content_index: 0 },
    }));
    let _ = controller.update(PiAction::Event(PiEvent::MessageUpdate {
        usage: None,
        assistant_message_event: AssistantMessageEvent::TextDelta {
            content_index: 0,
            delta: "Starting server in Node.js...".to_string(),
        },
    }));
    let _ = controller.update(PiAction::Event(PiEvent::MessageUpdate {
        usage: None,
        assistant_message_event: AssistantMessageEvent::TextEnd {
            content_index: 0,
            content: serde_json::Value::Null,
        },
    }));
    let _ = controller.update(PiAction::Event(PiEvent::MessageEnd {
        message: pi_rpc::protocol::AgentMessage {
            role: "assistant".to_string(),
            content: serde_json::json!([]),
            timestamp: Some(103),
            extra: Default::default(),
        },
    }));
    let _ = controller.update(PiAction::Event(PiEvent::TurnEnd {
        message: pi_rpc::protocol::AgentMessage {
            role: "assistant".to_string(),
            content: serde_json::json!([]),
            timestamp: Some(103),
            extra: Default::default(),
        },
        tool_results: vec![],
    }));

    assert_eq!(controller.history().len(), 2);
    assert_eq!(controller.history()[1].role, "assistant");
    assert_eq!(controller.history()[1].text(), "Starting server in Node.js...");

    // 3. User sends a Steer while busy
    let _ = controller.update(PiAction::Event(PiEvent::QueueUpdate {
        steering: vec!["Use Rust instead".to_string()],
        follow_up: vec![],
    }));
    assert_eq!(controller.steering_queue(), &["Use Rust instead"]);

    // 4. Steer turn starts: Pi emits TurnStart, then user message for the steer
    let _ = controller.update(PiAction::Event(PiEvent::TurnStart));
    let _ = controller.update(PiAction::Event(PiEvent::MessageStart {
        message: pi_rpc::protocol::AgentMessage {
            role: "user".to_string(),
            content: serde_json::json!([{"type": "text", "text": "Use Rust instead"}]),
            timestamp: Some(200),
            extra: Default::default(),
        },
    }));
    let _ = controller.update(PiAction::Event(PiEvent::MessageEnd {
        message: pi_rpc::protocol::AgentMessage {
            role: "user".to_string(),
            content: serde_json::json!([{"type": "text", "text": "Use Rust instead"}]),
            timestamp: Some(201),
            extra: Default::default(),
        },
    }));

    // Verify: The steer message is in history as a user message, and NO empty assistant turn was left
    assert_eq!(controller.history().len(), 3);
    assert_eq!(controller.history()[2].role, "user");
    assert_eq!(controller.history()[2].text(), "Use Rust instead");

    // 5. Assistant responds to steer
    let _ = controller.update(PiAction::Event(PiEvent::MessageStart {
        message: pi_rpc::protocol::AgentMessage {
            role: "assistant".to_string(),
            content: serde_json::json!([]),
            timestamp: Some(202),
            extra: Default::default(),
        },
    }));
    let _ = controller.update(PiAction::Event(PiEvent::MessageUpdate {
        usage: None,
        assistant_message_event: AssistantMessageEvent::TextStart { content_index: 0 },
    }));
    let _ = controller.update(PiAction::Event(PiEvent::MessageUpdate {
        usage: None,
        assistant_message_event: AssistantMessageEvent::TextDelta {
            content_index: 0,
            delta: "Switching to Rust axum...".to_string(),
        },
    }));
    let _ = controller.update(PiAction::Event(PiEvent::MessageUpdate {
        usage: None,
        assistant_message_event: AssistantMessageEvent::TextEnd {
            content_index: 0,
            content: serde_json::Value::Null,
        },
    }));
    let _ = controller.update(PiAction::Event(PiEvent::MessageEnd {
        message: pi_rpc::protocol::AgentMessage {
            role: "assistant".to_string(),
            content: serde_json::json!([]),
            timestamp: Some(203),
            extra: Default::default(),
        },
    }));
    let _ = controller.update(PiAction::Event(PiEvent::TurnEnd {
        message: pi_rpc::protocol::AgentMessage {
            role: "assistant".to_string(),
            content: serde_json::json!([]),
            timestamp: Some(203),
            extra: Default::default(),
        },
        tool_results: vec![],
    }));

    // History should have exactly 4 messages, in correct order, with NO empty turns
    assert_eq!(controller.history().len(), 4);
    assert_eq!(controller.history()[0].role, "user");
    assert_eq!(controller.history()[0].text(), "Build a web server");
    assert_eq!(controller.history()[1].role, "assistant");
    assert_eq!(controller.history()[1].text(), "Starting server in Node.js...");
    assert_eq!(controller.history()[2].role, "user");
    assert_eq!(controller.history()[2].text(), "Use Rust instead");
    assert_eq!(controller.history()[3].role, "assistant");
    assert_eq!(controller.history()[3].text(), "Switching to Rust axum...");

    // 6. Follow-up turn works identically
    let _ = controller.update(PiAction::Event(PiEvent::TurnStart));
    let _ = controller.update(PiAction::Event(PiEvent::MessageStart {
        message: pi_rpc::protocol::AgentMessage {
            role: "user".to_string(),
            content: serde_json::json!([{"type": "text", "text": "Add unit tests"}]),
            timestamp: Some(300),
            extra: Default::default(),
        },
    }));
    let _ = controller.update(PiAction::Event(PiEvent::MessageEnd {
        message: pi_rpc::protocol::AgentMessage {
            role: "user".to_string(),
            content: serde_json::json!([{"type": "text", "text": "Add unit tests"}]),
            timestamp: Some(301),
            extra: Default::default(),
        },
    }));

    assert_eq!(controller.history().len(), 5);
    assert_eq!(controller.history()[4].role, "user");
    assert_eq!(controller.history()[4].text(), "Add unit tests");

    // Agent settles
    let _ = controller.update(PiAction::Event(PiEvent::AgentSettled { aborted: false }));
    assert_eq!(*controller.status(), AgentStatus::Idle);
}

#[tokio::test]
async fn test_controller_empty_assistant_message_cleaned_up_on_abort() {
    let (mut controller, _) = create_mock_controller();

    // Turn starts and pushes an empty assistant message
    let _ = controller.update(PiAction::Event(PiEvent::TurnStart));
    assert_eq!(controller.history().len(), 1);
    assert_eq!(controller.history()[0].role, "assistant");
    assert!(controller.history()[0].blocks.is_empty());

    // Abort arrives before any content blocks are emitted
    let _ = controller.update(PiAction::Event(PiEvent::AgentSettled { aborted: true }));

    // The empty assistant message must be cleaned up
    assert!(controller.history().is_empty());
}
