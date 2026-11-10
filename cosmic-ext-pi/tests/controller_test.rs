use std::sync::Arc;
use tokio::io::duplex;

use cosmic_ext_pi::controller::{AgentStatus, MessageBlock, PiSessionController};
use cosmic_ext_pi::subscription::PiAction;
use pi_rpc::protocol::{
    AgentMessage, AgentState, EntryRecord, ExtensionUiRequest, ExtensionUiResponse,
    GenerationStatus, InboxMode, MessageChange, ModelRef, PiEvent, QueuedItem, RunStatus,
    SnapshotEvent, ThinkingLevel, ToolOutputUpdate, ToolSlot, Usage, UsageState,
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
async fn test_controller_snapshot_hydration_and_mid_run_restore() {
    let (mut controller, _) = create_mock_controller();

    let mut usage_state = UsageState::default();
    usage_state.models.insert(
        "radius/balanced".to_string(),
        Usage {
            input: 1200,
            output: 300,
            cache_read: 800,
            cache_write: 0,
            cache_write1h: None,
            reasoning: None,
            total_tokens: 2300,
            cost: None,
        },
    );

    let snap = SnapshotEvent {
        entries: vec![
            EntryRecord {
                id: 1,
                conversation_id: 1,
                kind: "pi.user".to_string(),
                model: Some(vec![AgentMessage {
                    role: "user".to_string(),
                    content: json!("Run cargo check"),
                    timestamp: Some(100),
                    usage: None,
                    tool_call_id: None,
                    tool_name: None,
                    is_error: None,
                    duration_ms: None,
                    extra: Default::default(),
                }]),
                data: None,
                head: None,
                by_task_id: None,
                extra: Default::default(),
            },
            EntryRecord {
                id: 2,
                conversation_id: 1,
                kind: "pi.assistant".to_string(),
                model: Some(vec![AgentMessage {
                    role: "assistant".to_string(),
                    content: json!([
                        { "type": "thinking", "thinking": "Checking workspace..." },
                        { "type": "toolCall", "id": "call-1", "name": "bash", "arguments": { "command": "cargo check" } }
                    ]),
                    timestamp: Some(101),
                    usage: None,
                    tool_call_id: None,
                    tool_name: None,
                    is_error: None,
                    duration_ms: None,
                    extra: Default::default(),
                }]),
                data: None,
                head: None,
                by_task_id: None,
                extra: Default::default(),
            },
        ],
        run: Some(RunStatus { inputs: vec![1] }),
        generation: Some(GenerationStatus {
            attempt: 1,
            message: None,
            retry: None,
            deferred: None,
        }),
        tools: vec![ToolSlot {
            call_id: "call-1".to_string(),
            name: "bash".to_string(),
            status: "running".to_string(),
            output: Some("Compiling crate...\n".to_string()),
            dropped_bytes: None,
            dropped_lines: None,
            details: None,
            diagnostics: None,
            task_id: Some(10),
            entry: None,
        }],
        nested_tools: vec![],
        compactions: vec![],
        compaction_failure: None,
        inbox: vec![QueuedItem {
            id: 2,
            mode: InboxMode::Steer,
            text: Some("Also run clippy".to_string()),
        }],
        agent: AgentState {
            model: Some(ModelRef::new("radius", "balanced")),
            thinking_level: Some(ThinkingLevel::High),
            cwd: Some("/tmp/proj".to_string()),
            instructions: None,
            extra: Default::default(),
        },
        usage: usage_state,
    };

    let _ = controller.update(PiAction::Event(PiEvent::Snapshot(snap)));

    // Verify full state hydrated from snapshot!
    assert_eq!(controller.session_id(), Some("durable-1"));
    assert_eq!(controller.model(), Some("radius/balanced"));
    assert_eq!(controller.thinking_level(), Some(ThinkingLevel::High));
    assert_eq!(controller.tokens_in(), 2000);
    assert_eq!(controller.steering_queue(), &["Also run clippy"]);
    assert_eq!(
        *controller.status(),
        AgentStatus::ExecutingTool {
            name: "bash".to_string(),
            tool_call_id: "call-1".to_string(),
        }
    );
    assert_eq!(controller.history().len(), 2);
    assert_eq!(controller.history()[0].role, "user");
    assert_eq!(controller.history()[0].text(), "Run cargo check");
    assert_eq!(controller.history()[1].role, "assistant");
    if let MessageBlock::ToolCall(tool) = &controller.history()[1].blocks[1] {
        assert_eq!(tool.id, "call-1");
        assert!(tool.is_executing);
        assert_eq!(tool.partial_result, Some(json!("Compiling crate...\n")));
    } else {
        panic!("Expected ToolCall block in hydrated assistant message");
    }
}

#[tokio::test]
async fn test_controller_streaming_text_deltas() {
    let (mut controller, _) = create_mock_controller();

    // 1. Agent starts turn
    let _ = controller.update(PiAction::Event(PiEvent::TurnStart));
    assert_eq!(controller.history().len(), 1);
    assert_eq!(controller.history()[0].role, "assistant");
    assert!(controller.history()[0].is_streaming);

    // 2. Stream message text chunks via MessageChange
    let _ = controller.update(PiAction::Event(PiEvent::MessageUpdate {
        usage: None,
        changes: vec![MessageChange::TextStart {
            content_index: 0,
            block: json!({ "type": "text", "text": "" }),
        }],
    }));

    let _ = controller.update(PiAction::Event(PiEvent::MessageUpdate {
        usage: None,
        changes: vec![MessageChange::TextDelta {
            content_index: 0,
            delta: "Hello, ".to_string(),
        }],
    }));

    let _ = controller.update(PiAction::Event(PiEvent::MessageUpdate {
        usage: None,
        changes: vec![MessageChange::TextDelta {
            content_index: 0,
            delta: "world!".to_string(),
        }],
    }));

    assert_eq!(controller.history()[0].text(), "Hello, world!");

    // 3. MessageEnd commits the durable entry
    let _ = controller.update(PiAction::Event(PiEvent::MessageEnd {
        entry: EntryRecord {
            id: 1,
            conversation_id: 1,
            kind: "pi.assistant".to_string(),
            model: Some(vec![AgentMessage {
                role: "assistant".to_string(),
                content: json!([{ "type": "text", "text": "Hello, world!" }]),
                timestamp: Some(100),
                usage: None,
                tool_call_id: None,
                tool_name: None,
                is_error: None,
                duration_ms: None,
                extra: Default::default(),
            }]),
            data: None,
            head: None,
            by_task_id: None,
            extra: Default::default(),
        },
    }));

    // Verify markdown was parsed and cached
    if let MessageBlock::Text { parsed, .. } = &controller.history()[0].blocks[0] {
        assert!(!parsed.is_empty());
    } else {
        panic!("Expected MessageBlock::Text");
    }

    // 4. Run ends
    let _ = controller.update(PiAction::Event(PiEvent::RunEnd { inputs: vec![1] }));
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
        changes: vec![MessageChange::ThinkingStart {
            content_index: 0,
            block: json!({ "type": "thinking", "thinking": "" }),
        }],
    }));
    assert_eq!(*controller.status(), AgentStatus::Thinking);

    // Stream thinking deltas
    let _ = controller.update(PiAction::Event(PiEvent::MessageUpdate {
        usage: None,
        changes: vec![
            MessageChange::ThinkingDelta {
                content_index: 0,
                delta: "Let me consider ".to_string(),
            },
            MessageChange::ThinkingDelta {
                content_index: 0,
                delta: "the options.".to_string(),
            },
        ],
    }));

    // Turn ends
    let _ = controller.update(PiAction::Event(PiEvent::TurnEnd));

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
        changes: vec![
            MessageChange::ToolcallStart {
                content_index: 0,
                block: json!({ "type": "toolCall", "id": "call-1", "name": "bash", "arguments": {} }),
            },
            MessageChange::ToolcallDelta {
                content_index: 0,
                path: vec![json!("command")],
                delta: "cargo check".to_string(),
            },
        ],
    }));

    // 2. Tool execution starts
    let _ = controller.update(PiAction::Event(PiEvent::ToolExecutionStart {
        tool_call_id: "call-1".to_string(),
        tool_name: "bash".to_string(),
        task_id: Some(10),
        parent_tool_call_id: None,
        parent_task_id: None,
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
        task_id: Some(10),
        parent_tool_call_id: None,
        parent_task_id: None,
        output: Some(ToolOutputUpdate {
            trim_start: None,
            append: Some("Compiling...".to_string()),
            set: None,
        }),
        details: None,
        diagnostics: None,
    }));

    // 4. Tool execution end with result entry
    let _ = controller.update(PiAction::Event(PiEvent::ToolExecutionEnd {
        tool_call_id: "call-1".to_string(),
        tool_name: "bash".to_string(),
        task_id: Some(10),
        parent_tool_call_id: None,
        parent_task_id: None,
        entry: Some(EntryRecord {
            id: 3,
            conversation_id: 1,
            kind: "pi.tool-result".to_string(),
            model: Some(vec![AgentMessage {
                role: "toolResult".to_string(),
                content: json!([{ "type": "text", "text": "Finished dev profile" }]),
                timestamp: Some(105),
                usage: None,
                tool_call_id: Some("call-1".to_string()),
                tool_name: Some("bash".to_string()),
                is_error: Some(false),
                duration_ms: Some(150.0),
                extra: Default::default(),
            }]),
            data: None,
            head: None,
            by_task_id: Some(10),
            extra: Default::default(),
        }),
        result: None,
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

    let select_req = ExtensionUiRequest::Select {
        id: "ui-sel".to_string(),
        title: "Pick option".to_string(),
        options: vec!["A".to_string(), "B".to_string()],
        timeout: None,
    };
    let _ = controller.update(PiAction::UiRequest(select_req));

    assert_eq!(controller.pending_dialogs_count(), 1);
    let _ = controller.update(PiAction::DialogSelectOption("ui-sel".to_string(), 1));

    let _ = controller.update(PiAction::RespondUi(ExtensionUiResponse::value(
        "ui-sel", "B",
    )));
    assert_eq!(controller.pending_dialogs_count(), 0);

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
async fn test_controller_agent_changed_and_inbox_update() {
    let (mut controller, _) = create_mock_controller();

    let _ = controller.update(PiAction::Event(PiEvent::AgentChanged {
        agent: AgentState {
            model: Some(ModelRef::new("anthropic", "claude-sonnet-4-20250514")),
            thinking_level: Some(ThinkingLevel::High),
            cwd: None,
            instructions: None,
            extra: Default::default(),
        },
    }));
    assert_eq!(
        controller.model(),
        Some("anthropic/claude-sonnet-4-20250514")
    );
    assert_eq!(controller.thinking_level(), Some(ThinkingLevel::High));

    let _ = controller.update(PiAction::Event(PiEvent::InboxUpdate {
        items: vec![
            QueuedItem {
                id: 1,
                mode: InboxMode::Steer,
                text: Some("Steer text".to_string()),
            },
            QueuedItem {
                id: 2,
                mode: InboxMode::FollowUp,
                text: Some("Follow up text".to_string()),
            },
        ],
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
        changes: vec![],
    }));

    // tokens_in should be input (1500) + cache_read (500) = 2000
    assert_eq!(controller.tokens_in(), 2000);
}

#[tokio::test]
async fn test_controller_slash_commands_help_and_unknown() {
    let (mut controller, _) = create_mock_controller();

    let _ = controller.submit_prompt("/help");

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

    let _ = controller.submit_prompt("/invalidcmd");
    assert_eq!(controller.history().len(), 4);
    assert_eq!(controller.history()[2].text(), "/invalidcmd");
    assert_eq!(controller.history()[3].role, "system");
    assert!(controller.history()[3]
        .text()
        .contains("Unknown command `/invalidcmd`"));
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
    assert!(all
        .iter()
        .any(|c| c.name == "antigravity.usage" && !c.is_builtin));

    let help = controller.format_help_markdown();
    assert!(help.contains("/skill:but"));
    assert!(help.contains("/antigravity.usage"));
}

#[tokio::test]
async fn test_controller_extension_ui_notify_and_prompt_handled() {
    let (mut controller, _) = create_mock_controller();

    let notify_req = ExtensionUiRequest::Notify {
        id: Some("notif-1".to_string()),
        message: "radius: connected, 123 tools (codemode)".to_string(),
        notify_type: Some(pi_rpc::protocol::NotifyType::Info),
    };
    let _ = controller.update(PiAction::UiRequest(notify_req));

    assert_eq!(controller.history().len(), 1);
    assert_eq!(controller.history()[0].role, "system");
    assert_eq!(
        controller.history()[0].text(),
        "radius: connected, 123 tools (codemode)"
    );

    assert_eq!(*controller.status(), AgentStatus::Idle);
    assert!(controller.view_active_dialog().is_none());
    assert_eq!(controller.pending_dialogs_count(), 0);

    let _ = controller.submit_prompt("/mcp");
    assert_eq!(*controller.status(), AgentStatus::Thinking);

    let _ = controller.update(PiAction::PromptCompleted(Ok(
        pi_rpc::protocol::PromptDisposition::Handled,
    )));

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
        message: AgentMessage {
            role: "user".to_string(),
            content: json!("Build a web server"),
            timestamp: Some(100),
            usage: None,
            tool_call_id: None,
            tool_name: None,
            is_error: None,
            duration_ms: None,
            extra: Default::default(),
        },
    }));
    let _ = controller.update(PiAction::Event(PiEvent::MessageEnd {
        entry: EntryRecord {
            id: 1,
            conversation_id: 1,
            kind: "pi.user".to_string(),
            model: Some(vec![AgentMessage {
                role: "user".to_string(),
                content: json!("Build a web server"),
                timestamp: Some(101),
                usage: None,
                tool_call_id: None,
                tool_name: None,
                is_error: None,
                duration_ms: None,
                extra: Default::default(),
            }]),
            data: None,
            head: None,
            by_task_id: None,
            extra: Default::default(),
        },
    }));
    // Should NOT duplicate initial prompt
    assert_eq!(controller.history().len(), 1);

    // Assistant response begins
    let _ = controller.update(PiAction::Event(PiEvent::MessageStart {
        message: AgentMessage {
            role: "assistant".to_string(),
            content: json!([]),
            timestamp: Some(102),
            usage: None,
            tool_call_id: None,
            tool_name: None,
            is_error: None,
            duration_ms: None,
            extra: Default::default(),
        },
    }));
    let _ = controller.update(PiAction::Event(PiEvent::MessageUpdate {
        usage: None,
        changes: vec![
            MessageChange::TextStart {
                content_index: 0,
                block: json!({ "type": "text", "text": "" }),
            },
            MessageChange::TextDelta {
                content_index: 0,
                delta: "Starting server in Node.js...".to_string(),
            },
        ],
    }));
    let _ = controller.update(PiAction::Event(PiEvent::MessageEnd {
        entry: EntryRecord {
            id: 2,
            conversation_id: 1,
            kind: "pi.assistant".to_string(),
            model: Some(vec![AgentMessage {
                role: "assistant".to_string(),
                content: json!([{ "type": "text", "text": "Starting server in Node.js..." }]),
                timestamp: Some(103),
                usage: None,
                tool_call_id: None,
                tool_name: None,
                is_error: None,
                duration_ms: None,
                extra: Default::default(),
            }]),
            data: None,
            head: None,
            by_task_id: None,
            extra: Default::default(),
        },
    }));
    let _ = controller.update(PiAction::Event(PiEvent::TurnEnd));

    assert_eq!(controller.history().len(), 2);
    assert_eq!(controller.history()[1].role, "assistant");
    assert_eq!(controller.history()[1].text(), "Starting server in Node.js...");

    // 3. User sends a Steer while busy
    let _ = controller.update(PiAction::Event(PiEvent::InboxUpdate {
        items: vec![QueuedItem {
            id: 2,
            mode: InboxMode::Steer,
            text: Some("Use Rust instead".to_string()),
        }],
    }));
    assert_eq!(controller.steering_queue(), &["Use Rust instead"]);

    // 4. Steer turn starts
    let _ = controller.update(PiAction::Event(PiEvent::TurnStart));
    let _ = controller.update(PiAction::Event(PiEvent::MessageStart {
        message: AgentMessage {
            role: "user".to_string(),
            content: json!([{"type": "text", "text": "Use Rust instead"}]),
            timestamp: Some(200),
            usage: None,
            tool_call_id: None,
            tool_name: None,
            is_error: None,
            duration_ms: None,
            extra: Default::default(),
        },
    }));
    let _ = controller.update(PiAction::Event(PiEvent::MessageEnd {
        entry: EntryRecord {
            id: 3,
            conversation_id: 1,
            kind: "pi.user".to_string(),
            model: Some(vec![AgentMessage {
                role: "user".to_string(),
                content: json!([{"type": "text", "text": "Use Rust instead"}]),
                timestamp: Some(201),
                usage: None,
                tool_call_id: None,
                tool_name: None,
                is_error: None,
                duration_ms: None,
                extra: Default::default(),
            }]),
            data: None,
            head: None,
            by_task_id: None,
            extra: Default::default(),
        },
    }));

    assert_eq!(controller.history().len(), 3);
    assert_eq!(controller.history()[2].role, "user");
    assert_eq!(controller.history()[2].text(), "Use Rust instead");

    // 5. Assistant responds to steer
    let _ = controller.update(PiAction::Event(PiEvent::MessageStart {
        message: AgentMessage {
            role: "assistant".to_string(),
            content: json!([]),
            timestamp: Some(202),
            usage: None,
            tool_call_id: None,
            tool_name: None,
            is_error: None,
            duration_ms: None,
            extra: Default::default(),
        },
    }));
    let _ = controller.update(PiAction::Event(PiEvent::MessageUpdate {
        usage: None,
        changes: vec![
            MessageChange::TextStart {
                content_index: 0,
                block: json!({ "type": "text", "text": "" }),
            },
            MessageChange::TextDelta {
                content_index: 0,
                delta: "Switching to Rust axum...".to_string(),
            },
        ],
    }));
    let _ = controller.update(PiAction::Event(PiEvent::MessageEnd {
        entry: EntryRecord {
            id: 4,
            conversation_id: 1,
            kind: "pi.assistant".to_string(),
            model: Some(vec![AgentMessage {
                role: "assistant".to_string(),
                content: json!([{ "type": "text", "text": "Switching to Rust axum..." }]),
                timestamp: Some(203),
                usage: None,
                tool_call_id: None,
                tool_name: None,
                is_error: None,
                duration_ms: None,
                extra: Default::default(),
            }]),
            data: None,
            head: None,
            by_task_id: None,
            extra: Default::default(),
        },
    }));
    let _ = controller.update(PiAction::Event(PiEvent::TurnEnd));

    assert_eq!(controller.history().len(), 4);
    assert_eq!(controller.history()[0].role, "user");
    assert_eq!(controller.history()[0].text(), "Build a web server");
    assert_eq!(controller.history()[1].role, "assistant");
    assert_eq!(controller.history()[1].text(), "Starting server in Node.js...");
    assert_eq!(controller.history()[2].role, "user");
    assert_eq!(controller.history()[2].text(), "Use Rust instead");
    assert_eq!(controller.history()[3].role, "assistant");
    assert_eq!(controller.history()[3].text(), "Switching to Rust axum...");

    // Run ends
    let _ = controller.update(PiAction::Event(PiEvent::RunEnd { inputs: vec![1, 2] }));
    assert_eq!(*controller.status(), AgentStatus::Idle);
}

#[tokio::test]
async fn test_controller_empty_assistant_message_cleaned_up_on_run_end() {
    let (mut controller, _) = create_mock_controller();

    let _ = controller.update(PiAction::Event(PiEvent::TurnStart));
    assert_eq!(controller.history().len(), 1);
    assert_eq!(controller.history()[0].role, "assistant");
    assert!(controller.history()[0].blocks.is_empty());

    let _ = controller.update(PiAction::Event(PiEvent::RunEnd { inputs: vec![1] }));

    assert!(controller.history().is_empty());
}

#[tokio::test]
async fn test_controller_surfaces_assistant_model_error_and_unanswered_submission() {
    let (mut controller, _) = create_mock_controller();

    let _ = controller.submit_prompt("Is this running Pi Durable?");
    let _ = controller.update(PiAction::Event(PiEvent::RunStart { inputs: vec![8] }));
    let _ = controller.update(PiAction::Event(PiEvent::TurnStart));

    let mut extra = std::collections::HashMap::new();
    extra.insert("stopReason".to_string(), json!("error"));
    extra.insert(
        "errorMessage".to_string(),
        json!("403 Forbidden: Radius has updated its Terms of Service."),
    );

    let err_assistant_msg = AgentMessage {
        role: "assistant".to_string(),
        content: json!([]),
        timestamp: Some(1000),
        usage: None,
        tool_call_id: None,
        tool_name: None,
        is_error: None,
        duration_ms: Some(386.0),
        extra,
    };

    let _ = controller.update(PiAction::Event(PiEvent::MessageStart {
        message: err_assistant_msg.clone(),
    }));
    let _ = controller.update(PiAction::Event(PiEvent::MessageEnd {
        entry: EntryRecord {
            id: 11,
            conversation_id: 1,
            kind: "pi.assistant".to_string(),
            model: Some(vec![err_assistant_msg]),
            data: None,
            head: None,
            by_task_id: Some(9),
            extra: Default::default(),
        },
    }));
    let _ = controller.update(PiAction::Event(PiEvent::TurnEnd));
    let _ = controller.update(PiAction::Event(PiEvent::RunEnd { inputs: vec![8] }));
    let _ = controller.update(PiAction::Event(PiEvent::Submission {
        record: serde_json::from_value(json!({
            "conversationId": 1,
            "type": "input",
            "status": "unanswered",
            "entry": 7,
            "id": 8,
            "reason": "model_error",
            "detail": "403 Forbidden: Radius has updated its Terms of Service."
        }))
        .unwrap(),
    }));

    assert_eq!(controller.history().len(), 2);
    assert_eq!(controller.history()[0].role, "user");
    assert_eq!(controller.history()[1].role, "assistant");
    assert!(controller.history()[1]
        .text()
        .contains("403 Forbidden: Radius has updated its Terms of Service."));
    assert!(matches!(
        controller.status(),
        AgentStatus::Error(msg) if msg.contains("403 Forbidden")
    ));
}

#[tokio::test]
async fn test_controller_multi_turn_tool_and_final_text_preserves_all_turns() {
    let (mut controller, _) = create_mock_controller();

    let _ = controller.submit_prompt("Check git log and answer");
    let _ = controller.update(PiAction::Event(PiEvent::RunStart { inputs: vec![1] }));

    // Turn 1: toolCall + empty text block (as emitted by Gemini/Antigravity)
    let _ = controller.update(PiAction::Event(PiEvent::TurnStart));
    let turn1_msg = AgentMessage {
        role: "assistant".to_string(),
        content: json!([
            {
                "type": "toolCall",
                "id": "call_1",
                "name": "bash",
                "arguments": { "command": "git log -n 1" }
            },
            {
                "type": "text",
                "text": ""
            }
        ]),
        timestamp: Some(100),
        usage: None,
        tool_call_id: None,
        tool_name: None,
        is_error: None,
        duration_ms: Some(200.0),
        extra: Default::default(),
    };
    let _ = controller.update(PiAction::Event(PiEvent::MessageStart {
        message: turn1_msg.clone(),
    }));
    let _ = controller.update(PiAction::Event(PiEvent::MessageEnd {
        entry: EntryRecord {
            id: 2,
            conversation_id: 1,
            kind: "pi.assistant".to_string(),
            model: Some(vec![turn1_msg]),
            data: None,
            head: None,
            by_task_id: Some(1),
            extra: Default::default(),
        },
    }));
    let _ = controller.update(PiAction::Event(PiEvent::ToolExecutionEnd {
        tool_call_id: "call_1".to_string(),
        tool_name: "bash".to_string(),
        task_id: Some(3),
        parent_tool_call_id: None,
        parent_task_id: None,
        entry: Some(EntryRecord {
            id: 4,
            conversation_id: 1,
            kind: "pi.tool-result".to_string(),
            model: Some(vec![AgentMessage {
                role: "toolResult".to_string(),
                content: json!([{ "type": "text", "text": "a1e1a76 Initial\n" }]),
                timestamp: Some(150),
                usage: None,
                tool_call_id: Some("call_1".to_string()),
                tool_name: Some("bash".to_string()),
                is_error: Some(false),
                duration_ms: Some(10.0),
                extra: Default::default(),
            }]),
            data: None,
            head: None,
            by_task_id: Some(3),
            extra: Default::default(),
        }),
        result: None,
    }));
    let _ = controller.update(PiAction::Event(PiEvent::TurnEnd));

    // Turn 2: Thinking + Text via MessageStart / MessageChange::Message / MessageEnd
    let _ = controller.update(PiAction::Event(PiEvent::TurnStart));
    let turn2_msg = AgentMessage {
        role: "assistant".to_string(),
        content: json!([
            {
                "type": "thinking",
                "thinking": "Checked the git log."
            },
            {
                "type": "text",
                "text": "Yes, I am running inside Pi Durable."
            }
        ]),
        timestamp: Some(200),
        usage: None,
        tool_call_id: None,
        tool_name: None,
        is_error: None,
        duration_ms: Some(300.0),
        extra: Default::default(),
    };
    let _ = controller.update(PiAction::Event(PiEvent::MessageStart {
        message: turn2_msg.clone(),
    }));
    let _ = controller.update(PiAction::Event(PiEvent::MessageUpdate {
        usage: None,
        changes: vec![MessageChange::Message {
            message: turn2_msg.clone(),
        }],
    }));
    let _ = controller.update(PiAction::Event(PiEvent::MessageEnd {
        entry: EntryRecord {
            id: 5,
            conversation_id: 1,
            kind: "pi.assistant".to_string(),
            model: Some(vec![turn2_msg]),
            data: None,
            head: None,
            by_task_id: Some(1),
            extra: Default::default(),
        },
    }));
    let _ = controller.update(PiAction::Event(PiEvent::TurnEnd));
    let _ = controller.update(PiAction::Event(PiEvent::RunEnd { inputs: vec![1] }));

    assert_eq!(controller.history().len(), 2);
    let assistant = &controller.history()[1];
    assert_eq!(assistant.blocks.len(), 3);
    assert!(matches!(&assistant.blocks[0], MessageBlock::ToolCall(t) if t.id == "call_1"));
    assert!(matches!(&assistant.blocks[1], MessageBlock::Thinking { text, .. } if text == "Checked the git log."));
    assert_eq!(assistant.text(), "Yes, I am running inside Pi Durable.");
}

