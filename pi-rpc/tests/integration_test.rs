use std::time::Duration;
use pi_rpc::protocol::PiEvent;
use pi_rpc::{PiClient, PiProcessConfig};

#[tokio::test]
async fn test_live_pi_durable_get_state_and_shutdown() {
    let config = PiProcessConfig::new()
        .with_no_session(true)
        .with_env("PI_CODING_AGENT_CORE_DIR", "/nonexistent");

    let client = PiClient::spawn(config)
        .await
        .expect("Failed to spawn live Pi Durable process");

    // 1. Test get_state
    let state = client
        .get_state()
        .await
        .expect("Failed to get state from live Pi Durable");

    assert!(state.session_id.starts_with("durable-"));
    assert!(!state.is_streaming);

    // 2. Test thinking levels
    let levels = client
        .get_available_thinking_levels()
        .await
        .expect("Failed to get thinking levels from live Pi Durable");

    assert!(!levels.is_empty());

    // 3. Test direct bash command
    let bash_res = client
        .bash("echo 'hello from pi durable rpc'", None)
        .await
        .expect("Failed to execute bash via live Pi Durable");

    assert!(bash_res.output.contains("hello from pi durable rpc"));

    // 4. Test graceful shutdown
    client
        .shutdown()
        .await
        .expect("Failed to gracefully shut down live Pi Durable");
}

#[tokio::test]
async fn test_live_pi_durable_sqlite_persistence_and_reload_continuity() {
    let temp_dir = std::env::temp_dir().join(format!("pi_durable_rpc_test_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    // 1. Spawn first client on temp_dir and run a prompt turn to completion
    {
        let config = PiProcessConfig::new()
            .with_working_dir(&temp_dir)
            .with_session_dir(&temp_dir)
            .with_env("PI_CODING_AGENT_CORE_DIR", "/nonexistent");

        let client = PiClient::spawn(config).await.unwrap();
        let mut rx = client.subscribe_events();

        client
            .prompt("Persist this across applet reload")
            .await
            .unwrap();

        // Wait for RunEnd
        tokio::time::timeout(Duration::from_secs(5), async {
            while let Ok(ev) = rx.recv().await {
                if matches!(ev, PiEvent::RunEnd { .. }) {
                    break;
                }
            }
        })
        .await
        .expect("Timed out waiting for RunEnd");

        client.shutdown().await.unwrap();
    }

    // 2. Spawn second client on the same project/session directory (simulating cosmic-panel reload)
    {
        let config = PiProcessConfig::new()
            .with_working_dir(&temp_dir)
            .with_session_dir(&temp_dir)
            .with_env("PI_CODING_AGENT_CORE_DIR", "/nonexistent");

        let client = PiClient::spawn(config).await.unwrap();
        let mut rx = client.subscribe_events();

        let snap = if let Some(s) = client.latest_snapshot() {
            s
        } else {
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    if let Ok(PiEvent::Snapshot(s)) = rx.recv().await {
                        return s;
                    }
                }
            })
            .await
            .expect("Timed out waiting for Snapshot on reload")
        };

        // Must contain the user entry and assistant answer from the previous process!
        assert!(snap.entries.iter().any(|e| e.kind == "pi.user"));
        assert!(snap.entries.iter().any(|e| e.kind == "pi.assistant"));

        client.shutdown().await.unwrap();
    }

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn test_live_pi_durable_mid_run_reload_auto_resumes() {
    let temp_dir = std::env::temp_dir().join(format!(
        "pi_durable_midrun_test_{}_{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    // 1. Spawn first client with a 400ms generation delay, submit a prompt, and drop/kill mid-run
    {
        let config = PiProcessConfig::new()
            .with_working_dir(&temp_dir)
            .with_session_dir(&temp_dir)
            .with_env("PI_CODING_AGENT_CORE_DIR", "/nonexistent")
            .with_env("PI_DURABLE_FAUX_DELAY_MS", "400");

        let client = PiClient::spawn(config).await.unwrap();
        let mut rx = client.subscribe_events();

        client
            .prompt("Finish this even if cosmic-panel reloads mid-turn")
            .await
            .unwrap();

        // Wait until RunStart is observed, then immediately drop the client (killing the child mid-turn)
        tokio::time::timeout(Duration::from_secs(5), async {
            while let Ok(ev) = rx.recv().await {
                if matches!(ev, PiEvent::RunStart { .. }) {
                    break;
                }
            }
        })
        .await
        .expect("Timed out waiting for RunStart");
    }

    // 2. Spawn second client on the same session directory without sending any new prompt:
    // harness.resume() must automatically finish the interrupted run and produce a pi.assistant entry!
    {
        let config = PiProcessConfig::new()
            .with_working_dir(&temp_dir)
            .with_session_dir(&temp_dir)
            .with_env("PI_CODING_AGENT_CORE_DIR", "/nonexistent")
            .with_env("PI_DURABLE_FAUX_DELAY_MS", "10");

        let client = PiClient::spawn(config).await.unwrap();
        let mut rx = client.subscribe_events();

        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(snap) = client.latest_snapshot() {
                    if snap.entries.iter().any(|e| e.kind == "pi.assistant") {
                        break;
                    }
                }
                match rx.recv().await {
                    Ok(PiEvent::MessageEnd { entry }) if entry.kind == "pi.assistant" => break,
                    Ok(PiEvent::RunEnd { .. }) => break,
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
        })
        .await
        .expect("Timed out waiting for auto-resumed run to finish");

        let messages = client.get_messages().await.unwrap();
        assert!(messages.iter().any(|m| m.role == "assistant"));

        client.shutdown().await.unwrap();
    }

    let _ = std::fs::remove_dir_all(&temp_dir);
}
