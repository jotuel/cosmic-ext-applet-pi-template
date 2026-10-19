use std::sync::Arc;
use tokio::io::duplex;

use pi_rpc::protocol::*;
use pi_rpc::transport::JsonLinesWriter;
use pi_rpc::PiClient;
use serde_json::json;

#[tokio::test]
async fn test_subscription_events_and_ui_requests() {
    let (client_stdin, _server_stdin) = duplex(4096);
    let (mut server_stdout, client_stdout) = duplex(4096);

    let client = Arc::new(PiClient::from_io(
        Box::new(client_stdin),
        Box::new(client_stdout),
        None,
    ));

    // Subscribe to client directly to verify raw stream
    let mut event_rx = client.subscribe_events();
    let mut ui_rx = client.subscribe_ui_requests();

    let server_handle = tokio::spawn(async move {
        let mut writer = JsonLinesWriter::new(&mut server_stdout);

        // 1. Emit an event
        let event = json!({
            "type": "thinking_level_changed",
            "level": "medium"
        });
        writer.write_record(&event).await.unwrap();

        // 2. Emit an extension UI request
        let req = json!({
            "type": "extension_ui_request",
            "id": "ui-dialog-sub",
            "method": "confirm",
            "title": "Authorize",
            "message": "Do you authorize?"
        });
        writer.write_record(&req).await.unwrap();
    });

    // Verify raw client subscription gets events
    let event = event_rx.recv().await.unwrap();
    assert_eq!(
        event,
        PiEvent::ThinkingLevelChanged {
            level: ThinkingLevel::Medium
        }
    );

    let ui_req = ui_rx.recv().await.unwrap();
    assert_eq!(ui_req.id(), Some("ui-dialog-sub"));

    server_handle.await.unwrap();
}
