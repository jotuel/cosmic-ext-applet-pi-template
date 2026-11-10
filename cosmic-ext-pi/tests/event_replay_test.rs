use std::sync::Arc;
use tokio::io::duplex;

use cosmic_ext_pi::controller::{AgentStatus, MessageBlock, PiSessionController};
use cosmic_ext_pi::subscription::PiAction;
use pi_rpc::protocol::PiEvent;
use pi_rpc::PiClient;

const RECORDED_SESSION_JSONL: &str = r#"
{"type":"snapshot","entries":[],"tools":[],"nestedTools":[],"compactions":[],"inbox":[],"agent":{"model":{"provider":"radius","modelId":"balanced"},"thinkingLevel":"medium"},"usage":{"models":{},"tools":{}}}
{"type":"run_start","inputs":[1]}
{"type":"turn_start"}
{"type":"message_start","message":{"role":"assistant","content":[],"timestamp":1700000000}}
{"type":"message_update","changes":[{"type":"thinking_start","contentIndex":0,"block":{"type":"thinking","thinking":""}},{"type":"thinking_delta","contentIndex":0,"delta":"User wants to list files. I will use the bash tool."},{"type":"toolcall_start","contentIndex":1,"block":{"type":"toolCall","id":"call_ls_123","name":"bash","arguments":{"command":"ls -la"}}}]}
{"type":"message_end","entry":{"id":1,"conversationId":1,"kind":"pi.assistant","model":[{"role":"assistant","content":[{"type":"thinking","thinking":"User wants to list files. I will use the bash tool."},{"type":"toolCall","id":"call_ls_123","name":"bash","arguments":{"command":"ls -la"}}],"timestamp":1700000001}]}}
{"type":"tool_execution_start","toolCallId":"call_ls_123","toolName":"bash","taskId":10,"args":{"command":"ls -la"}}
{"type":"tool_execution_update","toolCallId":"call_ls_123","toolName":"bash","taskId":10,"output":{"append":"total 8\n"}}
{"type":"tool_execution_end","toolCallId":"call_ls_123","toolName":"bash","taskId":10,"entry":{"id":2,"conversationId":1,"kind":"pi.tool-result","model":[{"role":"toolResult","toolCallId":"call_ls_123","toolName":"bash","content":[{"type":"text","text":"total 8\n-rw-r--r-- Cargo.toml\n-rw-r--r-- src/lib.rs"}],"isError":false,"durationMs":42.5,"timestamp":1700000002}]}}
{"type":"message_update","changes":[{"type":"text_start","contentIndex":2,"block":{"type":"text","text":""}},{"type":"text_delta","contentIndex":2,"delta":"Here are the files in the project:\n\n"},{"type":"text_delta","contentIndex":2,"delta":"- `Cargo.toml`\n- `src/lib.rs`\n"}]}
{"type":"message_end","entry":{"id":3,"conversationId":1,"kind":"pi.assistant","model":[{"role":"assistant","content":[{"type":"thinking","thinking":"User wants to list files. I will use the bash tool."},{"type":"toolCall","id":"call_ls_123","name":"bash","arguments":{"command":"ls -la"}},{"type":"text","text":"Here are the files in the project:\n\n- `Cargo.toml`\n- `src/lib.rs`\n"}],"timestamp":1700000005}]}}
{"type":"turn_end"}
{"type":"run_end","inputs":[1]}
"#;

#[tokio::test]
async fn test_replay_recorded_jsonl_events() {
    let (client_stdin, _server_stdin) = duplex(4096);
    let (_server_stdout, client_stdout) = duplex(4096);
    let client = Arc::new(PiClient::from_io(
        Box::new(client_stdin),
        Box::new(client_stdout),
        None,
    ));

    let mut controller = PiSessionController::new(client);

    // Replay each line
    for line in RECORDED_SESSION_JSONL.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let event: PiEvent = serde_json::from_str(line)
            .unwrap_or_else(|e| panic!("Failed to parse line {line}: {e}"));

        let _ = controller.update(PiAction::Event(event));
    }

    // Verify session reconstructed correctly
    assert_eq!(*controller.status(), AgentStatus::Idle);
    assert_eq!(controller.history().len(), 1);

    let assistant_msg = &controller.history()[0];
    assert_eq!(assistant_msg.role, "assistant");
    assert!(!assistant_msg.is_streaming);
    assert_eq!(assistant_msg.blocks.len(), 3);

    // 1. Thinking block
    match &assistant_msg.blocks[0] {
        MessageBlock::Thinking {
            text,
            collapsed,
            is_streaming,
        } => {
            assert_eq!(text, "User wants to list files. I will use the bash tool.");
            assert!(collapsed);
            assert!(!is_streaming);
        }
        other => panic!("Expected MessageBlock::Thinking, got {:?}", other),
    }

    // 2. Tool call block
    match &assistant_msg.blocks[1] {
        MessageBlock::ToolCall(tool) => {
            assert_eq!(tool.id, "call_ls_123");
            assert_eq!(tool.name, "bash");
            assert!(!tool.is_error);
            assert!(!tool.is_executing);
            assert!(tool.collapsed);
            assert_eq!(tool.duration_ms, Some(42.5));
            assert_eq!(
                tool.result,
                Some(serde_json::json!(
                    "total 8\n-rw-r--r-- Cargo.toml\n-rw-r--r-- src/lib.rs"
                ))
            );
        }
        other => panic!("Expected MessageBlock::ToolCall, got {:?}", other),
    }

    // 3. Text block
    match &assistant_msg.blocks[2] {
        MessageBlock::Text { text, parsed } => {
            assert!(text.contains("Here are the files in the project:"));
            assert!(text.contains("Cargo.toml"));
            assert!(!parsed.is_empty()); // Markdown was parsed and cached!
        }
        other => panic!("Expected MessageBlock::Text, got {:?}", other),
    }

    // Verify view generation
    let _ = controller.view_chat();
    let _ = controller.view_status_badge();
    assert!(controller.view_active_dialog().is_none());
}
