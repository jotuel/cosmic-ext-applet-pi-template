use std::path::PathBuf;
use pi_rpc::{PiClient, PiProcessConfig};

#[tokio::test]
async fn test_live_pi_get_state_and_shutdown() {
    let pi_path = if PathBuf::from("/usr/bin/pi").exists() {
        PathBuf::from("/usr/bin/pi")
    } else {
        PathBuf::from("pi")
    };

    let config = PiProcessConfig::new()
        .with_binary_path(pi_path)
        .with_no_session(true);

    let client = PiClient::spawn(config)
        .await
        .expect("Failed to spawn real Pi process");

    // 1. Test get_state
    let state = client
        .get_state()
        .await
        .expect("Failed to get state from live Pi");

    assert!(!state.session_id.is_empty());
    assert!(!state.is_streaming);

    // 2. Test get_commands
    let commands = client
        .get_commands()
        .await
        .expect("Failed to get slash commands from live Pi");

    assert!(!commands.is_empty());

    // 3. Test thinking levels
    let levels = client
        .get_available_thinking_levels()
        .await
        .expect("Failed to get thinking levels from live Pi");

    assert!(!levels.is_empty());

    // 4. Test direct bash command
    let bash_res = client
        .bash("echo 'hello from pi rpc'", None)
        .await
        .expect("Failed to execute bash via live Pi");

    assert!(bash_res.output.contains("hello from pi rpc"));

    // 5. Test graceful shutdown
    client
        .shutdown()
        .await
        .expect("Failed to gracefully shut down live Pi");
}
