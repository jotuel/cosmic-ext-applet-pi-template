use cosmic_ext_pi::controller::{AgentStatus, ChatMessage, MessageBlock, ToolCallData};
use cosmic_ext_pi::dialogs::{ConfirmDialog, EditorDialog, InputDialog, SelectDialog};
use cosmic_ext_pi::widget::{ChatView, StatusBadge, ThinkingBlock, ToolCard};
use serde_json::json;

#[test]
fn test_status_badge_variants() {
    let _ = StatusBadge::view(&AgentStatus::Idle);
    let _ = StatusBadge::view(&AgentStatus::Thinking);
    let _ = StatusBadge::view(&AgentStatus::ExecutingTool {
        name: "bash".to_string(),
        tool_call_id: "c1".to_string(),
    });
    let _ = StatusBadge::view(&AgentStatus::AwaitingUser {
        request_id: "req1".to_string(),
    });
    let _ = StatusBadge::view(&AgentStatus::Error("Failure".to_string()));
}

#[test]
fn test_thinking_block_variants() {
    // Collapsed and non-collapsed
    let _ = ThinkingBlock::view(0, 0, "Thinking deeply...", true, false);
    let _ = ThinkingBlock::view(0, 0, "Thinking deeply...", false, false);
    // Live streaming thinking
    let _ = ThinkingBlock::view(0, 0, "Thinking...", false, true);
}

#[test]
fn test_tool_card_variants() {
    let args = json!({ "path": "src/main.rs" });
    let result = json!("File contents...");

    // Running
    let _ = ToolCard::view(
        "t1", "read", &args, "", None, None, false, None, false, true,
    );

    // Completed
    let _ = ToolCard::view(
        "t1",
        "read",
        &args,
        "",
        None,
        Some(&result),
        false,
        Some(42.0),
        false,
        false,
    );

    // Failed
    let _ = ToolCard::view(
        "t1",
        "read",
        &args,
        "",
        None,
        Some(&json!("Not found")),
        true,
        Some(12.0),
        false,
        false,
    );
}

#[test]
fn test_dialog_views() {
    let _ = ConfirmDialog::view("d1", "Confirm Action", "Do you want to proceed?");

    let options = vec!["Option 1".to_string(), "Option 2".to_string()];
    let _ = SelectDialog::view("d2", "Select Option", &options, Some(0));

    let _ = InputDialog::view("d3", "Input", Some("placeholder"), "current value");

    let content = cosmic::iced::widget::text_editor::Content::with_text("fn main() {}");
    let _ = EditorDialog::view("d4", "Editor", &content);
}

#[test]
fn test_chat_view_rendering() {
    let user_msg = ChatMessage::user("Hello Pi");
    let mut assistant_msg = ChatMessage::assistant();
    assistant_msg.blocks.push(MessageBlock::Thinking {
        text: "Thinking...".to_string(),
        collapsed: false,
        is_streaming: false,
    });
    assistant_msg
        .blocks
        .push(MessageBlock::ToolCall(ToolCallData {
            id: "c1".to_string(),
            name: "bash".to_string(),
            args: json!({ "command": "echo hi" }),
            args_raw: "".to_string(),
            partial_result: None,
            result: Some(json!("hi")),
            is_error: false,
            duration_ms: Some(10.0),
            collapsed: false,
            is_executing: false,
        }));
    assistant_msg.blocks.push(MessageBlock::Text {
        text: "Here is your response.".to_string(),
        parsed: cosmic::iced::widget::markdown::parse("Here is your response.").collect(),
    });

    let system_msg = ChatMessage::system("Session started");

    let history = vec![system_msg, user_msg, assistant_msg];
    let _ = ChatView::view(&history);
    let _ = ChatView::view_ordered(&history, true);
    let _ = ChatView::view_ordered(&history, false);
}
