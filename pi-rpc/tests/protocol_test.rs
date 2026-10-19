use pi_rpc::protocol::*;
use serde_json::json;

#[test]
fn test_command_prompt_serialization() {
    let cmd = RpcCommand::Prompt {
        id: Some("req-1".into()),
        message: "Hello, Pi!".into(),
        images: Some(vec![ImageContent::new("image/png", "base64data==")]),
        streaming_behavior: Some(StreamingBehavior::Steer),
    };

    let serialized = serde_json::to_string(&cmd).unwrap();
    let val: serde_json::Value = serde_json::from_str(&serialized).unwrap();

    assert_eq!(val["type"], "prompt");
    assert_eq!(val["id"], "req-1");
    assert_eq!(val["message"], "Hello, Pi!");
    assert_eq!(val["streamingBehavior"], "steer");
    assert_eq!(val["images"][0]["mimeType"], "image/png");
    assert_eq!(val["images"][0]["data"], "base64data==");
}

#[test]
fn test_command_steer_and_follow_up_serialization() {
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
        "command": "prompt",
        "success": true,
        "data": {
            "disposition": "started"
        }
    });

    let record = IncomingRecord::parse(&serde_json::to_vec(&raw_success).unwrap()).unwrap();
    if let IncomingRecord::Response(resp) = record {
        assert_eq!(resp.id, Some("req-10".into()));
        assert_eq!(resp.command, "prompt");
        assert!(resp.success);
        let data: PromptData = serde_json::from_value(resp.data.unwrap()).unwrap();
        assert_eq!(data.disposition, PromptDisposition::Started);
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
fn test_session_state_response_deserialization() {
    let state_json = json!({
        "id": "test-state",
        "type": "response",
        "command": "get_state",
        "success": true,
        "data": {
            "model": {
                "id": "balanced",
                "name": "Balanced",
                "reasoning": true,
                "provider": "radius"
            },
            "thinkingLevel": "medium",
            "isStreaming": false,
            "isCompacting": false,
            "steeringMode": "one-at-a-time",
            "followUpMode": "all",
            "sessionId": "session-12345",
            "sessionName": "Feature Branch Work",
            "autoCompactionEnabled": true,
            "messageCount": 12,
            "pendingMessageCount": 1
        }
    });

    let record = IncomingRecord::parse(&serde_json::to_vec(&state_json).unwrap()).unwrap();
    if let IncomingRecord::Response(resp) = record {
        let state: RpcSessionState = serde_json::from_value(resp.data.unwrap()).unwrap();
        assert_eq!(state.session_id, "session-12345");
        assert_eq!(state.session_name, Some("Feature Branch Work".into()));
        assert_eq!(state.thinking_level, ThinkingLevel::Medium);
        assert_eq!(state.steering_mode, QueueMode::OneAtATime);
        assert_eq!(state.follow_up_mode, QueueMode::All);
        assert_eq!(state.message_count, 12);
        assert_eq!(state.pending_message_count, 1);
        let model = state.model.unwrap();
        assert_eq!(model.id, "balanced");
        assert_eq!(model.name, "Balanced");
        assert_eq!(model.reasoning, Some(true));
    } else {
        panic!("Expected IncomingRecord::Response");
    }
}

#[test]
fn test_streaming_session_events() {
    // 1. message_update event with text_delta
    let update_json = json!({
        "type": "message_update",
        "usage": {
            "input": 100,
            "output": 15,
            "cacheRead": 0,
            "cacheWrite": 0,
            "totalTokens": 115,
            "cost": {
                "input": 0.001,
                "output": 0.002,
                "cacheRead": 0.0,
                "cacheWrite": 0.0,
                "total": 0.003
            }
        },
        "assistantMessageEvent": {
            "type": "text_delta",
            "contentIndex": 0,
            "delta": "Hello from Pi assistant!"
        }
    });

    let record = IncomingRecord::parse(&serde_json::to_vec(&update_json).unwrap()).unwrap();
    if let IncomingRecord::Event(PiEvent::MessageUpdate {
        usage,
        assistant_message_event,
    }) = record
    {
        assert_eq!(usage.unwrap().total_tokens, 115);
        if let AssistantMessageEvent::TextDelta { content_index, delta } = assistant_message_event {
            assert_eq!(content_index, 0);
            assert_eq!(delta, "Hello from Pi assistant!");
        } else {
            panic!("Expected TextDelta");
        }
    } else {
        panic!("Expected PiEvent::MessageUpdate");
    }

    // 2. agent_settled event
    let settled_json = json!({
        "type": "agent_settled",
        "aborted": false
    });
    let settled_record = IncomingRecord::parse(&serde_json::to_vec(&settled_json).unwrap()).unwrap();
    assert_eq!(
        settled_record,
        IncomingRecord::Event(PiEvent::AgentSettled { aborted: false })
    );

    // 3. tool_execution_start and end events
    let tool_start = json!({
        "type": "tool_execution_start",
        "toolCallId": "call-1",
        "toolName": "bash",
        "args": { "command": "cargo check" }
    });
    let tool_record = IncomingRecord::parse(&serde_json::to_vec(&tool_start).unwrap()).unwrap();
    if let IncomingRecord::Event(PiEvent::ToolExecutionStart {
        tool_call_id,
        tool_name,
        args,
    }) = tool_record
    {
        assert_eq!(tool_call_id, "call-1");
        assert_eq!(tool_name, "bash");
        assert_eq!(args["command"], "cargo check");
    } else {
        panic!("Expected ToolExecutionStart");
    }
}

#[test]
fn test_extension_ui_requests_and_responses() {
    // Select request
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
        if let ExtensionUiRequest::Select {
            title,
            options,
            timeout,
            ..
        } = req
        {
            assert_eq!(title, "Allow dangerous command?");
            assert_eq!(options, vec!["Allow", "Block"]);
            assert_eq!(timeout, Some(10000));
        } else {
            panic!("Expected ExtensionUiRequest::Select");
        }
    } else {
        panic!("Expected IncomingRecord::ExtensionUi");
    }

    // Confirm request
    let confirm_json = json!({
        "type": "extension_ui_request",
        "id": "uuid-dialog-2",
        "method": "confirm",
        "title": "Clear session?",
        "message": "All messages will be lost.",
        "timeout": 5000
    });
    let confirm_record = IncomingRecord::parse(&serde_json::to_vec(&confirm_json).unwrap()).unwrap();
    if let IncomingRecord::ExtensionUi(ExtensionUiRequest::Confirm {
        id,
        title,
        message,
        timeout,
    }) = confirm_record
    {
        assert_eq!(id, "uuid-dialog-2");
        assert_eq!(title, "Clear session?");
        assert_eq!(message, "All messages will be lost.");
        assert_eq!(timeout, Some(5000));
    } else {
        panic!("Expected ExtensionUiRequest::Confirm");
    }

    // Fire-and-forget notification
    let notify_json = json!({
        "type": "extension_ui_request",
        "id": "uuid-notify-1",
        "method": "notify",
        "message": "Command blocked",
        "notifyType": "warning"
    });
    let notify_record = IncomingRecord::parse(&serde_json::to_vec(&notify_json).unwrap()).unwrap();
    if let IncomingRecord::ExtensionUi(req) = notify_record {
        assert!(!req.is_dialog());
        if let ExtensionUiRequest::Notify {
            message,
            notify_type,
            ..
        } = req
        {
            assert_eq!(message, "Command blocked");
            assert_eq!(notify_type, Some(NotifyType::Warning));
        } else {
            panic!("Expected Notify");
        }
    }

    // Responses
    let val_resp = ExtensionUiResponse::value("uuid-dialog-1", "Allow");
    let val_json: serde_json::Value = serde_json::to_value(&val_resp).unwrap();
    assert_eq!(val_json["type"], "extension_ui_response");
    assert_eq!(val_json["id"], "uuid-dialog-1");
    assert_eq!(val_json["value"], "Allow");

    let conf_resp = ExtensionUiResponse::confirm("uuid-dialog-2", true);
    let conf_json: serde_json::Value = serde_json::to_value(&conf_resp).unwrap();
    assert_eq!(conf_json["confirmed"], true);

    let cancel_resp = ExtensionUiResponse::cancel("uuid-dialog-3");
    let cancel_json: serde_json::Value = serde_json::to_value(&cancel_resp).unwrap();
    assert_eq!(cancel_json["cancelled"], true);
}

#[test]
fn test_agent_message_text_extraction() {
    let str_msg = AgentMessage {
        role: "user".into(),
        content: serde_json::Value::String("Plain user prompt".into()),
        timestamp: Some(1733234401000),
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
        extra: Default::default(),
    };
    assert_eq!(block_msg.text(), "Part 1. Part 2.");
}
