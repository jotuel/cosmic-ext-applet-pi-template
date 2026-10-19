use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use serde::de::DeserializeOwned;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{broadcast, mpsc, oneshot};
use uuid::Uuid;

use crate::error::PiError;
use crate::process::{PiProcess, PiProcessConfig};
use crate::protocol::*;
use crate::transport::{JsonLinesReader, JsonLinesWriter};

pub type BoxedWriter = Box<dyn AsyncWrite + Send + Unpin + 'static>;
pub type BoxedReader = Box<dyn AsyncRead + Send + Unpin + 'static>;

/// Asynchronous client for interacting with Pi in RPC mode.
#[derive(Clone)]
pub struct PiClient {
    writer: Arc<tokio::sync::Mutex<Option<JsonLinesWriter<BoxedWriter>>>>,
    pending_commands: Arc<Mutex<HashMap<String, oneshot::Sender<RawCommandResponse>>>>,
    event_tx: broadcast::Sender<PiEvent>,
    ui_subscribers: Arc<Mutex<Vec<mpsc::Sender<ExtensionUiRequest>>>>,
    process: Arc<tokio::sync::Mutex<Option<PiProcess>>>,
    recent_stderr: Arc<Mutex<VecDeque<String>>>,
}

impl PiClient {
    /// Launches a `pi --mode rpc` child process and starts JSONL protocol framing.
    pub async fn spawn(config: PiProcessConfig) -> Result<Self, PiError> {
        let (child_stdin, child_stdout, proc) = PiProcess::spawn(&config)?;
        Ok(Self::from_io(
            Box::new(child_stdin),
            Box::new(child_stdout),
            Some(proc),
        ))
    }

    /// Constructs a `PiClient` from arbitrary asynchronous I/O reader/writer streams.
    /// Useful for testing, in-memory mocks, and custom network transports.
    pub fn from_io(
        stdin_writer: BoxedWriter,
        stdout_reader: BoxedReader,
        process: Option<PiProcess>,
    ) -> Self {
        let writer = Arc::new(tokio::sync::Mutex::new(Some(JsonLinesWriter::new(stdin_writer))));
        let pending_commands: Arc<Mutex<HashMap<String, oneshot::Sender<RawCommandResponse>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let (event_tx, _) = broadcast::channel(2048);
        let ui_subscribers: Arc<Mutex<Vec<mpsc::Sender<ExtensionUiRequest>>>> =
            Arc::new(Mutex::new(Vec::new()));

        let recent_stderr = Arc::new(Mutex::new(VecDeque::new()));

        let pending_clone = Arc::clone(&pending_commands);
        let event_tx_clone = event_tx.clone();
        let ui_subs_clone = Arc::clone(&ui_subscribers);

        tokio::spawn(async move {
            let mut reader = JsonLinesReader::new(stdout_reader);
            loop {
                match reader.read_record::<IncomingRecord>().await {
                    Ok(Some(record)) => match record {
                        IncomingRecord::Response(raw_resp) => {
                            if let Some(ref id) = raw_resp.id {
                                let mut map = pending_clone.lock().unwrap();
                                if let Some(sender) = map.remove(id) {
                                    let _ = sender.send(raw_resp);
                                } else {
                                    tracing::warn!(
                                        target: "pi_rpc::client",
                                        "Received response for untracked command ID: {}",
                                        id
                                    );
                                }
                            } else {
                                tracing::warn!(
                                    target: "pi_rpc::client",
                                    "Received response without request ID: {:?}",
                                    raw_resp
                                );
                            }
                        }
                        IncomingRecord::ExtensionUi(req) => {
                            let mut subs = ui_subs_clone.lock().unwrap();
                            subs.retain(|tx| {
                                let _ = tx.try_send(req.clone());
                                !tx.is_closed()
                            });
                        }
                        IncomingRecord::Event(event) => {
                            let _ = event_tx_clone.send(event);
                        }
                    },
                    Ok(None) => {
                        tracing::debug!(target: "pi_rpc::client", "Stdout stream closed");
                        break;
                    }
                    Err(err) => {
                        tracing::warn!(target: "pi_rpc::client", "Error parsing incoming record: {}", err);
                        if let PiError::Io(_) = err {
                            break;
                        }
                    }
                }
            }

            let mut map = pending_clone.lock().unwrap();
            map.clear();
        });

        Self {
            writer,
            pending_commands,
            event_tx,
            ui_subscribers,
            process: Arc::new(tokio::sync::Mutex::new(process)),
            recent_stderr,
        }
    }

    /// Internal command dispatcher with request correlation.
    async fn execute_command<T: DeserializeOwned>(&self, mut cmd: RpcCommand) -> Result<T, PiError> {
        let id = cmd
            .id()
            .map(String::from)
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        cmd.set_id(id.clone());

        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending_commands.lock().unwrap();
            pending.insert(id.clone(), tx);
        }

        {
            let mut writer_guard = self.writer.lock().await;
            let writer = writer_guard.as_mut().ok_or(PiError::Shutdown)?;
            if let Err(e) = writer.write_record(&cmd).await {
                let mut pending = self.pending_commands.lock().unwrap();
                pending.remove(&id);
                return Err(e);
            }
        }

        let response = match rx.await {
            Ok(resp) => resp,
            Err(_) => {
                let stderr = self.stderr_dump().await;
                return Err(PiError::UnexpectedExit {
                    status: std::process::ExitStatus::default(),
                    stderr,
                });
            }
        };

        if !response.success {
            return Err(PiError::CommandRejected {
                command: response.command,
                error: response.error.unwrap_or_else(|| "Unknown error".to_string()),
            });
        }

        if let Some(data) = response.data {
            let parsed: T = serde_json::from_value(data)?;
            Ok(parsed)
        } else {
            let parsed: T = serde_json::from_value(serde_json::Value::Null)?;
            Ok(parsed)
        }
    }

    /// Subscribes to the broadcast stream of all incoming session events.
    pub fn subscribe_events(&self) -> broadcast::Receiver<PiEvent> {
        self.event_tx.subscribe()
    }

    /// Subscribes to incoming Extension UI requests.
    pub fn subscribe_ui_requests(&self) -> mpsc::Receiver<ExtensionUiRequest> {
        let (tx, rx) = mpsc::channel(100);
        let mut subs = self.ui_subscribers.lock().unwrap();
        subs.push(tx);
        rx
    }

    /// Sends an answer to an outstanding Extension UI request.
    pub async fn respond_ui(&self, response: ExtensionUiResponse) -> Result<(), PiError> {
        let mut writer_guard = self.writer.lock().await;
        let writer = writer_guard.as_mut().ok_or(PiError::Shutdown)?;
        writer.write_record(&response).await
    }

    /// Returns recent stderr captured from the child process.
    pub async fn stderr_dump(&self) -> String {
        let proc = self.process.lock().await;
        if let Some(ref p) = *proc {
            p.stderr_dump()
        } else if let Ok(buf) = self.recent_stderr.lock() {
            buf.iter().cloned().collect::<Vec<_>>().join("\n")
        } else {
            String::new()
        }
    }

    // ---------------- High-Level Command APIs ----------------

    /// Sends a prompt to Pi.
    pub async fn prompt(&self, prompt: impl Into<PromptRequest>) -> Result<PromptDisposition, PiError> {
        let req = prompt.into();
        let cmd = RpcCommand::Prompt {
            id: None,
            message: req.message,
            images: req.images,
            streaming_behavior: req.streaming_behavior,
        };
        let data: PromptData = self.execute_command(cmd).await?;
        Ok(data.disposition)
    }

    /// Out-of-band steering message delivered after current assistant turn.
    pub async fn steer(&self, message: impl Into<String>) -> Result<QueuedInputDisposition, PiError> {
        self.steer_with_images(message, Vec::new()).await
    }

    /// Out-of-band steering message with optional image attachments.
    pub async fn steer_with_images(
        &self,
        message: impl Into<String>,
        images: Vec<ImageContent>,
    ) -> Result<QueuedInputDisposition, PiError> {
        let cmd = RpcCommand::Steer {
            id: None,
            message: message.into(),
            images: if images.is_empty() { None } else { Some(images) },
        };
        let data: QueuedInputData = self.execute_command(cmd).await?;
        Ok(data.disposition)
    }

    /// Queue follow-up message to execute when current turns settle.
    pub async fn follow_up(&self, message: impl Into<String>) -> Result<QueuedInputDisposition, PiError> {
        self.follow_up_with_images(message, Vec::new()).await
    }

    /// Queue follow-up message with optional image attachments.
    pub async fn follow_up_with_images(
        &self,
        message: impl Into<String>,
        images: Vec<ImageContent>,
    ) -> Result<QueuedInputDisposition, PiError> {
        let cmd = RpcCommand::FollowUp {
            id: None,
            message: message.into(),
            images: if images.is_empty() { None } else { Some(images) },
        };
        let data: QueuedInputData = self.execute_command(cmd).await?;
        Ok(data.disposition)
    }

    /// Abort current agent activity and wait for session to become idle.
    pub async fn abort(&self) -> Result<(), PiError> {
        let cmd = RpcCommand::Abort { id: None };
        self.execute_command(cmd).await
    }

    /// Remove queued steering and follow-up messages and return their text.
    pub async fn clear_queue(&self) -> Result<ClearQueueData, PiError> {
        let cmd = RpcCommand::ClearQueue { id: None };
        self.execute_command(cmd).await
    }

    /// Reset or start a new session.
    pub async fn new_session(&self, parent_session: Option<String>) -> Result<bool, PiError> {
        let cmd = RpcCommand::NewSession {
            id: None,
            parent_session,
        };
        let data: SessionCancelledData = self.execute_command(cmd).await?;
        Ok(data.cancelled)
    }

    /// Query current session state, model, thinking level, and flags.
    pub async fn get_state(&self) -> Result<RpcSessionState, PiError> {
        let cmd = RpcCommand::GetState { id: None };
        self.execute_command(cmd).await
    }

    /// Switch active model dynamically.
    pub async fn set_model(
        &self,
        provider: impl Into<String>,
        model_id: impl Into<String>,
    ) -> Result<Model, PiError> {
        let cmd = RpcCommand::SetModel {
            id: None,
            provider: provider.into(),
            model_id: model_id.into(),
        };
        self.execute_command(cmd).await
    }

    /// Cycle to the next configured model.
    pub async fn cycle_model(&self) -> Result<Option<CycleModelData>, PiError> {
        let cmd = RpcCommand::CycleModel { id: None };
        self.execute_command(cmd).await
    }

    /// Retrieve all configured models.
    pub async fn get_available_models(&self) -> Result<Vec<Model>, PiError> {
        let cmd = RpcCommand::GetAvailableModels { id: None };
        let data: AvailableModelsData = self.execute_command(cmd).await?;
        Ok(data.models)
    }

    /// Set reasoning budget / thinking level.
    pub async fn set_thinking_level(&self, level: ThinkingLevel) -> Result<(), PiError> {
        let cmd = RpcCommand::SetThinkingLevel { id: None, level };
        self.execute_command(cmd).await
    }

    /// Cycle through available thinking levels.
    pub async fn cycle_thinking_level(&self) -> Result<Option<ThinkingLevel>, PiError> {
        let cmd = RpcCommand::CycleThinkingLevel { id: None };
        let data: Option<ThinkingLevelData> = self.execute_command(cmd).await?;
        Ok(data.map(|d| d.level))
    }

    /// Retrieve available thinking levels for the active model.
    pub async fn get_available_thinking_levels(&self) -> Result<Vec<ThinkingLevel>, PiError> {
        let cmd = RpcCommand::GetAvailableThinkingLevels { id: None };
        let data: AvailableThinkingLevelsData = self.execute_command(cmd).await?;
        Ok(data.levels)
    }

    /// Set delivery mode for steering messages.
    pub async fn set_steering_mode(&self, mode: QueueMode) -> Result<(), PiError> {
        let cmd = RpcCommand::SetSteeringMode { id: None, mode };
        self.execute_command(cmd).await
    }

    /// Set delivery mode for follow-up messages.
    pub async fn set_follow_up_mode(&self, mode: QueueMode) -> Result<(), PiError> {
        let cmd = RpcCommand::SetFollowUpMode { id: None, mode };
        self.execute_command(cmd).await
    }

    /// Trigger context compaction.
    pub async fn compact(&self, custom_instructions: Option<String>) -> Result<CompactionResult, PiError> {
        let cmd = RpcCommand::Compact {
            id: None,
            custom_instructions,
        };
        self.execute_command(cmd).await
    }

    /// Enable or disable auto compaction.
    pub async fn set_auto_compaction(&self, enabled: bool) -> Result<(), PiError> {
        let cmd = RpcCommand::SetAutoCompaction { id: None, enabled };
        self.execute_command(cmd).await
    }

    /// Enable or disable auto retry on errors.
    pub async fn set_auto_retry(&self, enabled: bool) -> Result<(), PiError> {
        let cmd = RpcCommand::SetAutoRetry { id: None, enabled };
        self.execute_command(cmd).await
    }

    /// Abort an ongoing automatic retry wait.
    pub async fn abort_retry(&self) -> Result<(), PiError> {
        let cmd = RpcCommand::AbortRetry { id: None };
        self.execute_command(cmd).await
    }

    /// Execute a bash command directly.
    pub async fn bash(
        &self,
        command: impl Into<String>,
        exclude_from_context: Option<bool>,
    ) -> Result<BashResult, PiError> {
        let cmd = RpcCommand::Bash {
            id: None,
            command: command.into(),
            exclude_from_context,
        };
        self.execute_command(cmd).await
    }

    /// Abort an ongoing direct bash execution.
    pub async fn abort_bash(&self) -> Result<(), PiError> {
        let cmd = RpcCommand::AbortBash { id: None };
        self.execute_command(cmd).await
    }

    /// Retrieve token usage and session statistics.
    pub async fn get_session_stats(&self) -> Result<SessionStats, PiError> {
        let cmd = RpcCommand::GetSessionStats { id: None };
        self.execute_command(cmd).await
    }

    /// Export the current session to HTML.
    pub async fn export_html(&self, output_path: Option<String>) -> Result<String, PiError> {
        let cmd = RpcCommand::ExportHtml {
            id: None,
            output_path,
        };
        let data: ExportHtmlData = self.execute_command(cmd).await?;
        Ok(data.path)
    }

    /// Switch to a different session file.
    pub async fn switch_session(&self, session_path: impl Into<String>) -> Result<bool, PiError> {
        let cmd = RpcCommand::SwitchSession {
            id: None,
            session_path: session_path.into(),
        };
        let data: SessionCancelledData = self.execute_command(cmd).await?;
        Ok(data.cancelled)
    }

    /// Fork session from a specific entry ID.
    pub async fn fork(&self, entry_id: impl Into<String>) -> Result<ForkData, PiError> {
        let cmd = RpcCommand::Fork {
            id: None,
            entry_id: entry_id.into(),
        };
        self.execute_command(cmd).await
    }

    /// Clone the current session into a new session file.
    pub async fn clone_session(&self) -> Result<bool, PiError> {
        let cmd = RpcCommand::CloneSession { id: None };
        let data: SessionCancelledData = self.execute_command(cmd).await?;
        Ok(data.cancelled)
    }

    /// Retrieve messages available for forking.
    pub async fn get_fork_messages(&self) -> Result<Vec<ForkMessage>, PiError> {
        let cmd = RpcCommand::GetForkMessages { id: None };
        let data: ForkMessagesData = self.execute_command(cmd).await?;
        Ok(data.messages)
    }

    /// Retrieve the text of the last assistant turn.
    pub async fn get_last_assistant_text(&self) -> Result<Option<String>, PiError> {
        let cmd = RpcCommand::GetLastAssistantText { id: None };
        let data: LastAssistantTextData = self.execute_command(cmd).await?;
        Ok(data.text)
    }

    /// Set session display name.
    pub async fn set_session_name(&self, name: impl Into<String>) -> Result<(), PiError> {
        let cmd = RpcCommand::SetSessionName {
            id: None,
            name: name.into(),
        };
        self.execute_command(cmd).await
    }

    /// Retrieve all messages in current session.
    pub async fn get_messages(&self) -> Result<Vec<AgentMessage>, PiError> {
        let cmd = RpcCommand::GetMessages { id: None };
        let data: GetMessagesData = self.execute_command(cmd).await?;
        Ok(data.messages)
    }

    /// Retrieve all registered slash commands, skills, and prompt templates.
    pub async fn get_commands(&self) -> Result<Vec<RpcSlashCommand>, PiError> {
        let cmd = RpcCommand::GetCommands { id: None };
        let data: GetCommandsData = self.execute_command(cmd).await?;
        Ok(data.commands)
    }

    /// Gracefully closes stdin and awaits child process termination.
    pub async fn shutdown(self) -> Result<(), PiError> {
        // Drop writer to send EOF on child stdin
        {
            let mut writer_guard = self.writer.lock().await;
            writer_guard.take();
        }

        let mut proc_guard = self.process.lock().await;
        if let Some(proc) = proc_guard.take() {
            proc.shutdown(Duration::from_secs(5)).await?;
        }

        Ok(())
    }
}
