use std::hash::{Hash, Hasher};
use std::sync::Arc;

use futures::SinkExt;
use pi_rpc::protocol::{ExtensionUiRequest, ExtensionUiResponse, PiEvent};
use pi_rpc::PiClient;

/// Application actions and incoming events for `cosmic-ext-pi`.
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum PiAction {
    /// Connected to Pi background event stream.
    Connected,

    /// Disconnected from event or UI stream.
    Disconnected(String),

    /// Session event emitted by Pi.
    Event(PiEvent),

    /// Incoming UI request from Pi extension (e.g. confirm, select, input, editor).
    UiRequest(ExtensionUiRequest),

    /// General or connection error message.
    Error(String),

    // --- High-level User & Controller Actions ---
    /// Submit a new prompt turn to the agent.
    SubmitPrompt(String),

    /// Submit an out-of-band steering message.
    Steer(String),

    /// Submit a queued follow-up message.
    FollowUp(String),

    /// Request immediate cancellation/abort of active run.
    Abort,

    /// Send a response answering an Extension UI request.
    RespondUi(ExtensionUiResponse),

    /// Toggle collapsed/expanded state of a thinking block.
    ToggleThinking {
        message_index: usize,
        block_index: usize,
    },

    /// Toggle collapsed/expanded state of a tool call card by tool call ID.
    ToggleTool(String),

    /// Text changed in active input dialog.
    DialogInputChanged(String, String),

    /// Option selected in active select dialog.
    DialogSelectOption(String, usize),

    /// Confirm dialog answered (id, confirmed).
    DialogConfirm(String, bool),

    /// Dialog cancelled (id).
    DialogCancel(String),

    /// Text editor action in active editor dialog.
    EditorAction(String, cosmic::iced::widget::text_editor::Action),

    /// Background command finished with result.
    CommandCompleted(Result<(), String>),

    /// Prompt completed with disposition or error.
    PromptCompleted(Result<pi_rpc::protocol::PromptDisposition, String>),

    /// Discovered slash commands loaded from Pi RPC.
    SlashCommandsLoaded(Vec<pi_rpc::protocol::RpcSlashCommand>),

    /// Add a system message to history.
    AddSystemMessage(String),

    /// Compaction finished.
    CompactionCompleted(Result<pi_rpc::protocol::CompactionResult, String>),

    /// Session reset completed.
    SessionResetCompleted(Result<(), String>),

    /// Model changed via /model.
    ModelChanged(Result<String, String>),

    /// Stats retrieved via /session or /stats.
    StatsRetrieved(Result<pi_rpc::protocol::SessionStats, String>),

    /// No-op action (e.g. for link clicks or ignored events).
    Noop,
}

#[derive(Clone)]
struct SubscriptionData {
    client: Arc<PiClient>,
    id: usize,
}

impl PartialEq for SubscriptionData {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && Arc::ptr_eq(&self.client, &other.client)
    }
}

impl Eq for SubscriptionData {}

impl Hash for SubscriptionData {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
        Arc::as_ptr(&self.client).hash(state);
    }
}

fn run_subscription(data: &SubscriptionData) -> impl futures::Stream<Item = PiAction> {
    let client = Arc::clone(&data.client);
    cosmic::iced::stream::channel(
        256,
        move |mut output: futures::channel::mpsc::Sender<PiAction>| async move {
            let _ = output.send(PiAction::Connected).await;

            let mut event_rx = client.subscribe_events();
            let mut ui_rx = client.subscribe_ui_requests();

            loop {
                tokio::select! {
                    event_res = event_rx.recv() => {
                        match event_res {
                            Ok(event) => {
                                if output.send(PiAction::Event(event)).await.is_err() {
                                    break;
                                }
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(dropped)) => {
                                tracing::warn!("Pi subscription event channel lagged, dropped {dropped} events");
                                if output.send(PiAction::Error(format!("Dropped {dropped} events due to lag"))).await.is_err() {
                                    break;
                                }
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                                let _ = output.send(PiAction::Disconnected("Event stream closed".to_string())).await;
                                break;
                            }
                        }
                    }
                    ui_req = ui_rx.recv() => {
                        match ui_req {
                            Some(req) => {
                                if output.send(PiAction::UiRequest(req)).await.is_err() {
                                    break;
                                }
                            }
                            None => {
                                let _ = output.send(PiAction::Disconnected("UI request stream closed".to_string())).await;
                                break;
                            }
                        }
                    }
                }
            }
        },
    )
}

/// Creates an Iced Subscription that listens to a running `PiClient` with a custom subscription ID.
pub fn pi_subscription_with_id(
    id: usize,
    client: Arc<PiClient>,
) -> cosmic::iced::Subscription<PiAction> {
    cosmic::iced::Subscription::run_with(SubscriptionData { client, id }, run_subscription)
}

/// Creates an Iced Subscription that listens to a running `PiClient`.
pub fn pi_subscription(client: Arc<PiClient>) -> cosmic::iced::Subscription<PiAction> {
    let key = Arc::as_ptr(&client) as usize;
    pi_subscription_with_id(key, client)
}
