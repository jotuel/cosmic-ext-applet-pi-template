use std::collections::HashMap;
use std::sync::Arc;

use cosmic::iced::Task;
use cosmic::Element;
use pi_rpc::protocol::{
    AssistantMessageEvent, ExtensionUiRequest, ExtensionUiResponse, PiEvent, ThinkingLevel, Usage,
};
use pi_rpc::PiClient;
use uuid::Uuid;

use crate::dialogs::{ConfirmDialog, DialogState, EditorDialog, InputDialog, SelectDialog};
use crate::subscription::PiAction;
use crate::widget::{ChatView, StatusBadge};

/// High-level operational status of the Pi agent.
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

/// Human-readable metadata for a built-in or discovered slash command.
#[derive(Debug, Clone, PartialEq)]
pub struct SlashCommandInfo {
    pub name: String,
    pub description: String,
    pub is_builtin: bool,
}

/// Session controller managing Pi RPC connection, session state, chat history,
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
}

impl PiSessionController {
    /// Creates a new `PiSessionController` wrapping a running `PiClient`.
    pub fn new(client: Arc<PiClient>) -> Self {
        Self {
            client,
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
        }
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
                    tracing::warn!("Failed to fetch discoverable commands from Pi RPC: {e}");
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
                            .unwrap_or(0.0);
                        let (input, output, cache_read, cache_write) =
                            if let Some(toks) = stats.extra.get("tokens") {
                                (
                                    toks.get("input").and_then(|v| v.as_u64()).unwrap_or(0),
                                    toks.get("output").and_then(|v| v.as_u64()).unwrap_or(0),
                                    toks.get("cacheRead").and_then(|v| v.as_u64()).unwrap_or(0),
                                    toks.get("cacheWrite").and_then(|v| v.as_u64()).unwrap_or(0),
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
                        self.status = AgentStatus::Thinking;
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

    /// Submits a prompt request to Pi, intercepting client-side slash commands
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

    /// Submits a prompt to Pi, intercepting client-side slash commands (e.g. `/help`, `/compact`, `/new`)
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
                Task::perform(async move { client.new_session(None).await }, |res| {
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
                                        found
                                            .provider
                                            .clone()
                                            .unwrap_or_else(|| "anthropic".to_string()),
                                        found.id.clone(),
                                    )
                                } else {
                                    ("anthropic".to_string(), target.clone())
                                }
                            } else {
                                ("anthropic".to_string(), target.clone())
                            };
                            client.set_model(provider, model_id).await
                        },
                        |res| PiAction::ModelChanged(res.map(|m| m.id).map_err(|e| e.to_string())),
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

    /// Internal helper to update state from incoming `PiEvent`s.
    fn handle_event(&mut self, event: PiEvent) {
        match event {
            PiEvent::AgentStart => {
                self.status = AgentStatus::Thinking;
            }

            PiEvent::AgentSettled { aborted } => {
                self.status = AgentStatus::Idle;
                for msg in &mut self.history {
                    if msg.is_streaming {
                        msg.is_streaming = false;
                        msg.update_markdown_cache();
                    }
                }
                // Drop any trailing empty assistant turn
                if let Some(last) = self.history.last() {
                    if last.role == "assistant" && last.blocks.is_empty() {
                        self.history.pop();
                    }
                }
                if aborted {
                    tracing::info!("Agent run was aborted");
                }
            }

            PiEvent::AgentEnd { will_retry, .. } => {
                if !will_retry {
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
            }

            PiEvent::TurnStart => {
                let needs_new = match self.history.last() {
                    Some(msg) => msg.role != "assistant" || !msg.is_streaming,
                    None => true,
                };
                if needs_new {
                    self.history.push(ChatMessage::assistant());
                }
            }

            PiEvent::TurnEnd { .. } => {
                if let Some(last) = self.history.last_mut() {
                    last.is_streaming = false;
                    last.update_markdown_cache();
                }
                // Drop empty assistant turns (0 blocks)
                if let Some(last) = self.history.last() {
                    if last.role == "assistant" && last.blocks.is_empty() {
                        self.history.pop();
                    }
                }
            }

            PiEvent::MessageStart { message } => {
                if message.role == "assistant" {
                    let needs_new = match self.history.last() {
                        Some(msg) => msg.role != "assistant" || !msg.is_streaming,
                        None => true,
                    };
                    if needs_new {
                        let mut msg = ChatMessage::assistant();
                        msg.timestamp = message.timestamp;
                        self.history.push(msg);
                    } else if let Some(last) = self.history.last_mut() {
                        if last.timestamp.is_none() {
                            last.timestamp = message.timestamp;
                        }
                    }
                } else if message.role == "user" {
                    // If an empty assistant message was preemptively pushed by TurnStart, remove it
                    if let Some(last) = self.history.last() {
                        if last.role == "assistant" && last.blocks.is_empty() {
                            self.history.pop();
                        }
                    }

                    let text = message.text();
                    let already_exists = self.history.last().is_some_and(|m| {
                        m.role == "user" && m.text() == text
                    });
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

            PiEvent::MessageUpdate {
                usage,
                assistant_message_event,
            } => {
                if let Some(u) = usage {
                    self.usage = Some(u);
                }
                self.handle_assistant_message_event(assistant_message_event);
            }

            PiEvent::MessageEnd { message } => {
                if message.role == "assistant" {
                    if let Some(last) = self.history.last_mut() {
                        if last.role == "assistant" {
                            last.is_streaming = false;
                            last.timestamp = message.timestamp;
                            last.update_markdown_cache();
                        }
                    }
                } else if message.role == "user" {
                    if let Some(last) = self.history.last_mut() {
                        if last.role == "user" && last.timestamp.is_none() {
                            last.timestamp = message.timestamp;
                        }
                    }
                }
            }

            PiEvent::ToolExecutionStart {
                tool_call_id,
                tool_name,
                args,
            } => {
                self.status = AgentStatus::ExecutingTool {
                    name: tool_name.clone(),
                    tool_call_id: tool_call_id.clone(),
                };
                self.update_tool_call_in_history(&tool_call_id, |tool| {
                    tool.is_executing = true;
                    if !args.is_null() {
                        tool.args = args;
                    }
                });
            }

            PiEvent::ToolExecutionUpdate {
                tool_call_id,
                partial_result,
                ..
            } => {
                self.update_tool_call_in_history(&tool_call_id, |tool| {
                    tool.partial_result = partial_result;
                });
            }

            PiEvent::ToolExecutionEnd {
                tool_call_id,
                result,
                is_error,
                duration_ms,
                ..
            } => {
                self.update_tool_call_in_history(&tool_call_id, |tool| {
                    tool.result = result;
                    tool.is_error = is_error;
                    tool.duration_ms = duration_ms;
                    tool.is_executing = false;
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

            PiEvent::SessionInfoChanged { name } => {
                self.session_name = name;
            }

            PiEvent::ThinkingLevelChanged { level } => {
                self.thinking_level = Some(level);
            }

            PiEvent::QueueUpdate {
                steering,
                follow_up,
            } => {
                self.steering_queue = steering;
                self.follow_up_queue = follow_up;
            }

            _ => {}
        }
    }

    /// Updates or appends streaming assistant content.
    fn handle_assistant_message_event(&mut self, event: AssistantMessageEvent) {
        let msg = match self.history.last_mut() {
            Some(m) if m.role == "assistant" => m,
            _ => {
                self.history.push(ChatMessage::assistant());
                self.history.last_mut().unwrap()
            }
        };

        match event {
            AssistantMessageEvent::Start => {
                self.status = AgentStatus::Thinking;
            }

            AssistantMessageEvent::ThinkingStart { .. } => {
                self.status = AgentStatus::Thinking;
                msg.blocks.push(MessageBlock::Thinking {
                    text: String::new(),
                    collapsed: true,
                    is_streaming: true,
                });
            }

            AssistantMessageEvent::ThinkingDelta { delta, .. } => {
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

            AssistantMessageEvent::ThinkingEnd { .. } => {
                for block in msg.blocks.iter_mut().rev() {
                    if let MessageBlock::Thinking { is_streaming, .. } = block {
                        *is_streaming = false;
                        break;
                    }
                }
            }

            AssistantMessageEvent::TextStart { .. } => {
                msg.blocks.push(MessageBlock::Text {
                    text: String::new(),
                    parsed: Vec::new(),
                });
            }

            AssistantMessageEvent::TextDelta { delta, .. } => {
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

            AssistantMessageEvent::TextEnd { .. } => {
                for block in msg.blocks.iter_mut().rev() {
                    if let MessageBlock::Text { text, parsed } = block {
                        *parsed = cosmic::iced::widget::markdown::parse(text).collect();
                        break;
                    }
                }
            }

            AssistantMessageEvent::ToolcallStart { id, tool_name, .. } => {
                msg.blocks.push(MessageBlock::ToolCall(ToolCallData {
                    id,
                    name: tool_name,
                    args: serde_json::Value::Null,
                    args_raw: String::new(),
                    partial_result: None,
                    result: None,
                    is_error: false,
                    duration_ms: None,
                    collapsed: true,
                    is_executing: true,
                }));
            }

            AssistantMessageEvent::ToolcallDelta { delta, .. } => {
                for block in msg.blocks.iter_mut().rev() {
                    if let MessageBlock::ToolCall(tool) = block {
                        tool.args_raw.push_str(&delta);
                        if let Ok(parsed) =
                            serde_json::from_str::<serde_json::Value>(&tool.args_raw)
                        {
                            tool.args = parsed;
                        }
                        break;
                    }
                }
            }

            AssistantMessageEvent::ToolcallEnd { tool_call, .. } => {
                for block in msg.blocks.iter_mut().rev() {
                    if let MessageBlock::ToolCall(tool) = block {
                        tool.args = tool_call;
                        break;
                    }
                }
            }

            AssistantMessageEvent::Done { .. } => {
                msg.is_streaming = false;
                msg.update_markdown_cache();
            }

            AssistantMessageEvent::Error { error, .. } => {
                msg.is_streaming = false;
                self.status = AgentStatus::Error(error);
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
