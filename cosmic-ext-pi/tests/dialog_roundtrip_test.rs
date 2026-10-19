use std::sync::Arc;
use tokio::io::duplex;

use cosmic_ext_pi::controller::{AgentStatus, PiSessionController};
use cosmic_ext_pi::subscription::PiAction;
use pi_rpc::protocol::*;
use pi_rpc::transport::{JsonLinesReader, JsonLinesWriter};
use pi_rpc::PiClient;

#[tokio::test]
async fn test_dialog_roundtrip_delivery() {
    let (client_stdin, mut server_stdin) = duplex(4096);
    let (mut server_stdout, client_stdout) = duplex(4096);

    let client = Arc::new(PiClient::from_io(
        Box::new(client_stdin),
        Box::new(client_stdout),
        None,
    ));

    let mut controller = PiSessionController::new(Arc::clone(&client));

    let server_task = tokio::spawn(async move {
        let mut writer = JsonLinesWriter::new(&mut server_stdout);
        let mut reader = JsonLinesReader::new(&mut server_stdin);

        // 1. Server emits confirm request
        let req = ExtensionUiRequest::Confirm {
            id: "req_confirm_99".to_string(),
            title: "Confirm Danger".to_string(),
            message: "Execute rm -rf?".to_string(),
            timeout: None,
        };
        writer.write_record(&req).await.unwrap();

        // 2. Server waits for client response on stdin
        let response: ExtensionUiResponse = reader.read_record().await.unwrap().unwrap();
        assert_eq!(response.id, "req_confirm_99");
        assert_eq!(
            response.payload,
            ExtensionUiResponsePayload::Confirm { confirmed: true }
        );
    });

    // Simulate subscription forwarding request to controller
    let req = ExtensionUiRequest::Confirm {
        id: "req_confirm_99".to_string(),
        title: "Confirm Danger".to_string(),
        message: "Execute rm -rf?".to_string(),
        timeout: None,
    };
    let _ = controller.update(PiAction::UiRequest(req));

    assert_eq!(
        *controller.status(),
        AgentStatus::AwaitingUser {
            request_id: "req_confirm_99".to_string(),
        }
    );
    assert!(controller.view_active_dialog().is_some());

    // Respond by confirming true directly via client
    client
        .respond_ui(ExtensionUiResponse::confirm("req_confirm_99", true))
        .await
        .unwrap();

    server_task.await.unwrap();
}
