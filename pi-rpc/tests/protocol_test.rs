use pi_rpc::protocol::*;
use serde_json::json;

#[test]
fn test_command_submit_serialization() {
    let cmd = RpcCommand::Submit {
        id: Some("req-1".into()),
        message: "Hello, Pi Durable!".into(),
        images: Some(vec![ImageContent::new("image/png", "base64data==")]),
        when_busy: Some(StreamingBehavior::Steer),
        request_id: Some("req-id-1".into()),
    };

    let serialized = serde_json::to_string(&cmd).unwrap();
    let val: serde_json::Value = serde_json::from_str(&serialized).unwrap();

    assert_eq!(val["type"], "submit");
    assert_eq!(val["id"], "req-1");
    assert_eq!(val["message"], "Hello, Pi Durable!");
    assert_eq!(val["whenBusy"], "steer");
    assert_eq!(val["requestId"], "req-id-1");
    assert_eq!(val["images"][0]["mimeType"], "image/png");
    assert_eq!(val["images"][0]["data"], "base64data==");
}

#[test]
fn test_command_steer_follow_up_and_reset_serialization() {
    let steer = RpcCommand::Steer {
        id: Some("s-1".into()),
        message: "Steer now".into(),
        images: None,
    };
    let s_val: serde_json::Value = serde_json::to_value(&steer).unwrap();
    assert_eq!(s_val["type"], "steer");
    assert_eq!(s_val["message"], "Steer now");

    let follow = RpcCommand::FollowUp {
        id: Some("f-1".into()),
        message: "Follow up after".into(),
        images: None,
    };
    let f_val: serde_json::Value = serde_json::to_value(&follow).unwrap();
    assert_eq!(f_val["type"], "follow_up");
    assert_eq!(f_val["message"], "Follow up after");

    let reset = RpcCommand::Reset {
        id: Some("r-1".into()),
        handoff: Some("Continue fixing test".into()),
    };
    let r_val: serde_json::Value = serde_json::to_value(&reset).unwrap();
    assert_eq!(r_val["type"], "reset");
    assert_eq!(r_val["handoff"], "Continue fixing test");
}

#[test]
fn test_command_set_model_and_thinking_level() {
    let set_model = RpcCommand::SetModel {
        id: Some("m-1".into()),
        provider: "anthropic".into(),
        model_id: "claude-sonnet-4-20250514".into(),
    };
    let val: serde_json::Value = serde_json::to_value(&set_model).unwrap();
    assert_eq!(val["type"], "set_model");
    assert_eq!(val["provider"], "anthropic");
    assert_eq!(val["modelId"], "claude-sonnet-4-20250514");

    let set_thinking = RpcCommand::SetThinkingLevel {
        id: Some("t-1".into()),
        level: ThinkingLevel::High,
    };
    let t_val: serde_json::Value = serde_json::to_value(&set_thinking).unwrap();
    assert_eq!(t_val["type"], "set_thinking_level");
    assert_eq!(t_val["level"], "high");
}

#[test]
fn test_response_parsing_success_and_error() {
    let raw_success = json!({
        "id": "req-10",
        "type": "response",
        "command": "submit",
        "success": true,
        "data": {
            "disposition": "started",
            "submissionId": 1,
            "status": "placed"
        }
    });

    let record = IncomingRecord::parse(&serde_json::to_vec(&raw_success).unwrap()).unwrap();
    if let IncomingRecord::Response(resp) = record {
        assert_eq!(resp.id, Some("req-10".into()));
        assert_eq!(resp.command, "submit");
        assert!(resp.success);
        let data: PromptData = serde_json::from_value(resp.data.unwrap()).unwrap();
        assert_eq!(data.disposition, PromptDisposition::Started);
        assert_eq!(data.submission_id, Some(1));
        assert_eq!(data.status.as_deref(), Some("placed"));
    } else {
        panic!("Expected IncomingRecord::Response");
    }

    let raw_error = json!({
        "id": "req-11",
        "type": "response",
        "command": "set_model",
        "success": false,
        "error": "Model not found: invalid/model"
    });

    let err_record = IncomingRecord::parse(&serde_json::to_vec(&raw_error).unwrap()).unwrap();
    if let IncomingRecord::Response(resp) = err_record {
        assert_eq!(resp.id, Some("req-11".into()));
        assert!(!resp.success);
        assert_eq!(resp.error, Some("Model not found: invalid/model".into()));
    } else {
        panic!("Expected IncomingRecord::Response");
    }
}

#[test]
fn test_snapshot_and_streaming_durable_events() {
    // 1. snapshot event
    let snap_json = json!({
        "type": "snapshot",
        "entries": [
            {
                "id": 1,
                "conversationId": 1,
                "kind": "pi.user",
                "model": [{ "role": "user", "content": "Hello durable!" }]
            },
            {
                "id": 2,
                "conversationId": 1,
                "kind": "pi.assistant",
                "model": [{
                    "role": "assistant",
                    "content": [
                        { "type": "thinking", "thinking": "Thinking..." },
                        { "type": "text", "text": "Hi there!" }
                    ]
                }]
            }
        ],
        "run": { "inputs": [1] },
        "tools": [
            {
                "callId": "call-1",
                "name": "bash",
                "status": "running",
                "output": "cargo test\n"
            }
        ],
        "nestedTools": [],
        "compactions": [],
        "inbox": [
            { "id": 2, "mode": "steer", "text": "Use release mode" }
        ],
        "agent": {
            "model": { "provider": "radius", "modelId": "balanced" },
            "thinkingLevel": "high",
            "cwd": "/tmp/project"
        },
        "usage": {
            "models": {
                "radius/balanced": {
                    "input": 200,
                    "output": 50,
                    "cacheRead": 100,
                    "cacheWrite": 0,
                    "totalTokens": 350
                }
            },
            "tools": {}
        }
    });

    let record = IncomingRecord::parse(&serde_json::to_vec(&snap_json).unwrap()).unwrap();
    if let IncomingRecord::Event(PiEvent::Snapshot(snap)) = record {
        assert_eq!(snap.entries.len(), 2);
        assert_eq!(snap.entries[0].kind, "pi.user");
        assert_eq!(snap.entries[1].kind, "pi.assistant");
        assert_eq!(snap.run.unwrap().inputs, vec![1]);
        assert_eq!(snap.tools.len(), 1);
        assert_eq!(snap.tools[0].call_id, "call-1");
        assert_eq!(snap.inbox.len(), 1);
        assert_eq!(snap.inbox[0].mode, InboxMode::Steer);
        assert_eq!(snap.inbox[0].text.as_deref(), Some("Use release mode"));
        assert_eq!(
            snap.agent.model,
            Some(ModelRef::new("radius", "balanced"))
        );
        assert_eq!(snap.agent.thinking_level, Some(ThinkingLevel::High));
        let total = snap.usage.total_usage();
        assert_eq!(total.input, 200);
        assert_eq!(total.cache_read, 100);
        assert_eq!(total.total_tokens, 350);
    } else {
        panic!("Expected PiEvent::Snapshot");
    }

    // 2. message_update with MessageChange[]
    let update_json = json!({
        "type": "message_update",
        "usage": {
            "input": 100,
            "output": 15,
            "cacheRead": 0,
            "cacheWrite": 0,
            "totalTokens": 115
        },
        "changes": [
            {
                "type": "text_start",
                "contentIndex": 0,
                "block": { "type": "text", "text": "Hello" }
            },
            {
                "type": "text_delta",
                "contentIndex": 0,
                "delta": " from Pi Durable!"
            }
        ]
    });

    let record = IncomingRecord::parse(&serde_json::to_vec(&update_json).unwrap()).unwrap();
    if let IncomingRecord::Event(PiEvent::MessageUpdate { usage, changes }) = record {
        assert_eq!(usage.unwrap().total_tokens, 115);
        assert_eq!(changes.len(), 2);
        if let MessageChange::TextDelta { content_index, delta } = &changes[1] {
            assert_eq!(*content_index, 0);
            assert_eq!(delta, " from Pi Durable!");
        } else {
            panic!("Expected MessageChange::TextDelta");
        }
    } else {
        panic!("Expected PiEvent::MessageUpdate");
    }

    // 3. run_end event
    let run_end_json = json!({
        "type": "run_end",
        "inputs": [1]
    });
    let run_end_record = IncomingRecord::parse(&serde_json::to_vec(&run_end_json).unwrap()).unwrap();
    assert_eq!(
        run_end_record,
        IncomingRecord::Event(PiEvent::RunEnd { inputs: vec![1] })
    );

    // 4. tool_execution_start, update, and end events
    let tool_start = json!({
        "type": "tool_execution_start",
        "toolCallId": "call-1",
        "toolName": "bash",
        "taskId": 10,
        "args": { "command": "cargo check" }
    });
    let tool_record = IncomingRecord::parse(&serde_json::to_vec(&tool_start).unwrap()).unwrap();
    if let IncomingRecord::Event(PiEvent::ToolExecutionStart {
        tool_call_id,
        tool_name,
        task_id,
        args,
        ..
    }) = tool_record
    {
        assert_eq!(tool_call_id, "call-1");
        assert_eq!(tool_name, "bash");
        assert_eq!(task_id, Some(10));
        assert_eq!(args["command"], "cargo check");
    } else {
        panic!("Expected ToolExecutionStart");
    }

    let tool_update = json!({
        "type": "tool_execution_update",
        "toolCallId": "call-1",
        "toolName": "bash",
        "output": { "append": "Checking pi-rpc...\n" }
    });
    let update_rec = IncomingRecord::parse(&serde_json::to_vec(&tool_update).unwrap()).unwrap();
    if let IncomingRecord::Event(PiEvent::ToolExecutionUpdate { output, .. }) = update_rec {
        assert_eq!(output.unwrap().append.as_deref(), Some("Checking pi-rpc...\n"));
    } else {
        panic!("Expected ToolExecutionUpdate");
    }
}

#[test]
fn test_extension_ui_requests_and_responses() {
    let select_json = json!({
        "type": "extension_ui_request",
        "id": "uuid-dialog-1",
        "method": "select",
        "title": "Allow dangerous command?",
        "options": ["Allow", "Block"],
        "timeout": 10000
    });

    let record = IncomingRecord::parse(&serde_json::to_vec(&select_json).unwrap()).unwrap();
    if let IncomingRecord::ExtensionUi(req) = record {
        assert_eq!(req.id(), Some("uuid-dialog-1"));
        assert!(req.is_dialog());
    } else {
        panic!("Expected IncomingRecord::ExtensionUi");
    }

    let val_resp = ExtensionUiResponse::value("uuid-dialog-1", "Allow");
    let val_json: serde_json::Value = serde_json::to_value(&val_resp).unwrap();
    assert_eq!(val_json["type"], "extension_ui_response");
    assert_eq!(val_json["id"], "uuid-dialog-1");
    assert_eq!(val_json["value"], "Allow");
}

#[test]
fn test_agent_message_text_extraction() {
    let str_msg = AgentMessage {
        role: "user".into(),
        content: serde_json::Value::String("Plain user prompt".into()),
        timestamp: Some(1733234401000),
        usage: None,
        tool_call_id: None,
        tool_name: None,
        is_error: None,
        duration_ms: None,
        extra: Default::default(),
    };
    assert_eq!(str_msg.text(), "Plain user prompt");

    let block_msg = AgentMessage {
        role: "assistant".into(),
        content: json!([
            { "type": "text", "text": "Part 1. " },
            { "type": "toolCall", "id": "t1", "name": "read" },
            { "type": "text", "text": "Part 2." }
        ]),
        timestamp: Some(1733234402000),
        usage: None,
        tool_call_id: None,
        tool_name: None,
        is_error: None,
        duration_ms: None,
        extra: Default::default(),
    };
    assert_eq!(block_msg.text(), "Part 1. Part 2.");
}
