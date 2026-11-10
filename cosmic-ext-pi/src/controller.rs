use std::collections::HashMap;
use std::sync::Arc;

use cosmic::iced::Task;
use cosmic::Element;
use pi_rpc::protocol::{
    EntryRecord, ExtensionUiRequest, ExtensionUiResponse, InboxMode, MessageChange, PiEvent,
    QueuedItem, SnapshotEvent, ThinkingLevel, ToolOutputUpdate, Usage,
};
use pi_rpc::PiClient;
use uuid::Uuid;

use crate::dialogs::{ConfirmDialog, DialogState, EditorDialog, InputDialog, SelectDialog};
use crate::subscription::PiAction;
use crate::widget::{ChatView, StatusBadge};

/// High-level operational status of the Pi Durable agent.
#[derive(Debug, Clone, PartialEq)]
pub enum AgentStatus {
    /// Agent is idle and ready for new instructions.
    Idle,

    /// Model inference / thinking is actively occurring.
    Thinking,

    /// Agent is currently executing a tool (e.g. bash command, file read).
    ExecutingTool { name: String, tool_call_id: String },

    /// Agent is blocked waiting for user confirmation or input via Extension UI.
    AwaitingUser { request_id: String },

    /// An error occurred during execution.
    Error(String),
}

impl AgentStatus {
    /// Returns true if the agent is actively computing or executing.
    pub fn is_busy(&self) -> bool {
        matches!(self, Self::Thinking | Self::ExecutingTool { .. })
    }

    /// Returns a human-readable text label.
    pub fn label(&self) -> &str {
        match self {
            Self::Idle => "Ready",
            Self::Thinking => "Thinking...",
            Self::ExecutingTool { name, .. } => name.as_str(),
            Self::AwaitingUser { .. } => "Awaiting confirmation",
            Self::Error(err) => err.as_str(),
        }
    }
}

/// Execution and display state of a tool call.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCallData {
    pub id: String,
    pub name: String,
    pub args: serde_json::Value,
    pub args_raw: String,
    pub partial_result: Option<serde_json::Value>,
    pub result: Option<serde_json::Value>,
    pub is_error: bool,
    pub duration_ms: Option<f64>,
    pub collapsed: bool,
    pub is_executing: bool,
}

/// A structured content block within a `ChatMessage`.
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum MessageBlock {
    /// Internal reasoning / thinking block.
    Thinking {
        text: String,
        collapsed: bool,
        is_streaming: bool,
    },

    /// Tool call invocation with arguments and optional results.
    ToolCall(ToolCallData),

    /// Formatted markdown or regular text block.
    Text {
        text: String,
        parsed: Vec<cosmic::iced::widget::markdown::Item>,
    },
}

impl PartialEq for MessageBlock {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Self::Thinking {
                    text: t1,
                    collapsed: c1,
                    is_streaming: s1,
                },
                Self::Thinking {
                    text: t2,
                    collapsed: c2,
                    is_streaming: s2,
                },
            ) => t1 == t2 && c1 == c2 && s1 == s2,
            (Self::ToolCall(d1), Self::ToolCall(d2)) => d1 == d2,
            (Self::Text { text: t1, .. }, Self::Text { text: t2, .. }) => t1 == t2,
            _ => false,
        }
    }
}

/// A message in the session history.
#[derive(Debug, Clone, PartialEq)]
pub struct ChatMessage {
    pub id: String,
    pub role: String,
    pub blocks: Vec<MessageBlock>,
    pub timestamp: Option<u64>,
    pub is_streaming: bool,
}

impl ChatMessage {
    /// Construct a user message.
    pub fn user(text: impl Into<String>) -> Self {
        let text_str = text.into();
        Self {
            id: Uuid::new_v4().to_string(),
            role: "user".to_string(),
            blocks: vec![MessageBlock::Text {
                parsed: cosmic::iced::widget::markdown::parse(&text_str).collect(),
                text: text_str,
            }],
            timestamp: None,
            is_streaming: false,
        }
    }

    /// Construct a streaming assistant message.
    pub fn assistant() -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            role: "assistant".to_string(),
            blocks: Vec::new(),
            timestamp: None,
            is_streaming: true,
        }
    }

    /// Construct a system message.
    pub fn system(text: impl Into<String>) -> Self {
        let text_str = text.into();
        Self {
            id: Uuid::new_v4().to_string(),
            role: "system".to_string(),
            blocks: vec![MessageBlock::Text {
                parsed: cosmic::iced::widget::markdown::parse(&text_str).collect(),
                text: text_str,
            }],
            timestamp: None,
            is_streaming: false,
        }
    }

    /// Extracts combined plain text from all text blocks.
    pub fn text(&self) -> String {
        let mut out = String::new();
        for block in &self.blocks {
            if let MessageBlock::Text { text, .. } = block {
                out.push_str(text);
            }
        }
        out
    }

    /// Re-parses and caches markdown items for any text blocks that lack them.
    pub fn update_markdown_cache(&mut self) {
        for block in &mut self.blocks {
            if let MessageBlock::Text { text, parsed } = block {
                if parsed.is_empty() && !text.is_empty() {
                    *parsed = cosmic::iced::widget::markdown::parse(text).collect();
                }
            }
        }
    }
}

/// Extracts `errorMessage` from an assistant `AgentMessage` if present and non-empty.
fn assistant_error_message(msg: &pi_rpc::protocol::AgentMessage) -> Option<&str> {
    msg.extra
        .get("errorMessage")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
}

/// Parses an assistant `AgentMessage` (including any `errorMessage`) into `MessageBlock` items.
fn parse_assistant_message_blocks(
    msg: &pi_rpc::protocol::AgentMessage,
    is_streaming: bool,
) -> Vec<MessageBlock> {
    let mut blocks = parse_assistant_content_blocks(&msg.content, is_streaming);
    if let Some(err_msg) = assistant_error_message(msg) {
        let already_has = blocks
            .iter()
            .any(|b| matches!(b, MessageBlock::Text { text, .. } if text.contains(err_msg)));
        if !already_has {
            let formatted = format!("⚠️ **Error:** {err_msg}");
            blocks.push(MessageBlock::Text {
                parsed: cosmic::iced::widget::markdown::parse(&formatted).collect(),
                text: formatted,
            });
        }
    }
    blocks
}

/// Parses an assistant message's `content` value into `MessageBlock` items.
fn parse_assistant_content_blocks(
    content: &serde_json::Value,
    is_streaming: bool,
) -> Vec<MessageBlock> {
    let mut blocks = Vec::new();
    match content {
        serde_json::Value::String(s) if !s.is_empty() => {
            blocks.push(MessageBlock::Text {
                parsed: if is_streaming {
                    Vec::new()
                } else {
                    cosmic::iced::widget::markdown::parse(s).collect()
                },
                text: s.clone(),
            });
        }
        serde_json::Value::Array(items) => {
            for item in items {
                if let Some(block) = parse_single_content_block(item, is_streaming) {
                    blocks.push(block);
                }
            }
        }
        _ => {}
    }
    blocks
}

fn parse_single_content_block(
    item: &serde_json::Value,
    is_streaming: bool,
) -> Option<MessageBlock> {
    let block_type = item.get("type").and_then(|v| v.as_str())?;
    match block_type {
        "thinking" => {
            let text = item
                .get("thinking")
                .or_else(|| item.get("text"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if !is_streaming && text.is_empty() {
                return None;
            }
            Some(MessageBlock::Thinking {
                text,
                collapsed: true,
                is_streaming,
            })
        }
        "text" => {
            let text = item
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if !is_streaming && text.is_empty() {
                return None;
            }
            let parsed = if is_streaming || text.is_empty() {
                Vec::new()
            } else {
                cosmic::iced::widget::markdown::parse(&text).collect()
            };
            Some(MessageBlock::Text { text, parsed })
        }
        "toolCall" => {
            let id = item
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let name = item
                .get("name")
                .or_else(|| item.get("toolName"))
                .and_then(|v| v.as_str())
                .unwrap_or("tool")
                .to_string();
            let args = item
                .get("arguments")
                .or_else(|| item.get("args"))
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            let args_raw = if args.is_null() {
                String::new()
            } else {
                serde_json::to_string(&args).unwrap_or_default()
            };
            Some(MessageBlock::ToolCall(ToolCallData {
                id,
                name,
                args,
                args_raw,
                partial_result: None,
                result: None,
                is_error: false,
                duration_ms: None,
                collapsed: true,
                is_executing: is_streaming,
            }))
        }
        _ => None,
    }
}

/// Human-readable metadata for a built-in or discovered slash command.
#[derive(Debug, Clone, PartialEq)]
pub struct SlashCommandInfo {
    pub name: String,
    pub description: String,
    pub is_builtin: bool,
}

/// Session controller managing Pi Durable RPC connection, session state, chat history,
/// extension UI requests, and UI rendering.
pub struct PiSessionController {
    client: Arc<PiClient>,
    status: AgentStatus,
    session_id: Option<String>,
    session_name: Option<String>,
    model: Option<String>,
    thinking_level: Option<ThinkingLevel>,
    history: Vec<ChatMessage>,
    pending_ui_requests: HashMap<String, ExtensionUiRequest>,
    active_dialog_id: Option<String>,
    dialog_states: HashMap<String, DialogState>,
    usage: Option<Usage>,
    model_context_window: Option<u64>,
    steering_queue: Vec<String>,
    follow_up_queue: Vec<String>,
    discovered_commands: Vec<pi_rpc::protocol::RpcSlashCommand>,
    current_turn_block_offset: usize,
}

impl PiSessionController {
    /// Creates a new `PiSessionController` wrapping a running `PiClient`.
    pub fn new(client: Arc<PiClient>) -> Self {
        let mut ctrl = Self {
            client: Arc::clone(&client),
            status: AgentStatus::Idle,
            session_id: None,
            session_name: None,
            model: None,
            thinking_level: None,
            history: Vec::new(),
            pending_ui_requests: HashMap::new(),
            active_dialog_id: None,
            dialog_states: HashMap::new(),
            usage: None,
            model_context_window: None,
            steering_queue: Vec::new(),
            follow_up_queue: Vec::new(),
            discovered_commands: Vec::new(),
            current_turn_block_offset: 0,
        };

        if let Some(snap) = client.latest_snapshot() {
            ctrl.hydrate_from_snapshot(snap);
        }

        ctrl
    }

    pub fn client(&self) -> &Arc<PiClient> {
        &self.client
    }

    pub fn status(&self) -> &AgentStatus {
        &self.status
    }

    pub fn history(&self) -> &[ChatMessage] {
        &self.history
    }

    pub fn history_mut(&mut self) -> &mut Vec<ChatMessage> {
        &mut self.history
    }

    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    pub fn session_name(&self) -> Option<&str> {
        self.session_name.as_deref()
    }

    pub fn model(&self) -> Option<&str> {
        self.model.as_deref()
    }

    pub fn set_model(&mut self, model: Option<String>) {
        self.model = model;
    }

    pub fn model_context_window(&self) -> Option<u64> {
        self.model_context_window
    }

    pub fn set_model_context_window(&mut self, window: Option<u64>) {
        self.model_context_window = window;
    }

    pub fn tokens_in(&self) -> u64 {
        self.usage
            .as_ref()
            .map(|u| u.input + u.cache_read)
            .unwrap_or(0)
    }

    pub fn thinking_level(&self) -> Option<ThinkingLevel> {
        self.thinking_level
    }

    pub fn usage(&self) -> Option<&Usage> {
        self.usage.as_ref()
    }

    pub fn steering_queue(&self) -> &[String] {
        &self.steering_queue
    }

    pub fn follow_up_queue(&self) -> &[String] {
        &self.follow_up_queue
    }

    pub fn pending_dialogs_count(&self) -> usize {
        self.pending_ui_requests.len()
    }

    pub fn discovered_commands(&self) -> &[pi_rpc::protocol::RpcSlashCommand] {
        &self.discovered_commands
    }

    pub fn set_discovered_commands(&mut self, commands: Vec<pi_rpc::protocol::RpcSlashCommand>) {
        self.discovered_commands = commands;
    }

    /// Triggers background fetch of discoverable commands, skills, and prompt templates.
    pub fn load_discovered_commands(&mut self) -> Task<PiAction> {
        let client = Arc::clone(&self.client);
        Task::perform(
            async move { client.get_commands().await },
            |res| match res {
                Ok(cmds) => PiAction::SlashCommandsLoaded(cmds),
                Err(e) => {
                    tracing::warn!("Failed to fetch discoverable commands from Pi Durable RPC: {e}");
                    PiAction::Noop
                }
            },
        )
    }

    /// Returns all available commands (built-in and discovered).
    pub fn available_commands(&self) -> Vec<SlashCommandInfo> {
        let mut list = vec![
            SlashCommandInfo {
                name: "help".to_string(),
                description: "Show available commands, skills, and usage".to_string(),
                is_builtin: true,
            },
            SlashCommandInfo {
                name: "compact".to_string(),
                description: "Compact session context to save tokens".to_string(),
                is_builtin: true,
            },
            SlashCommandInfo {
                name: "new".to_string(),
                description: "Start a fresh session and clear history".to_string(),
                is_builtin: true,
            },
            SlashCommandInfo {
                name: "clear".to_string(),
                description: "Alias for /new".to_string(),
                is_builtin: true,
            },
            SlashCommandInfo {
                name: "model".to_string(),
                description: "View or switch active AI model".to_string(),
                is_builtin: true,
            },
            SlashCommandInfo {
                name: "thinking".to_string(),
                description: "View or set reasoning thinking level".to_string(),
                is_builtin: true,
            },
            SlashCommandInfo {
                name: "session".to_string(),
                description: "Show session token stats and cost".to_string(),
                is_builtin: true,
            },
            SlashCommandInfo {
                name: "stats".to_string(),
                description: "Alias for /session".to_string(),
                is_builtin: true,
            },
            SlashCommandInfo {
                name: "name".to_string(),
                description: "Set or view session display name".to_string(),
                is_builtin: true,
            },
            SlashCommandInfo {
                name: "abort".to_string(),
                description: "Abort current execution or inference".to_string(),
                is_builtin: true,
            },
            SlashCommandInfo {
                name: "steer".to_string(),
                description: "Queue out-of-band steering message for next turn".to_string(),
                is_builtin: true,
            },
            SlashCommandInfo {
                name: "followup".to_string(),
                description: "Queue follow-up message after agent finishes".to_string(),
                is_builtin: true,
            },
        ];

        for cmd in &self.discovered_commands {
            list.push(SlashCommandInfo {
                name: cmd.name.clone(),
                description: cmd.description.clone().unwrap_or_default(),
                is_builtin: false,
            });
        }

        list
    }

    /// Formats the `/help` markdown text summarizing built-in and discovered commands.
    pub fn format_help_markdown(&self) -> String {
        let mut out = String::from("### Available Commands\n\n");
        out.push_str("**Built-in Commands:**\n");
        out.push_str("- `/help` — Show this help message\n");
        out.push_str("- `/compact [instructions]` — Compact conversation context\n");
        out.push_str("- `/new`, `/clear` — Start a fresh session\n");
        out.push_str("- `/model [provider/model]` — View or switch model\n");
        out.push_str("- `/thinking [level]` — View or set thinking level (`off`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max`)\n");
        out.push_str("- `/session`, `/stats` — View token usage and statistics\n");
        out.push_str("- `/name [name]` — View or set session name\n");
        out.push_str("- `/steer [message]` — Queue out-of-band steering message for next turn\n");
        out.push_str("- `/followup [message]` — Queue follow-up message after agent finishes\n");
        out.push_str("- `/abort` — Cancel current running task\n\n");

        let mut skills = Vec::new();
        let mut templates = Vec::new();
        let mut extensions = Vec::new();

        for cmd in &self.discovered_commands {
            if cmd.source == "skill" || cmd.name.starts_with("skill:") {
                skills.push(cmd);
            } else if cmd.source == "prompt" {
                templates.push(cmd);
            } else {
                extensions.push(cmd);
            }
        }

        if !skills.is_empty() {
            out.push_str("**Skills:**\n");
            for s in skills {
                let desc = s.description.as_deref().unwrap_or("");
                let short_desc = desc.lines().next().unwrap_or("");
                out.push_str(&format!("- `/{}` — {}\n", s.name, short_desc));
            }
            out.push('\n');
        }

        if !templates.is_empty() {
            out.push_str("**Prompt Templates:**\n");
            for t in templates {
                let desc = t.description.as_deref().unwrap_or("");
                out.push_str(&format!("- `/{}` — {}\n", t.name, desc));
            }
            out.push('\n');
        }

        if !extensions.is_empty() {
            out.push_str("**Extensions:**\n");
            for e in extensions {
                let desc = e.description.as_deref().unwrap_or("");
                out.push_str(&format!("- `/{}` — {}\n", e.name, desc));
            }
            out.push('\n');
        }

        out
    }

    /// Handles incoming `PiAction` events, updating state and returning background Tasks.
    pub fn update(&mut self, action: PiAction) -> Task<PiAction> {
        match action {
            PiAction::Connected => self.load_discovered_commands(),

            PiAction::SlashCommandsLoaded(cmds) => {
                self.discovered_commands = cmds;
                Task::none()
            }

            PiAction::AddSystemMessage(msg) => {
                self.history.push(ChatMessage::system(msg));
                self.status = AgentStatus::Idle;
                Task::none()
            }

            PiAction::CompactionCompleted(res) => {
                match res {
                    Ok(comp) => {
                        let summary = if comp.summary.is_empty() {
                            "Context compaction completed successfully.".to_string()
                        } else {
                            format!(
                                "### Context Compacted\n\n{}\n\n*Tokens before: {}*",
                                comp.summary, comp.tokens_before
                            )
                        };
                        self.history.push(ChatMessage::system(summary));
                    }
                    Err(e) => {
                        self.history
                            .push(ChatMessage::system(format!("**Compaction failed**: {e}")));
                    }
                }
                self.status = AgentStatus::Idle;
                Task::none()
            }

            PiAction::SessionResetCompleted(res) => {
                match res {
                    Ok(_) => {
                        self.history.clear();
                        self.usage = None;
                        self.history
                            .push(ChatMessage::system("**New session started.**"));
                    }
                    Err(e) => {
                        self.history.push(ChatMessage::system(format!(
                            "**Failed to start new session**: {e}"
                        )));
                    }
                }
                self.status = AgentStatus::Idle;
                Task::none()
            }

            PiAction::ModelChanged(res) => {
                match res {
                    Ok(model_name) => {
                        self.model = Some(model_name.clone());
                        self.history.push(ChatMessage::system(format!(
                            "Switched model to **{model_name}**."
                        )));
                    }
                    Err(e) => {
                        self.history.push(ChatMessage::system(format!(
                            "**Failed to switch model**: {e}"
                        )));
                    }
                }
                self.status = AgentStatus::Idle;
                Task::none()
            }

            PiAction::StatsRetrieved(res) => {
                match res {
                    Ok(stats) => {
                        let total_messages = stats
                            .extra
                            .get("totalMessages")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0);
                        let cost = stats
                            .extra
                            .get("cost")
                            .and_then(|v| v.as_f64())
                            .or_else(|| {
                                stats
                                    .usage
                                    .as_ref()
                                    .and_then(|u| u.total_usage().cost.map(|c| c.total))
                            })
                            .unwrap_or(0.0);
                        let (input, output, cache_read, cache_write) =
                            if let Some(toks) = stats.extra.get("tokens") {
                                (
                                    toks.get("input").and_then(|v| v.as_u64()).unwrap_or(0),
                                    toks.get("output").and_then(|v| v.as_u64()).unwrap_or(0),
                                    toks.get("cacheRead").and_then(|v| v.as_u64()).unwrap_or(0),
                                    toks.get("cacheWrite").and_then(|v| v.as_u64()).unwrap_or(0),
                                )
                            } else if let Some(ref usage_state) = stats.usage {
                                let total = usage_state.total_usage();
                                (
                                    total.input,
                                    total.output,
                                    total.cache_read,
                                    total.cache_write,
                                )
                            } else {
                                (0, 0, 0, 0)
                            };

                        let msg = format!(
                            "### Session Statistics\n- **Total Messages**: {}\n- **Input Tokens**: {}\n- **Output Tokens**: {}\n- **Cache Read**: {}\n- **Cache Write**: {}\n- **Estimated Cost**: ${:.4}",
                            total_messages, input, output, cache_read, cache_write, cost
                        );
                        self.history.push(ChatMessage::system(msg));
                    }
                    Err(e) => {
                        self.history.push(ChatMessage::system(format!(
                            "**Failed to retrieve stats**: {e}"
                        )));
                    }
                }
                self.status = AgentStatus::Idle;
                Task::none()
            }

            PiAction::Disconnected(reason) => {
                self.status = AgentStatus::Error(format!("Disconnected: {reason}"));
                Task::none()
            }

            PiAction::Error(err) => {
                self.status = AgentStatus::Error(err);
                Task::none()
            }

            PiAction::Event(event) => {
                self.handle_event(event);
                Task::none()
            }

            PiAction::UiRequest(request) => {
                self.handle_ui_request(request);
                Task::none()
            }

            PiAction::SubmitPrompt(text) => self.submit_prompt(text),

            PiAction::Steer(text) => self.steer(text),

            PiAction::FollowUp(text) => self.follow_up(text),

            PiAction::Abort => self.abort(),

            PiAction::RespondUi(response) => self.respond_ui(response),

            PiAction::ToggleThinking {
                message_index,
                block_index,
            } => {
                if let Some(msg) = self.history.get_mut(message_index) {
                    if let Some(MessageBlock::Thinking { collapsed, .. }) =
                        msg.blocks.get_mut(block_index)
                    {
                        *collapsed = !*collapsed;
                    }
                }
                Task::none()
            }

            PiAction::ToggleTool(tool_call_id) => {
                for msg in &mut self.history {
                    for block in &mut msg.blocks {
                        if let MessageBlock::ToolCall(tool) = block {
                            if tool.id == tool_call_id {
                                tool.collapsed = !tool.collapsed;
                            }
                        }
                    }
                }
                Task::none()
            }

            PiAction::DialogInputChanged(id, val) => {
                if let Some(DialogState::Input { ref mut value, .. }) =
                    self.dialog_states.get_mut(&id)
                {
                    *value = val;
                }
                Task::none()
            }

            PiAction::DialogSelectOption(id, idx) => {
                if let Some(DialogState::Select { ref mut selected }) =
                    self.dialog_states.get_mut(&id)
                {
                    *selected = Some(idx);
                }
                Task::none()
            }

            PiAction::DialogConfirm(id, confirmed) => {
                self.respond_ui(ExtensionUiResponse::confirm(id, confirmed))
            }

            PiAction::DialogCancel(id) => self.respond_ui(ExtensionUiResponse::cancel(id)),

            PiAction::EditorAction(id, act) => {
                if let Some(DialogState::Editor { ref mut content }) =
                    self.dialog_states.get_mut(&id)
                {
                    content.perform(act);
                }
                Task::none()
            }

            PiAction::CommandCompleted(res) => {
                if let Err(e) = res {
                    self.status = AgentStatus::Error(e);
                }
                Task::none()
            }

            PiAction::PromptCompleted(res) => {
                match res {
                    Ok(pi_rpc::protocol::PromptDisposition::Handled) => {
                        self.status = AgentStatus::Idle;
                    }
                    Ok(pi_rpc::protocol::PromptDisposition::Started) => {
                        if !self.status.is_busy() {
                            self.status = AgentStatus::Thinking;
                        }
                    }
                    Ok(pi_rpc::protocol::PromptDisposition::Queued) => {
                        // Prompt was queued behind active work.
                    }
                    Err(e) => {
                        self.status = AgentStatus::Error(e.clone());
                        self.history
                            .push(ChatMessage::system(format!("**Error**: {e}")));
                    }
                }
                Task::none()
            }

            PiAction::Noop => Task::none(),
        }
    }

    /// Submits a prompt request to Pi Durable, intercepting client-side slash commands
    /// and routing discovered commands/skills to the RPC engine.
    pub fn submit_prompt_request(
        &mut self,
        request: pi_rpc::protocol::PromptRequest,
    ) -> Task<PiAction> {
        let trimmed = request.message.trim();

        if trimmed.starts_with('/') {
            return self.handle_slash_command(trimmed);
        }

        self.history
            .push(ChatMessage::user(request.message.clone()));
        self.status = AgentStatus::Thinking;

        let client = Arc::clone(&self.client);
        Task::perform(async move { client.prompt(request).await }, |res| {
            PiAction::PromptCompleted(res.map_err(|e| e.to_string()))
        })
    }

    /// Submits a prompt to Pi Durable, intercepting client-side slash commands (e.g. `/help`, `/compact`, `/new`)
    /// and routing discovered commands/skills to the RPC engine.
    pub fn submit_prompt(&mut self, text: impl Into<String>) -> Task<PiAction> {
        self.submit_prompt_request(pi_rpc::protocol::PromptRequest::new(text))
    }

    fn handle_slash_command(&mut self, trimmed: &str) -> Task<PiAction> {
        let without_slash = &trimmed[1..];
        let mut parts = without_slash.splitn(2, char::is_whitespace);
        let cmd = parts.next().unwrap_or("").trim();
        let args = parts.next().map(str::trim).unwrap_or("");

        match cmd {
            "help" => {
                self.history.push(ChatMessage::user(trimmed));
                let help_text = self.format_help_markdown();
                self.history.push(ChatMessage::system(help_text));
                self.status = AgentStatus::Idle;

                if self.discovered_commands.is_empty() {
                    return self.load_discovered_commands();
                }
                Task::none()
            }

            "compact" => {
                self.history.push(ChatMessage::user(trimmed));
                self.history
                    .push(ChatMessage::system("Compacting context..."));
                self.status = AgentStatus::Thinking;

                let custom_instructions = if args.is_empty() {
                    None
                } else {
                    Some(args.to_string())
                };
                let client = Arc::clone(&self.client);
                Task::perform(
                    async move { client.compact(custom_instructions).await },
                    |res| PiAction::CompactionCompleted(res.map_err(|e| e.to_string())),
                )
            }

            "new" | "clear" => {
                self.history.push(ChatMessage::user(trimmed));
                self.status = AgentStatus::Thinking;

                let client = Arc::clone(&self.client);
                Task::perform(async move { client.reset(None).await }, |res| {
                    PiAction::SessionResetCompleted(res.map(|_| ()).map_err(|e| e.to_string()))
                })
            }

            "abort" => {
                self.history.push(ChatMessage::user(trimmed));
                self.abort()
            }

            "model" => {
                self.history.push(ChatMessage::user(trimmed));
                if args.is_empty() {
                    let current = self.model.as_deref().unwrap_or("default").to_string();
                    let client = Arc::clone(&self.client);
                    Task::perform(
                        async move { client.get_available_models().await },
                        move |res| match res {
                            Ok(models) => {
                                let mut lines = vec![format!(
                                    "**Current model**: `{current}`\n\n**Available models**:"
                                )];
                                for m in models.iter().take(20) {
                                    let provider_str = m.provider.as_deref().unwrap_or("unknown");
                                    lines.push(format!(
                                        "- `{}/{}` ({})",
                                        provider_str, m.id, m.name
                                    ));
                                }
                                if models.len() > 20 {
                                    lines.push(format!("*...and {} more*", models.len() - 20));
                                }
                                PiAction::AddSystemMessage(lines.join("\n"))
                            }
                            Err(e) => PiAction::AddSystemMessage(format!(
                                "**Current model**: `{current}`\n\n*(Could not list models: {e})*"
                            )),
                        },
                    )
                } else {
                    let client = Arc::clone(&self.client);
                    let target = args.to_string();
                    Task::perform(
                        async move {
                            let (provider, model_id) = if let Some((p, m)) = target.split_once('/')
                            {
                                (p.to_string(), m.to_string())
                            } else if let Ok(available) = client.get_available_models().await {
                                if let Some(found) = available
                                    .iter()
                                    .find(|m| m.id == target || m.name == target)
                                {
                                    (
                                        found.provider.clone().unwrap_or_default(),
                                        found.id.clone(),
                                    )
                                } else {
                                    (String::new(), target.clone())
                                }
                            } else {
                                (String::new(), target.clone())
                            };
                            client.set_model(provider, model_id).await
                        },
                        |res| {
                            PiAction::ModelChanged(
                                res.map(|m| match m.provider {
                                    Some(ref p) if !p.is_empty() && !m.id.contains('/') => {
                                        format!("{p}/{}", m.id)
                                    }
                                    _ => m.id,
                                })
                                .map_err(|e| e.to_string()),
                            )
                        },
                    )
                }
            }

            "thinking" => {
                self.history.push(ChatMessage::user(trimmed));
                if args.is_empty() {
                    let current = self
                        .thinking_level
                        .map(|l| format!("{:?}", l))
                        .unwrap_or_else(|| "off".to_string());
                    let client = Arc::clone(&self.client);
                    Task::perform(
                        async move { client.cycle_thinking_level().await },
                        move |res| match res {
                            Ok(Some(lvl)) => PiAction::AddSystemMessage(format!(
                                "Thinking level changed to **{:?}**.",
                                lvl
                            )),
                            Ok(None) => PiAction::AddSystemMessage(format!(
                                "Current thinking level: **{}**",
                                current
                            )),
                            Err(e) => PiAction::AddSystemMessage(format!(
                                "Current thinking level: **{}** (cycle failed: {})",
                                current, e
                            )),
                        },
                    )
                } else {
                    let parsed_level = match args.to_lowercase().as_str() {
                        "off" => Some(ThinkingLevel::Off),
                        "minimal" => Some(ThinkingLevel::Minimal),
                        "low" => Some(ThinkingLevel::Low),
                        "medium" => Some(ThinkingLevel::Medium),
                        "high" => Some(ThinkingLevel::High),
                        "xhigh" => Some(ThinkingLevel::Xhigh),
                        "max" => Some(ThinkingLevel::Max),
                        _ => None,
                    };

                    if let Some(level) = parsed_level {
                        self.thinking_level = Some(level);
                        let client = Arc::clone(&self.client);
                        Task::perform(
                            async move { client.set_thinking_level(level).await },
                            move |res| match res {
                                Ok(_) => PiAction::AddSystemMessage(format!(
                                    "Thinking level set to **{:?}**.",
                                    level
                                )),
                                Err(e) => PiAction::AddSystemMessage(format!(
                                    "**Failed to set thinking level**: {e}"
                                )),
                            },
                        )
                    } else {
                        self.history.push(ChatMessage::system(format!(
                            "Invalid thinking level `{args}`. Valid levels: `off`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max`."
                        )));
                        self.status = AgentStatus::Idle;
                        Task::none()
                    }
                }
            }

            "session" | "stats" => {
                self.history.push(ChatMessage::user(trimmed));
                let client = Arc::clone(&self.client);
                Task::perform(async move { client.get_session_stats().await }, |res| {
                    PiAction::StatsRetrieved(res.map_err(|e| e.to_string()))
                })
            }

            "name" => {
                self.history.push(ChatMessage::user(trimmed));
                if args.is_empty() {
                    let current = self.session_name.as_deref().unwrap_or("unnamed");
                    self.history
                        .push(ChatMessage::system(format!("Session name: **{current}**")));
                    self.status = AgentStatus::Idle;
                    Task::none()
                } else {
                    let new_name = args.to_string();
                    self.session_name = Some(new_name.clone());
                    let client = Arc::clone(&self.client);
                    let name_for_task = new_name.clone();
                    Task::perform(
                        async move { client.set_session_name(name_for_task).await },
                        move |res| match res {
                            Ok(_) => PiAction::AddSystemMessage(format!(
                                "Session name set to **{}**.",
                                new_name
                            )),
                            Err(e) => PiAction::AddSystemMessage(format!(
                                "**Failed to set session name**: {e}"
                            )),
                        },
                    )
                }
            }

            "steer" => {
                self.history.push(ChatMessage::user(trimmed));
                if args.is_empty() {
                    self.history.push(ChatMessage::system("Usage: `/steer <message>`"));
                    self.status = AgentStatus::Idle;
                    Task::none()
                } else {
                    self.steer(args)
                }
            }

            "followup" | "follow-up" => {
                self.history.push(ChatMessage::user(trimmed));
                if args.is_empty() {
                    self.history.push(ChatMessage::system("Usage: `/followup <message>`"));
                    self.status = AgentStatus::Idle;
                    Task::none()
                } else {
                    self.follow_up(args)
                }
            }

            _ => {
                let is_known = cmd == "mcp"
                    || cmd.starts_with("skill:")
                    || self.discovered_commands.iter().any(|c| c.name == cmd);

                if is_known {
                    self.history.push(ChatMessage::user(trimmed));
                    self.status = AgentStatus::Thinking;

                    let client = Arc::clone(&self.client);
                    let prompt_text = trimmed.to_string();
                    Task::perform(async move { client.prompt(prompt_text).await }, |res| {
                        PiAction::PromptCompleted(res.map_err(|e| e.to_string()))
                    })
                } else {
                    self.history.push(ChatMessage::user(trimmed));
                    self.history.push(ChatMessage::system(format!(
                        "Unknown command `/{cmd}`. Type `/help` to see available commands."
                    )));
                    self.status = AgentStatus::Idle;
                    Task::none()
                }
            }
        }
    }

    /// Submits an out-of-band steering message.
    pub fn steer(&mut self, text: impl Into<String>) -> Task<PiAction> {
        let text_str = text.into();
        let client = Arc::clone(&self.client);
        Task::perform(
            async move { client.steer(text_str).await },
            |res| match res {
                Ok(_) => PiAction::CommandCompleted(Ok(())),
                Err(e) => PiAction::CommandCompleted(Err(e.to_string())),
            },
        )
    }

    /// Submits an out-of-band steering message with optional image attachments.
    pub fn steer_with_images(
        &mut self,
        text: impl Into<String>,
        images: Vec<pi_rpc::protocol::ImageContent>,
    ) -> Task<PiAction> {
        let text_str = text.into();
        let client = Arc::clone(&self.client);
        Task::perform(
            async move { client.steer_with_images(text_str, images).await },
            |res| match res {
                Ok(_) => PiAction::CommandCompleted(Ok(())),
                Err(e) => PiAction::CommandCompleted(Err(e.to_string())),
            },
        )
    }

    /// Submits a follow-up turn.
    pub fn follow_up(&mut self, text: impl Into<String>) -> Task<PiAction> {
        let text_str = text.into();
        let client = Arc::clone(&self.client);
        Task::perform(
            async move { client.follow_up(text_str).await },
            |res| match res {
                Ok(_) => PiAction::CommandCompleted(Ok(())),
                Err(e) => PiAction::CommandCompleted(Err(e.to_string())),
            },
        )
    }

    /// Submits a follow-up turn with optional image attachments.
    pub fn follow_up_with_images(
        &mut self,
        text: impl Into<String>,
        images: Vec<pi_rpc::protocol::ImageContent>,
    ) -> Task<PiAction> {
        let text_str = text.into();
        let client = Arc::clone(&self.client);
        Task::perform(
            async move { client.follow_up_with_images(text_str, images).await },
            |res| match res {
                Ok(_) => PiAction::CommandCompleted(Ok(())),
                Err(e) => PiAction::CommandCompleted(Err(e.to_string())),
            },
        )
    }

    /// Aborts current agent activity.
    pub fn abort(&mut self) -> Task<PiAction> {
        let client = Arc::clone(&self.client);
        Task::perform(async move { client.abort().await }, |res| match res {
            Ok(_) => PiAction::CommandCompleted(Ok(())),
            Err(e) => PiAction::CommandCompleted(Err(e.to_string())),
        })
    }

    /// Sends an answer to an outstanding Extension UI request.
    pub fn respond_ui(&mut self, response: ExtensionUiResponse) -> Task<PiAction> {
        let id = response.id.clone();
        self.pending_ui_requests.remove(&id);
        self.dialog_states.remove(&id);

        if self.active_dialog_id.as_deref() == Some(&id) {
            self.active_dialog_id = self.pending_ui_requests.keys().next().cloned();
            if self.active_dialog_id.is_none() {
                if matches!(self.status, AgentStatus::AwaitingUser { .. }) {
                    self.status = AgentStatus::Idle;
                }
            } else if let Some(ref next_id) = self.active_dialog_id {
                self.status = AgentStatus::AwaitingUser {
                    request_id: next_id.clone(),
                };
            }
        }

        let client = Arc::clone(&self.client);
        Task::perform(
            async move { client.respond_ui(response).await },
            |res| match res {
                Ok(_) => PiAction::CommandCompleted(Ok(())),
                Err(e) => PiAction::CommandCompleted(Err(e.to_string())),
            },
        )
    }

    /// Hydrates full controller state from a `SnapshotEvent` emitted on attachment or overflow.
    pub fn hydrate_from_snapshot(&mut self, snap: SnapshotEvent) {
        self.history.clear();
        self.current_turn_block_offset = 0;

        // Filter to active range if a head marker is present
        let active_entries: Vec<&EntryRecord> = if let Some((head_idx, head_entry)) = snap
            .entries
            .iter()
            .enumerate()
            .rfind(|(_, e)| e.head.is_some())
        {
            let target_id = head_entry.head.unwrap_or(head_entry.id);
            let mut slice = vec![head_entry];
            for e in snap.entries.iter().skip(head_idx + 1) {
                if e.head.is_none() && e.id >= target_id {
                    slice.push(e);
                }
            }
            slice
        } else {
            snap.entries.iter().collect()
        };

        for entry in active_entries {
            if self.session_id.is_none() {
                self.session_id = Some(format!("durable-{}", entry.conversation_id));
            }
            self.apply_entry_to_history(entry, false);
        }

        // Attach in-flight generation partial message if present
        if let Some(ref gen) = snap.generation {
            if let Some(ref partial_msg) = gen.message {
                let blocks = parse_assistant_message_blocks(partial_msg, true);
                if let Some(ref u) = partial_msg.usage {
                    self.usage = Some(u.clone());
                }
                let offset = {
                    let msg = self.ensure_streaming_assistant_message();
                    let off = msg.blocks.len();
                    if msg.timestamp.is_none() {
                        msg.timestamp = partial_msg.timestamp;
                    }
                    msg.blocks.extend(blocks);
                    off
                };
                self.current_turn_block_offset = offset;
            }
        }

        // Overlay running/pending tool slots onto history
        for slot in &snap.tools {
            let is_running = slot.status == "running" || slot.status == "pending";
            let partial = slot
                .output
                .as_ref()
                .filter(|s| !s.is_empty())
                .map(|s| serde_json::Value::String(s.clone()));

            let mut found = false;
            self.update_tool_call_in_history(&slot.call_id, |tool| {
                found = true;
                tool.is_executing = is_running;
                if partial.is_some() {
                    tool.partial_result = partial.clone();
                }
            });

            if !found && is_running {
                let msg = self.ensure_streaming_assistant_message();
                msg.blocks.push(MessageBlock::ToolCall(ToolCallData {
                    id: slot.call_id.clone(),
                    name: slot.name.clone(),
                    args: serde_json::Value::Null,
                    args_raw: String::new(),
                    partial_result: partial,
                    result: None,
                    is_error: false,
                    duration_ms: None,
                    collapsed: true,
                    is_executing: true,
                }));
            }
        }

        // Restore agent model & thinking level
        if let Some(ref model_ref) = snap.agent.model {
            self.model = Some(model_ref.full_id());
        }
        if let Some(level) = snap.agent.thinking_level {
            self.thinking_level = Some(level);
        }

        // Restore cumulative usage
        let total_usage = snap.usage.total_usage();
        if total_usage.total_tokens > 0 || total_usage.input > 0 || total_usage.output > 0 {
            self.usage = Some(total_usage);
        }

        // Restore inbox queues
        self.apply_inbox_items(&snap.inbox);

        // Determine active status
        if let Some(running_tool) = snap.tools.iter().find(|t| t.status == "running") {
            self.status = AgentStatus::ExecutingTool {
                name: running_tool.name.clone(),
                tool_call_id: running_tool.call_id.clone(),
            };
        } else if snap.run.is_some() || snap.generation.is_some() {
            self.status = AgentStatus::Thinking;
        } else if self.active_dialog_id.is_none() {
            self.status = AgentStatus::Idle;
        }
    }

    fn apply_inbox_items(&mut self, items: &[QueuedItem]) {
        self.steering_queue.clear();
        self.follow_up_queue.clear();
        for item in items {
            let label = item
                .text
                .clone()
                .unwrap_or_else(|| format!("Queued #{}", item.id));
            match item.mode {
                InboxMode::Steer => self.steering_queue.push(label),
                InboxMode::FollowUp => self.follow_up_queue.push(label),
                InboxMode::Write => {}
            }
        }
    }

    fn ensure_streaming_assistant_message(&mut self) -> &mut ChatMessage {
        let needs_new = match self.history.last() {
            Some(msg) => msg.role != "assistant",
            None => true,
        };
        if needs_new {
            self.current_turn_block_offset = 0;
            self.history.push(ChatMessage::assistant());
        } else if let Some(last) = self.history.last_mut() {
            last.is_streaming = true;
        }
        self.history.last_mut().unwrap()
    }

    fn apply_entry_to_history(&mut self, entry: &EntryRecord, from_live_event: bool) {
        match entry.kind.as_str() {
            "pi.reset" => {
                self.history.clear();
                self.usage = None;
                if let Some(msg) = entry.model.as_ref().and_then(|m| m.first()) {
                    let text = msg.text();
                    if !text.is_empty() {
                        let mut user_msg = ChatMessage::user(text);
                        user_msg.timestamp = msg.timestamp;
                        self.history.push(user_msg);
                    }
                } else if from_live_event {
                    self.history
                        .push(ChatMessage::system("**New session started.**"));
                }
            }

            "pi.compaction" => {
                if let Some(msg) = entry.model.as_ref().and_then(|m| m.first()) {
                    let summary = msg.text();
                    if !summary.is_empty() {
                        self.history.push(ChatMessage::system(format!(
                            "### Context Compacted\n\n{summary}"
                        )));
                    }
                }
            }

            "pi.user" => {
                if let Some(last) = self.history.last() {
                    if last.role == "assistant" && last.blocks.is_empty() {
                        self.history.pop();
                    }
                }
                if let Some(msg) = entry.model.as_ref().and_then(|m| m.first()) {
                    let text = msg.text();
                    let already_exists = self
                        .history
                        .last()
                        .is_some_and(|m| m.role == "user" && m.text() == text);
                    if !already_exists && !text.is_empty() {
                        let mut user_msg = ChatMessage::user(text);
                        user_msg.timestamp = msg.timestamp;
                        self.history.push(user_msg);
                    } else if let Some(last) = self.history.last_mut() {
                        if last.role == "user" && last.timestamp.is_none() {
                            last.timestamp = msg.timestamp;
                        }
                    }
                }
            }

            "pi.assistant" => {
                if let Some(msg) = entry.model.as_ref().and_then(|m| m.first()) {
                    if let Some(ref u) = msg.usage {
                        self.usage = Some(u.clone());
                    }
                    if from_live_event {
                        if let Some(err_msg) = assistant_error_message(msg) {
                            self.status = AgentStatus::Error(err_msg.to_string());
                        }
                    }
                    let parsed_blocks = parse_assistant_message_blocks(msg, false);

                    if let Some(last) = self.history.last_mut() {
                        if last.role == "assistant" {
                            last.is_streaming = false;
                            if last.timestamp.is_none() {
                                last.timestamp = msg.timestamp;
                            }
                            if from_live_event {
                                let offset = self.current_turn_block_offset.min(last.blocks.len());
                                let repeats_prior = offset > 0
                                    && last.blocks[..offset].iter().any(|b| match b {
                                        MessageBlock::ToolCall(t) => parsed_blocks.iter().any(
                                            |inc| matches!(inc, MessageBlock::ToolCall(it) if it.id == t.id),
                                        ),
                                        _ => false,
                                    });
                                if repeats_prior {
                                    Self::merge_settled_blocks(&mut last.blocks, parsed_blocks);
                                } else {
                                    let mut turn_blocks = last.blocks.split_off(offset);
                                    Self::merge_settled_blocks(&mut turn_blocks, parsed_blocks);
                                    last.blocks.extend(turn_blocks);
                                }
                            } else {
                                last.blocks.extend(parsed_blocks);
                            }
                            last.update_markdown_cache();
                            self.current_turn_block_offset = last.blocks.len();
                            return;
                        }
                    }

                    let mut chat_msg = ChatMessage::assistant();
                    chat_msg.is_streaming = false;
                    chat_msg.timestamp = msg.timestamp;
                    chat_msg.blocks = parsed_blocks;
                    chat_msg.update_markdown_cache();
                    self.current_turn_block_offset = chat_msg.blocks.len();
                    if !chat_msg.blocks.is_empty() {
                        self.history.push(chat_msg);
                    }
                }
            }

            "pi.tool-result" => {
                if let Some(msg) = entry.model.as_ref().and_then(|m| m.first()) {
                    if let Some(ref call_id) = msg.tool_call_id {
                        let text = msg.text();
                        let res_val = if !text.is_empty() {
                            Some(serde_json::Value::String(text))
                        } else if !msg.content.is_null() {
                            Some(msg.content.clone())
                        } else {
                            None
                        };
                        let is_err = msg.is_error.unwrap_or(false);
                        let dur = msg.duration_ms;

                        self.update_tool_call_in_history(call_id, |tool| {
                            if res_val.is_some() {
                                tool.result = res_val;
                            }
                            tool.is_error = is_err;
                            if dur.is_some() {
                                tool.duration_ms = dur;
                            }
                            tool.is_executing = false;
                        });
                    }
                }
            }

            _ => {
                if let Some(msg) = entry.model.as_ref().and_then(|m| m.first()) {
                    if msg.role == "user" {
                        let text = msg.text();
                        if !text.is_empty() {
                            self.history.push(ChatMessage::user(text));
                        }
                    }
                }
            }
        }
    }

    fn merge_settled_blocks(existing: &mut Vec<MessageBlock>, incoming: Vec<MessageBlock>) {
        if incoming.is_empty() {
            for b in existing.iter_mut() {
                if let MessageBlock::Thinking { is_streaming, .. } = b {
                    *is_streaming = false;
                }
            }
            return;
        }

        if existing.is_empty() {
            *existing = incoming;
            return;
        }

        // If existing blocks already contain settled tool calls from an earlier turn of the same
        // run that are not repeated in `incoming` (e.g. a post-tool generation turn that only
        // carries the final text block), keep those earlier blocks in front and merge/replace the
        // tail blocks of the current turn.
        let incoming_has_all_tools = existing.iter().all(|b| match b {
            MessageBlock::ToolCall(t) => incoming
                .iter()
                .any(|inc| matches!(inc, MessageBlock::ToolCall(it) if it.id == t.id)),
            _ => true,
        });

        if !incoming_has_all_tools {
            // Finalize any streaming blocks at the tail
            for b in existing.iter_mut() {
                if let MessageBlock::Thinking { is_streaming, .. } = b {
                    *is_streaming = false;
                }
            }
            for inc in incoming {
                match inc {
                    MessageBlock::Text { ref text, .. } => {
                        if let Some(MessageBlock::Text {
                            text: ex_text,
                            parsed: ex_parsed,
                        }) = existing
                            .iter_mut()
                            .rev()
                            .find(|b| matches!(b, MessageBlock::Text { .. }))
                        {
                            if ex_text == text || text.starts_with(ex_text.as_str()) {
                                *ex_text = text.clone();
                                *ex_parsed = cosmic::iced::widget::markdown::parse(text).collect();
                                continue;
                            }
                        }
                        existing.push(inc);
                    }
                    MessageBlock::ToolCall(ref inc_tool) => {
                        if !existing.iter().any(
                            |b| matches!(b, MessageBlock::ToolCall(t) if t.id == inc_tool.id),
                        ) {
                            existing.push(inc);
                        }
                    }
                    MessageBlock::Thinking { ref text, .. } => {
                        if !existing
                            .iter()
                            .any(|b| matches!(b, MessageBlock::Thinking { text: t, .. } if t == text))
                        {
                            existing.push(inc);
                        }
                    }
                }
            }
            return;
        }

        // Preserve collapse state and tool execution results from existing blocks
        let mut merged = Vec::with_capacity(incoming.len());
        for mut inc in incoming {
            match &mut inc {
                MessageBlock::Thinking { collapsed, .. } => {
                    if let Some(MessageBlock::Thinking {
                        collapsed: prev_col,
                        ..
                    }) = existing
                        .iter()
                        .find(|b| matches!(b, MessageBlock::Thinking { .. }))
                    {
                        *collapsed = *prev_col;
                    }
                }
                MessageBlock::ToolCall(inc_tool) => {
                    if let Some(MessageBlock::ToolCall(prev_tool)) = existing.iter().find(|b| {
                        matches!(b, MessageBlock::ToolCall(t) if t.id == inc_tool.id)
                    }) {
                        inc_tool.collapsed = prev_tool.collapsed;
                        inc_tool.partial_result = prev_tool.partial_result.clone();
                        if inc_tool.result.is_none() {
                            inc_tool.result = prev_tool.result.clone();
                            inc_tool.is_error = prev_tool.is_error;
                            inc_tool.duration_ms = prev_tool.duration_ms;
                        }
                        inc_tool.is_executing = prev_tool.is_executing;
                    }
                }
                MessageBlock::Text { .. } => {}
            }
            merged.push(inc);
        }
        *existing = merged;
    }

    /// Internal helper to update state from incoming `PiEvent`s.
    fn handle_event(&mut self, event: PiEvent) {
        match event {
            PiEvent::Snapshot(snap) => {
                self.hydrate_from_snapshot(snap);
            }

            PiEvent::RunStart { .. } => {
                self.status = AgentStatus::Thinking;
            }

            PiEvent::RunEnd { .. } => {
                if !matches!(self.status, AgentStatus::Error(_)) {
                    self.status = AgentStatus::Idle;
                }
                for msg in &mut self.history {
                    if msg.is_streaming {
                        msg.is_streaming = false;
                        msg.update_markdown_cache();
                    }
                }
                if let Some(last) = self.history.last() {
                    if last.role == "assistant" && last.blocks.is_empty() {
                        self.history.pop();
                    }
                }
            }

            PiEvent::TurnStart => {
                self.status = AgentStatus::Thinking;
                let offset = self.ensure_streaming_assistant_message().blocks.len();
                self.current_turn_block_offset = offset;
            }

            PiEvent::TurnEnd => {
                if let Some(last) = self.history.last_mut() {
                    for b in &mut last.blocks {
                        if let MessageBlock::Thinking { is_streaming, .. } = b {
                            *is_streaming = false;
                        }
                    }
                    last.is_streaming = false;
                    last.update_markdown_cache();
                    self.current_turn_block_offset = last.blocks.len();
                }
                if let Some(last) = self.history.last() {
                    if last.role == "assistant" && last.blocks.is_empty() {
                        self.history.pop();
                        self.current_turn_block_offset = 0;
                    }
                }
            }

            PiEvent::MessageStart { message } => {
                if message.role == "assistant" {
                    if let Some(ref u) = message.usage {
                        self.usage = Some(u.clone());
                    }
                    if let Some(err_msg) = assistant_error_message(&message) {
                        self.status = AgentStatus::Error(err_msg.to_string());
                    }
                    let initial_blocks = parse_assistant_message_blocks(&message, true);
                    let _ = self.ensure_streaming_assistant_message();
                    let cur_offset = self.current_turn_block_offset;
                    if let Some(msg) = self.history.last_mut() {
                        if msg.timestamp.is_none() {
                            msg.timestamp = message.timestamp;
                        }
                        let offset = cur_offset.min(msg.blocks.len());
                        if !initial_blocks.is_empty() {
                            msg.blocks.truncate(offset);
                            msg.blocks.extend(initial_blocks);
                        }
                    }
                } else if message.role == "user" {
                    if let Some(last) = self.history.last() {
                        if last.role == "assistant" && last.blocks.is_empty() {
                            self.history.pop();
                        }
                    }
                    self.current_turn_block_offset = 0;

                    let text = message.text();
                    let already_exists = self
                        .history
                        .last()
                        .is_some_and(|m| m.role == "user" && m.text() == text);
                    if !already_exists && !text.is_empty() {
                        let mut user_msg = ChatMessage::user(text);
                        user_msg.timestamp = message.timestamp;
                        self.history.push(user_msg);
                    } else if let Some(last) = self.history.last_mut() {
                        if last.role == "user" && last.timestamp.is_none() {
                            last.timestamp = message.timestamp;
                        }
                    }
                }
            }

            PiEvent::MessageUpdate { usage, changes } => {
                if let Some(u) = usage {
                    self.usage = Some(u);
                }
                for change in changes {
                    self.apply_message_change(change);
                }
            }

            PiEvent::MessageEnd { entry } => {
                self.apply_entry_to_history(&entry, true);
            }

            PiEvent::ToolExecutionStart {
                tool_call_id,
                tool_name,
                args,
                ..
            } => {
                self.status = AgentStatus::ExecutingTool {
                    name: tool_name.clone(),
                    tool_call_id: tool_call_id.clone(),
                };
                let mut found = false;
                let args_clone = args.clone();
                self.update_tool_call_in_history(&tool_call_id, |tool| {
                    found = true;
                    tool.is_executing = true;
                    if !args_clone.is_null() {
                        tool.args = args_clone;
                    }
                });
                if !found {
                    let msg = self.ensure_streaming_assistant_message();
                    let args_raw = if args.is_null() {
                        String::new()
                    } else {
                        serde_json::to_string(&args).unwrap_or_default()
                    };
                    msg.blocks.push(MessageBlock::ToolCall(ToolCallData {
                        id: tool_call_id,
                        name: tool_name,
                        args,
                        args_raw,
                        partial_result: None,
                        result: None,
                        is_error: false,
                        duration_ms: None,
                        collapsed: true,
                        is_executing: true,
                    }));
                }
            }

            PiEvent::ToolExecutionUpdate {
                tool_call_id,
                output,
                ..
            } => {
                if let Some(out) = output {
                    self.update_tool_call_in_history(&tool_call_id, |tool| {
                        apply_tool_output_update(&mut tool.partial_result, &out);
                    });
                }
            }

            PiEvent::ToolExecutionEnd {
                tool_call_id,
                entry,
                result,
                ..
            } => {
                self.update_tool_call_in_history(&tool_call_id, |tool| {
                    tool.is_executing = false;
                    if let Some(ref entry_rec) = entry {
                        if let Some(msg) = entry_rec.model.as_ref().and_then(|m| m.first()) {
                            let text = msg.text();
                            if !text.is_empty() {
                                tool.result = Some(serde_json::Value::String(text));
                            } else if !msg.content.is_null() {
                                tool.result = Some(msg.content.clone());
                            }
                            tool.is_error = msg.is_error.unwrap_or(false);
                            if msg.duration_ms.is_some() {
                                tool.duration_ms = msg.duration_ms;
                            }
                        }
                    } else if let Some(ref nested) = result {
                        tool.result = nested.structured_output.clone();
                        tool.is_error = nested.is_error;
                        tool.duration_ms = nested.duration_ms;
                    }
                });

                if let AgentStatus::ExecutingTool {
                    tool_call_id: ref id,
                    ..
                } = self.status
                {
                    if id == &tool_call_id {
                        self.status = AgentStatus::Thinking;
                    }
                }
            }

            PiEvent::InboxUpdate { items } => {
                self.apply_inbox_items(&items);
            }

            PiEvent::Submission { record } => {
                if record.status == "unanswered" {
                    if let Some(ref detail) = record.detail {
                        if !detail.is_empty() {
                            self.status = AgentStatus::Error(detail.clone());
                            let already_shown = self
                                .history
                                .last()
                                .is_some_and(|m| m.text().contains(detail.as_str()));
                            if !already_shown {
                                self.history
                                    .push(ChatMessage::system(format!("⚠️ **Error:** {detail}")));
                            }
                        }
                    }
                }
            }

            PiEvent::EntryAppended { entry } => {
                self.apply_entry_to_history(&entry, true);
            }

            PiEvent::AgentChanged { agent } => {
                if let Some(ref m) = agent.model {
                    self.model = Some(m.full_id());
                }
                if let Some(lvl) = agent.thinking_level {
                    self.thinking_level = Some(lvl);
                }
            }

            PiEvent::UsageChanged { usage } => {
                self.usage = Some(usage.total_usage());
            }

            PiEvent::TaskFailed { message, .. } => {
                self.status = AgentStatus::Error(message.clone());
                self.history
                    .push(ChatMessage::system(format!("**Error**: {message}")));
            }

            _ => {}
        }
    }

    /// Applies one `MessageChange` delta to the in-flight assistant message.
    fn apply_message_change(&mut self, change: MessageChange) {
        if matches!(
            change,
            MessageChange::ThinkingStart { .. } | MessageChange::ThinkingDelta { .. }
        ) {
            self.status = AgentStatus::Thinking;
        }

        let msg = self.ensure_streaming_assistant_message();

        match change {
            MessageChange::ThinkingStart { block, .. } => {
                let initial = block
                    .get("thinking")
                    .or_else(|| block.get("text"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                msg.blocks.push(MessageBlock::Thinking {
                    text: initial,
                    collapsed: true,
                    is_streaming: true,
                });
            }

            MessageChange::ThinkingDelta { delta, .. } => {
                let has_thinking = msg
                    .blocks
                    .iter()
                    .any(|b| matches!(b, MessageBlock::Thinking { .. }));
                if !has_thinking {
                    msg.blocks.push(MessageBlock::Thinking {
                        text: String::new(),
                        collapsed: true,
                        is_streaming: true,
                    });
                }
                for block in msg.blocks.iter_mut().rev() {
                    if let MessageBlock::Thinking { text, .. } = block {
                        text.push_str(&delta);
                        break;
                    }
                }
            }

            MessageChange::TextStart { block, .. } => {
                for b in msg.blocks.iter_mut() {
                    if let MessageBlock::Thinking { is_streaming, .. } = b {
                        *is_streaming = false;
                    }
                }
                let initial = block
                    .get("text")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                msg.blocks.push(MessageBlock::Text {
                    text: initial,
                    parsed: Vec::new(),
                });
            }

            MessageChange::TextDelta { delta, .. } => {
                let has_text = msg
                    .blocks
                    .iter()
                    .any(|b| matches!(b, MessageBlock::Text { .. }));
                if !has_text {
                    msg.blocks.push(MessageBlock::Text {
                        text: String::new(),
                        parsed: Vec::new(),
                    });
                }
                for block in msg.blocks.iter_mut().rev() {
                    if let MessageBlock::Text { text, .. } = block {
                        text.push_str(&delta);
                        break;
                    }
                }
            }

            MessageChange::ToolcallStart { block, .. } => {
                let id = block
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let name = block
                    .get("name")
                    .or_else(|| block.get("toolName"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("tool")
                    .to_string();
                let args = block
                    .get("arguments")
                    .or_else(|| block.get("args"))
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                let args_raw = if args.is_null() {
                    String::new()
                } else {
                    serde_json::to_string(&args).unwrap_or_default()
                };
                msg.blocks.push(MessageBlock::ToolCall(ToolCallData {
                    id,
                    name,
                    args,
                    args_raw,
                    partial_result: None,
                    result: None,
                    is_error: false,
                    duration_ms: None,
                    collapsed: true,
                    is_executing: true,
                }));
            }

            MessageChange::ToolcallDelta { path, delta, .. } => {
                for block in msg.blocks.iter_mut().rev() {
                    if let MessageBlock::ToolCall(tool) = block {
                        if let Some(first_key) = path.first().and_then(|v| v.as_str()) {
                            if !tool.args.is_object() {
                                tool.args = serde_json::json!({});
                            }
                            if let Some(obj) = tool.args.as_object_mut() {
                                let entry = obj
                                    .entry(first_key.to_string())
                                    .or_insert_with(|| serde_json::Value::String(String::new()));
                                if let Some(existing_str) = entry.as_str() {
                                    let mut updated = existing_str.to_string();
                                    updated.push_str(&delta);
                                    *entry = serde_json::Value::String(updated);
                                }
                            }
                        } else {
                            tool.args_raw.push_str(&delta);
                            if let Ok(parsed) =
                                serde_json::from_str::<serde_json::Value>(&tool.args_raw)
                            {
                                tool.args = parsed;
                            }
                        }
                        break;
                    }
                }
            }

            MessageChange::Block {
                content_index,
                block,
            } => {
                let _ = self.ensure_streaming_assistant_message();
                let offset = self.current_turn_block_offset;
                if let Some(parsed_block) = parse_single_content_block(&block, true) {
                    if let Some(msg) = self.history.last_mut() {
                        let target_idx = offset + content_index;
                        if target_idx < msg.blocks.len() {
                            msg.blocks[target_idx] = parsed_block;
                        } else {
                            msg.blocks.push(parsed_block);
                        }
                    }
                }
            }

            MessageChange::Message {
                message: full_message,
            } => {
                let new_blocks = parse_assistant_message_blocks(&full_message, true);
                let _ = self.ensure_streaming_assistant_message();
                let offset = self.current_turn_block_offset;
                if let Some(msg) = self.history.last_mut() {
                    let trunc_len = offset.min(msg.blocks.len());
                    msg.blocks.truncate(trunc_len);
                    msg.blocks.extend(new_blocks);
                }
            }
        }
    }

    /// Mutates an existing tool call block across history.
    fn update_tool_call_in_history(
        &mut self,
        tool_call_id: &str,
        update: impl FnOnce(&mut ToolCallData),
    ) {
        for msg in &mut self.history {
            for block in &mut msg.blocks {
                if let MessageBlock::ToolCall(tool) = block {
                    if tool.id == tool_call_id {
                        update(tool);
                        return;
                    }
                }
            }
        }
    }

    /// Handles incoming Extension UI requests.
    fn handle_ui_request(&mut self, request: ExtensionUiRequest) {
        match &request {
            ExtensionUiRequest::Notify {
                message,
                notify_type,
                ..
            } => {
                let formatted = match notify_type {
                    Some(pi_rpc::protocol::NotifyType::Error) => format!("**Error**: {message}"),
                    Some(pi_rpc::protocol::NotifyType::Warning) => {
                        format!("**Warning**: {message}")
                    }
                    _ => message.clone(),
                };
                self.history.push(ChatMessage::system(formatted));
                return;
            }
            ExtensionUiRequest::SetTitle { title, .. } => {
                self.session_name = Some(title.clone());
                return;
            }
            _ => {}
        }

        if let Some(id) = request.id() {
            let id_str = id.to_string();
            match &request {
                ExtensionUiRequest::Confirm { .. } => {
                    self.dialog_states
                        .insert(id_str.clone(), DialogState::Confirm);
                    self.pending_ui_requests
                        .insert(id_str.clone(), request.clone());
                }
                ExtensionUiRequest::Select { options, .. } => {
                    let selected = if options.is_empty() { None } else { Some(0) };
                    self.dialog_states
                        .insert(id_str.clone(), DialogState::Select { selected });
                    self.pending_ui_requests
                        .insert(id_str.clone(), request.clone());
                }
                ExtensionUiRequest::Input { placeholder, .. } => {
                    self.dialog_states.insert(
                        id_str.clone(),
                        DialogState::Input {
                            value: String::new(),
                            placeholder: placeholder.clone(),
                        },
                    );
                    self.pending_ui_requests
                        .insert(id_str.clone(), request.clone());
                }
                ExtensionUiRequest::Editor { prefill, .. } => {
                    let content = cosmic::iced::widget::text_editor::Content::with_text(
                        prefill.as_deref().unwrap_or(""),
                    );
                    self.dialog_states
                        .insert(id_str.clone(), DialogState::Editor { content });
                    self.pending_ui_requests
                        .insert(id_str.clone(), request.clone());
                }
                _ => {}
            }

            if self.active_dialog_id.is_none() && request.is_dialog() {
                self.active_dialog_id = Some(id_str.clone());
                self.status = AgentStatus::AwaitingUser { request_id: id_str };
            }
        }
    }

    /// Returns the active Extension UI dialog element, if any.
    pub fn view_active_dialog(&self) -> Option<Element<'_, PiAction>> {
        let id = self.active_dialog_id.as_ref()?;
        let request = self.pending_ui_requests.get(id)?;
        let state = self.dialog_states.get(id)?;

        match (request, state) {
            (ExtensionUiRequest::Confirm { title, message, .. }, DialogState::Confirm) => {
                Some(ConfirmDialog::view(id, title, message))
            }
            (
                ExtensionUiRequest::Select { title, options, .. },
                DialogState::Select { selected },
            ) => Some(SelectDialog::view(id, title, options, *selected)),
            (
                ExtensionUiRequest::Input {
                    title, placeholder, ..
                },
                DialogState::Input { value, .. },
            ) => Some(InputDialog::view(
                id,
                title,
                placeholder.as_deref(),
                value.as_str(),
            )),
            (ExtensionUiRequest::Editor { title, .. }, DialogState::Editor { content }) => {
                Some(EditorDialog::view(id, title, content))
            }
            _ => None,
        }
    }

    /// Renders the complete streaming chat view with default chronological order.
    pub fn view_chat(&self) -> Element<'_, PiAction> {
        self.view_chat_ordered(false)
    }

    /// Renders the complete streaming chat view with ordering preference.
    pub fn view_chat_ordered(&self, newest_first: bool) -> Element<'_, PiAction> {
        ChatView::view_ordered(&self.history, newest_first)
    }

    /// Renders the current status indicator badge.
    pub fn view_status_badge(&self) -> Element<'_, PiAction> {
        StatusBadge::view(&self.status)
    }
}

fn apply_tool_output_update(
    partial_result: &mut Option<serde_json::Value>,
    update: &ToolOutputUpdate,
) {
    if let Some(ref set_text) = update.set {
        *partial_result = Some(serde_json::Value::String(set_text.clone()));
        return;
    }

    let mut current = partial_result
        .as_ref()
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    if let Some(trim) = update.trim_start {
        if trim >= current.len() {
            current.clear();
        } else {
            // Drain up to `trim` UTF-8 characters / bytes safely
            let byte_idx = current
                .char_indices()
                .nth(trim)
                .map(|(i, _)| i)
                .unwrap_or(current.len());
            current.drain(..byte_idx);
        }
    }

    if let Some(ref append_text) = update.append {
        current.push_str(append_text);
    }

    *partial_result = Some(serde_json::Value::String(current));
}
