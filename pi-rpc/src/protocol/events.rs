use std::collections::HashMap;
use serde::{Deserialize, Serialize};

use super::commands::ThinkingLevel;

/// Represents an agent message across all roles (system, user, assistant, toolResult).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentMessage {
    pub role: String,
    #[serde(default)]
    pub content: serde_json::Value,
    #[serde(default)]
    pub timestamp: Option<u64>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

impl AgentMessage {
    /// Returns the message role as a string slice (e.g., "user", "assistant", "system", "toolResult").
    pub fn role(&self) -> &str {
        &self.role
    }

    /// Extracts any textual content from this message.
    /// Handles both raw string content and structured blocks (like `[{"type": "text", "text": "..."}]`).
    pub fn text(&self) -> String {
        match &self.content {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Array(items) => {
                let mut out = String::new();
                for item in items {
                    if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                        out.push_str(text);
                    }
                }
                out
            }
            _ => String::new(),
        }
    }
}

/// Cumulative token usage and pricing reported by model providers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    #[serde(default)]
    pub input: u64,
    #[serde(default)]
    pub output: u64,
    #[serde(default)]
    pub cache_read: u64,
    #[serde(default)]
    pub cache_write: u64,
    #[serde(default)]
    pub cache_write1h: Option<u64>,
    #[serde(default)]
    pub reasoning: Option<u64>,
    #[serde(default)]
    pub total_tokens: u64,
    #[serde(default)]
    pub cost: Option<Cost>,
}

/// Cost breakdown for model usage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cost {
    #[serde(default)]
    pub input: f64,
    #[serde(default)]
    pub output: f64,
    #[serde(default)]
    pub cache_read: f64,
    #[serde(default)]
    pub cache_write: f64,
    #[serde(default)]
    pub total: f64,
}

/// Streaming delta event nested inside a `message_update` event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AssistantMessageEvent {
    #[serde(rename = "start")]
    Start,

    #[serde(rename = "text_start")]
    TextStart {
        #[serde(rename = "contentIndex")]
        content_index: usize,
    },

    #[serde(rename = "text_delta")]
    TextDelta {
        #[serde(rename = "contentIndex")]
        content_index: usize,
        delta: String,
    },

    #[serde(rename = "text_end")]
    TextEnd {
        #[serde(rename = "contentIndex")]
        content_index: usize,
        #[serde(default)]
        content: serde_json::Value,
    },

    #[serde(rename = "thinking_start")]
    ThinkingStart {
        #[serde(rename = "contentIndex")]
        content_index: usize,
    },

    #[serde(rename = "thinking_delta")]
    ThinkingDelta {
        #[serde(rename = "contentIndex")]
        content_index: usize,
        delta: String,
    },

    #[serde(rename = "thinking_end")]
    ThinkingEnd {
        #[serde(rename = "contentIndex")]
        content_index: usize,
        #[serde(default)]
        content: serde_json::Value,
    },

    #[serde(rename = "toolcall_start")]
    ToolcallStart {
        #[serde(rename = "contentIndex")]
        content_index: usize,
        id: String,
        #[serde(rename = "toolName")]
        tool_name: String,
    },

    #[serde(rename = "toolcall_delta")]
    ToolcallDelta {
        #[serde(rename = "contentIndex")]
        content_index: usize,
        delta: String,
    },

    #[serde(rename = "toolcall_end")]
    ToolcallEnd {
        #[serde(rename = "contentIndex")]
        content_index: usize,
        #[serde(rename = "toolCall")]
        tool_call: serde_json::Value,
    },

    #[serde(rename = "done")]
    Done {
        reason: String,
        #[serde(default)]
        message: Option<AgentMessage>,
    },

    #[serde(rename = "error")]
    Error {
        reason: String,
        error: String,
    },
}

/// Strongly typed session events emitted by `pi` on stdout during execution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum PiEvent {
    #[serde(rename = "agent_start")]
    AgentStart,

    #[serde(rename = "agent_end")]
    AgentEnd {
        #[serde(default)]
        messages: Vec<AgentMessage>,
        #[serde(rename = "willRetry", default)]
        will_retry: bool,
    },

    #[serde(rename = "agent_settled")]
    AgentSettled {
        #[serde(default)]
        aborted: bool,
    },

    #[serde(rename = "turn_start")]
    TurnStart,

    #[serde(rename = "turn_end")]
    TurnEnd {
        message: AgentMessage,
        #[serde(rename = "toolResults", default)]
        tool_results: Vec<serde_json::Value>,
    },

    #[serde(rename = "message_start")]
    MessageStart {
        message: AgentMessage,
    },

    #[serde(rename = "message_update")]
    MessageUpdate {
        #[serde(default)]
        usage: Option<Usage>,
        #[serde(rename = "assistantMessageEvent")]
        assistant_message_event: AssistantMessageEvent,
    },

    #[serde(rename = "message_end")]
    MessageEnd {
        message: AgentMessage,
    },

    #[serde(rename = "tool_execution_start")]
    ToolExecutionStart {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        #[serde(rename = "toolName")]
        tool_name: String,
        #[serde(default)]
        args: serde_json::Value,
    },

    #[serde(rename = "tool_execution_update")]
    ToolExecutionUpdate {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        #[serde(rename = "toolName")]
        tool_name: String,
        #[serde(default)]
        args: serde_json::Value,
        #[serde(rename = "partialResult", default)]
        partial_result: Option<serde_json::Value>,
    },

    #[serde(rename = "tool_execution_end")]
    ToolExecutionEnd {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        #[serde(rename = "toolName")]
        tool_name: String,
        #[serde(default)]
        result: Option<serde_json::Value>,
        #[serde(rename = "isError", default)]
        is_error: bool,
        #[serde(rename = "durationMs", default)]
        duration_ms: Option<f64>,
    },

    #[serde(rename = "queue_update")]
    QueueUpdate {
        #[serde(default)]
        steering: Vec<String>,
        #[serde(rename = "followUp", default)]
        follow_up: Vec<String>,
    },

    #[serde(rename = "entry_appended")]
    EntryAppended {
        entry: serde_json::Value,
    },

    #[serde(rename = "session_info_changed")]
    SessionInfoChanged {
        #[serde(default)]
        name: Option<String>,
    },

    #[serde(rename = "thinking_level_changed")]
    ThinkingLevelChanged {
        level: ThinkingLevel,
    },

    #[serde(rename = "compaction_start")]
    CompactionStart {
        reason: String,
    },

    #[serde(rename = "compaction_end")]
    CompactionEnd {
        reason: String,
        #[serde(default)]
        result: Option<serde_json::Value>,
        #[serde(default)]
        aborted: bool,
        #[serde(rename = "willRetry", default)]
        will_retry: bool,
        #[serde(rename = "errorMessage", default)]
        error_message: Option<String>,
    },

    #[serde(rename = "auto_retry_start")]
    AutoRetryStart {
        attempt: u32,
        #[serde(rename = "maxAttempts")]
        max_attempts: u32,
        #[serde(rename = "delayMs")]
        delay_ms: u64,
        #[serde(rename = "errorMessage")]
        error_message: String,
    },

    #[serde(rename = "auto_retry_end")]
    AutoRetryEnd {
        success: bool,
        attempt: u32,
        #[serde(rename = "finalError", default)]
        final_error: Option<String>,
    },

    #[serde(rename = "summarization_retry_scheduled")]
    SummarizationRetryScheduled {
        attempt: u32,
        #[serde(rename = "maxAttempts")]
        max_attempts: u32,
        #[serde(rename = "delayMs")]
        delay_ms: u64,
        #[serde(rename = "errorMessage")]
        error_message: String,
    },

    #[serde(rename = "summarization_retry_attempt_start")]
    SummarizationRetryAttemptStart {
        source: String,
        #[serde(default)]
        reason: Option<String>,
    },

    #[serde(rename = "summarization_retry_finished")]
    SummarizationRetryFinished,

    #[serde(rename = "bash_execution_update")]
    BashExecutionUpdate {
        #[serde(default)]
        id: Option<String>,
        delta: String,
    },

    #[serde(rename = "extension_error")]
    ExtensionError {
        #[serde(rename = "extensionPath")]
        extension_path: String,
        event: String,
        error: String,
    },

    /// Fallback for new or unknown session events.
    #[serde(other)]
    Unknown,
}
