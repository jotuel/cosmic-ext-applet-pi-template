use pi_rpc::error::PiError;
use pi_rpc::protocol::*;
use pi_rpc::transport::{JsonLinesReader, JsonLinesWriter};
use pi_rpc::PiClient;
use serde_json::json;
use tokio::io::duplex;

#[tokio::test]
async fn test_client_prompt_and_durable_events() {
    let (client_stdin, mut server_stdin) = duplex(4096);
    let (mut server_stdout, client_stdout) = duplex(4096);

    let client = PiClient::from_io(Box::new(client_stdin), Box::new(client_stdout), None);
    let mut event_rx = client.subscribe_events();

    let server_handle = tokio::spawn(async move {
        let mut reader = JsonLinesReader::new(&mut server_stdin);
        let mut writer = JsonLinesWriter::new(&mut server_stdout);

        // 1. Emit initial snapshot
        let snapshot_event = json!({
            "type": "snapshot",
            "entries": [],
            "tools": [],
            "nestedTools": [],
            "compactions": [],
            "inbox": [],
            "agent": {
                "model": { "provider": "radius", "modelId": "balanced" },
                "thinkingLevel": "medium"
            },
            "usage": { "models": {}, "tools": {} }
        });
        writer.write_record(&snapshot_event).await.unwrap();

        // 2. Expect submit command
        let cmd: RpcCommand = reader.read_record().await.unwrap().unwrap();
        let cmd_id = cmd.id().unwrap().to_string();

        if let RpcCommand::Submit { message, .. } = cmd {
            assert_eq!(message, "Write a test");
        } else {
            panic!("Expected submit command");
        }

        // 3. Send submit response
        let resp = json!({
            "id": cmd_id,
            "type": "response",
            "command": "submit",
            "success": true,
            "data": {
                "disposition": "started",
                "submissionId": 1,
                "status": "placed"
            }
        });
        writer.write_record(&resp).await.unwrap();

        // 4. Stream a message_update event with MessageChange::TextDelta
        let delta_event = json!({
            "type": "message_update",
            "changes": [
                {
                    "type": "text_delta",
                    "contentIndex": 0,
                    "delta": "Writing test..."
                }
            ]
        });
        writer.write_record(&delta_event).await.unwrap();

        // 5. Stream run_end event
        let run_end_event = json!({
            "type": "run_end",
            "inputs": [1]
        });
        writer.write_record(&run_end_event).await.unwrap();
    });

    let disposition = client.prompt("Write a test").await.unwrap();
    assert_eq!(disposition, PromptDisposition::Started);

    // Verify snapshot received and cached on client
    let event0 = event_rx.recv().await.unwrap();
    assert!(matches!(event0, PiEvent::Snapshot(_)));
    assert!(client.latest_snapshot().is_some());

    // Verify message_update received
    let event1 = event_rx.recv().await.unwrap();
    if let PiEvent::MessageUpdate { changes, .. } = event1 {
        assert_eq!(changes.len(), 1);
        if let MessageChange::TextDelta { delta, .. } = &changes[0] {
            assert_eq!(delta, "Writing test...");
        } else {
            panic!("Expected text delta");
        }
    } else {
        panic!("Expected MessageUpdate");
    }

    // Verify run_end received
    let event2 = event_rx.recv().await.unwrap();
    assert_eq!(event2, PiEvent::RunEnd { inputs: vec![1] });

    server_handle.await.unwrap();
}

#[tokio::test]
async fn test_client_correlation_out_of_order() {
    let (client_stdin, mut server_stdin) = duplex(4096);
    let (mut server_stdout, client_stdout) = duplex(4096);

    let client = PiClient::from_io(Box::new(client_stdin), Box::new(client_stdout), None);

    let server_handle = tokio::spawn(async move {
        let mut reader = JsonLinesReader::new(&mut server_stdin);
        let mut writer = JsonLinesWriter::new(&mut server_stdout);

        let cmd1: RpcCommand = reader.read_record().await.unwrap().unwrap();
        let cmd2: RpcCommand = reader.read_record().await.unwrap().unwrap();

        let id1 = cmd1.id().unwrap().to_string();
        let id2 = cmd2.id().unwrap().to_string();

        // Respond to cmd2 FIRST (out of order correlation test)
        let resp2 = json!({
            "id": id2,
            "type": "response",
            "command": "get_commands",
            "success": true,
            "data": {
                "commands": [
                    { "name": "review", "source": "skill" }
                ]
            }
        });
        writer.write_record(&resp2).await.unwrap();

        // Respond to cmd1 SECOND
        let resp1 = json!({
            "id": id1,
            "type": "response",
            "command": "get_state",
            "success": true,
            "data": {
                "thinkingLevel": "high",
                "isStreaming": false,
                "isCompacting": false,
                "steeringMode": "all",
                "followUpMode": "all",
                "sessionId": "durable-1",
                "autoCompactionEnabled": false,
                "messageCount": 0,
                "pendingMessageCount": 0
            }
        });
        writer.write_record(&resp1).await.unwrap();
    });

    let client1 = client.clone();
    let fut1 = tokio::spawn(async move { client1.get_state().await });

    let client2 = client.clone();
    let fut2 = tokio::spawn(async move { client2.get_commands().await });

    let (res1, res2) = tokio::join!(fut1, fut2);
    let state = res1.unwrap().unwrap();
    let commands = res2.unwrap().unwrap();

    assert_eq!(state.session_id, "durable-1");
    assert_eq!(state.thinking_level, ThinkingLevel::High);

    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].name, "review");

    server_handle.await.unwrap();
}

#[tokio::test]
async fn test_client_command_rejected_error() {
    let (client_stdin, mut server_stdin) = duplex(4096);
    let (mut server_stdout, client_stdout) = duplex(4096);

    let client = PiClient::from_io(Box::new(client_stdin), Box::new(client_stdout), None);

    let server_handle = tokio::spawn(async move {
        let mut reader = JsonLinesReader::new(&mut server_stdin);
        let mut writer = JsonLinesWriter::new(&mut server_stdout);

        let cmd: RpcCommand = reader.read_record().await.unwrap().unwrap();
        let cmd_id = cmd.id().unwrap().to_string();

        let resp = json!({
            "id": cmd_id,
            "type": "response",
            "command": "set_model",
            "success": false,
            "error": "Model not found: nonexistent/model"
        });
        writer.write_record(&resp).await.unwrap();
    });

    let err = client
        .set_model("nonexistent", "model")
        .await
        .unwrap_err();

    match err {
        PiError::CommandRejected { command, error } => {
            assert_eq!(command, "set_model");
            assert_eq!(error, "Model not found: nonexistent/model");
        }
        other => panic!("Unexpected error variant: {:?}", other),
    }

    server_handle.await.unwrap();
}

#[tokio::test]
async fn test_client_extension_ui_dialog_and_response() {
    let (client_stdin, mut server_stdin) = duplex(4096);
    let (mut server_stdout, client_stdout) = duplex(4096);

    let client = PiClient::from_io(Box::new(client_stdin), Box::new(client_stdout), None);
    let mut ui_rx = client.subscribe_ui_requests();

    let server_handle = tokio::spawn(async move {
        let mut reader = JsonLinesReader::new(&mut server_stdin);
        let mut writer = JsonLinesWriter::new(&mut server_stdout);

        let req = json!({
            "type": "extension_ui_request",
            "id": "ui-dialog-42",
            "method": "select",
            "title": "Choose action",
            "options": ["Proceed", "Abort"]
        });
        writer.write_record(&req).await.unwrap();

        let resp: ExtensionUiResponse = reader.read_record().await.unwrap().unwrap();
        assert_eq!(response_id(&resp), "ui-dialog-42");
        assert_eq!(
            resp.payload,
            ExtensionUiResponsePayload::Value {
                value: "Proceed".into()
            }
        );
    });

    let req = ui_rx.recv().await.unwrap();
    assert_eq!(req.id(), Some("ui-dialog-42"));
    assert!(req.is_dialog());

    client
        .respond_ui(ExtensionUiResponse::value("ui-dialog-42", "Proceed"))
        .await
        .unwrap();

    server_handle.await.unwrap();
}

fn response_id(r: &ExtensionUiResponse) -> &str {
    &r.id
}
