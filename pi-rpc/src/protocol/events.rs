use std::collections::HashMap;
use serde::{Deserialize, Serialize};

use super::commands::ThinkingLevel;

/// Provider and model identifier pair stored in `pi.agent`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRef {
    pub provider: String,
    pub model_id: String,
}

impl ModelRef {
    pub fn new(provider: impl Into<String>, model_id: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            model_id: model_id.into(),
        }
    }

    pub fn full_id(&self) -> String {
        format!("{}/{}", self.provider, self.model_id)
    }
}

/// Stored conversation agent configuration (`pi.agent`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentState {
    #[serde(default)]
    pub model: Option<ModelRef>,
    #[serde(default)]
    pub thinking_level: Option<ThinkingLevel>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub instructions: Option<String>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

/// Represents an agent message across all roles (`system`, `user`, `assistant`, `toolResult`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentMessage {
    pub role: String,
    #[serde(default)]
    pub content: serde_json::Value,
    #[serde(default)]
    pub timestamp: Option<u64>,
    #[serde(default)]
    pub usage: Option<Usage>,
    #[serde(default)]
    pub tool_call_id: Option<String>,
    #[serde(default)]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub is_error: Option<bool>,
    #[serde(default)]
    pub duration_ms: Option<f64>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

impl AgentMessage {
    /// Returns the message role as a string slice (`"user"`, `"assistant"`, `"system"`, `"toolResult"`).
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
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
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

impl Usage {
    pub fn add(&mut self, other: &Usage) {
        self.input += other.input;
        self.output += other.output;
        self.cache_read += other.cache_read;
        self.cache_write += other.cache_write;
        self.total_tokens += other.total_tokens;
        if let Some(c) = other.cache_write1h {
            *self.cache_write1h.get_or_insert(0) += c;
        }
        if let Some(r) = other.reasoning {
            *self.reasoning.get_or_insert(0) += r;
        }
        if let Some(ref other_cost) = other.cost {
            let cost = self.cost.get_or_insert_with(Cost::default);
            cost.input += other_cost.input;
            cost.output += other_cost.output;
            cost.cache_read += other_cost.cache_read;
            cost.cache_write += other_cost.cache_write;
            cost.total += other_cost.total;
        }
    }
}

/// Cost breakdown for model usage.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
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

/// Conversation usage ledger (`pi.usage`) keyed by model (`provider/modelId`) and tool name.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageState {
    #[serde(default)]
    pub models: HashMap<String, Usage>,
    #[serde(default)]
    pub tools: HashMap<String, Usage>,
}

impl UsageState {
    /// Sums all model and tool usage buckets into a single aggregate `Usage`.
    pub fn total_usage(&self) -> Usage {
        let mut total = Usage::default();
        for u in self.models.values() {
            total.add(u);
        }
        for u in self.tools.values() {
            total.add(u);
        }
        total
    }
}

/// Immutable transcript entry record stored by `pi-durable`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryRecord {
    pub id: u64,
    pub conversation_id: u64,
    pub kind: String,
    #[serde(default)]
    pub model: Option<Vec<AgentMessage>>,
    #[serde(default)]
    pub data: Option<serde_json::Value>,
    #[serde(default)]
    pub head: Option<u64>,
    #[serde(default)]
    pub by_task_id: Option<u64>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

/// Diagnostic remark attached to a tool call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolDiagnostic {
    pub severity: String,
    pub message: String,
    #[serde(default)]
    pub code: Option<String>,
}

/// Live presentation of a running or completed tool call in the current round (`pi.live.tools`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolSlot {
    pub call_id: String,
    pub name: String,
    pub status: String,
    #[serde(default)]
    pub output: Option<String>,
    #[serde(default)]
    pub dropped_bytes: Option<u64>,
    #[serde(default)]
    pub dropped_lines: Option<u64>,
    #[serde(default)]
    pub details: Option<serde_json::Value>,
    #[serde(default)]
    pub diagnostics: Option<Vec<ToolDiagnostic>>,
    #[serde(default)]
    pub task_id: Option<u64>,
    #[serde(default)]
    pub entry: Option<u64>,
}

/// Summary of how a nested tool call ended.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NestedToolSummary {
    pub is_error: bool,
    #[serde(default)]
    pub duration_ms: Option<f64>,
    #[serde(default)]
    pub usage: Option<Usage>,
    #[serde(default)]
    pub error: Option<String>,
}

/// Live presentation of a nested tool call (`pi.live.nestedTools`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NestedToolSlot {
    pub call_id: String,
    pub name: String,
    pub status: String,
    pub parent_call_id: String,
    pub parent_task_id: u64,
    pub task_id: u64,
    #[serde(default)]
    pub arguments: serde_json::Value,
    #[serde(default)]
    pub output: Option<String>,
    #[serde(default)]
    pub details: Option<serde_json::Value>,
    #[serde(default)]
    pub diagnostics: Option<Vec<ToolDiagnostic>>,
    #[serde(default)]
    pub summary: Option<NestedToolSummary>,
}

/// Active run state (`pi.live.run`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunStatus {
    #[serde(default)]
    pub inputs: Vec<u64>,
}

/// Retry backoff status in `pi.live`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetryStatus {
    pub at: u64,
    pub error: String,
}

/// Provider-side deferred response poll status in `pi.live`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeferredStatus {
    pub poll_at: u64,
}

/// Current generation attempt in `pi.live.generation`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationStatus {
    pub attempt: u32,
    #[serde(default)]
    pub message: Option<AgentMessage>,
    #[serde(default)]
    pub retry: Option<RetryStatus>,
    #[serde(default)]
    pub deferred: Option<DeferredStatus>,
}

/// Live compaction task status (`pi.live.compactions`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactionStatus {
    pub task_id: u64,
    pub reason: String,
    #[serde(default)]
    pub blocking: bool,
    pub attempt: u32,
    #[serde(default)]
    pub retry: Option<RetryStatus>,
}

/// Newest compaction failure (`pi.live.compactionFailure`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactionFailure {
    pub task_id: u64,
    pub reason: String,
    pub error: String,
}

/// Queue mode of an item in `pi.inbox`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InboxMode {
    Steer,
    FollowUp,
    Write,
}

/// A queued item in `pi.inbox`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueuedItem {
    pub id: u64,
    pub mode: InboxMode,
    #[serde(default)]
    pub text: Option<String>,
}

/// Initial or overflow conversation snapshot emitted by `watchEvents` (`type: "snapshot"`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotEvent {
    #[serde(default)]
    pub entries: Vec<EntryRecord>,
    #[serde(default)]
    pub run: Option<RunStatus>,
    #[serde(default)]
    pub generation: Option<GenerationStatus>,
    #[serde(default)]
    pub tools: Vec<ToolSlot>,
    #[serde(default)]
    pub nested_tools: Vec<NestedToolSlot>,
    #[serde(default)]
    pub compactions: Vec<CompactionStatus>,
    #[serde(default)]
    pub compaction_failure: Option<CompactionFailure>,
    #[serde(default)]
    pub inbox: Vec<QueuedItem>,
    #[serde(default)]
    pub agent: AgentState,
    #[serde(default)]
    pub usage: UsageState,
}

/// One change to the in-flight assistant message inside `PiEvent::MessageUpdate`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum MessageChange {
    #[serde(rename = "text_start")]
    TextStart {
        #[serde(rename = "contentIndex")]
        content_index: usize,
        #[serde(default)]
        block: serde_json::Value,
    },

    #[serde(rename = "thinking_start")]
    ThinkingStart {
        #[serde(rename = "contentIndex")]
        content_index: usize,
        #[serde(default)]
        block: serde_json::Value,
    },

    #[serde(rename = "toolcall_start")]
    ToolcallStart {
        #[serde(rename = "contentIndex")]
        content_index: usize,
        #[serde(default)]
        block: serde_json::Value,
    },

    #[serde(rename = "text_delta")]
    TextDelta {
        #[serde(rename = "contentIndex")]
        content_index: usize,
        delta: String,
    },

    #[serde(rename = "thinking_delta")]
    ThinkingDelta {
        #[serde(rename = "contentIndex")]
        content_index: usize,
        delta: String,
    },

    #[serde(rename = "toolcall_delta")]
    ToolcallDelta {
        #[serde(rename = "contentIndex")]
        content_index: usize,
        #[serde(default)]
        path: Vec<serde_json::Value>,
        delta: String,
    },

    #[serde(rename = "block")]
    Block {
        #[serde(rename = "contentIndex")]
        content_index: usize,
        block: serde_json::Value,
    },

    #[serde(rename = "message")]
    Message {
        message: AgentMessage,
    },
}

/// Incremental tool output update inside `PiEvent::ToolExecutionUpdate`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolOutputUpdate {
    #[serde(default)]
    pub trim_start: Option<usize>,
    #[serde(default)]
    pub append: Option<String>,
    #[serde(default)]
    pub set: Option<String>,
}

/// Result of a nested tool call inside `PiEvent::ToolExecutionEnd`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NestedToolExecutionResult {
    pub task_id: u64,
    #[serde(default)]
    pub structured_output: Option<serde_json::Value>,
    #[serde(default)]
    pub is_error: bool,
    #[serde(default)]
    pub details: Option<serde_json::Value>,
    #[serde(default)]
    pub diagnostics: Vec<ToolDiagnostic>,
    #[serde(default)]
    pub usage: Option<Usage>,
    #[serde(default)]
    pub duration_ms: Option<f64>,
}

/// Durable submission lifecycle record emitted in `PiEvent::Submission`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmissionRecord {
    pub id: u64,
    pub conversation_id: u64,
    #[serde(rename = "type")]
    pub submission_type: String,
    pub status: String,
    #[serde(default)]
    pub request_id: Option<String>,
    #[serde(default)]
    pub entry: Option<u64>,
    #[serde(default)]
    pub answer: Option<u64>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub detail: Option<String>,
}

/// Native `pi-durable` `AgentEvent` stream records emitted on stdout.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum PiEvent {
    #[serde(rename = "snapshot")]
    Snapshot(SnapshotEvent),

    #[serde(rename = "run_start")]
    RunStart {
        #[serde(default)]
        inputs: Vec<u64>,
    },

    #[serde(rename = "run_end")]
    RunEnd {
        #[serde(default)]
        inputs: Vec<u64>,
    },

    #[serde(rename = "turn_start")]
    TurnStart,

    #[serde(rename = "turn_end")]
    TurnEnd,

    #[serde(rename = "message_start")]
    MessageStart {
        message: AgentMessage,
    },

    #[serde(rename = "message_update")]
    MessageUpdate {
        #[serde(default)]
        usage: Option<Usage>,
        #[serde(default)]
        changes: Vec<MessageChange>,
    },

    #[serde(rename = "message_end")]
    MessageEnd {
        entry: EntryRecord,
    },

    #[serde(rename = "tool_execution_start")]
    ToolExecutionStart {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        #[serde(rename = "toolName")]
        tool_name: String,
        #[serde(rename = "taskId", default)]
        task_id: Option<u64>,
        #[serde(rename = "parentToolCallId", default)]
        parent_tool_call_id: Option<String>,
        #[serde(rename = "parentTaskId", default)]
        parent_task_id: Option<u64>,
        #[serde(default)]
        args: serde_json::Value,
    },

    #[serde(rename = "tool_execution_update")]
    ToolExecutionUpdate {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        #[serde(rename = "toolName")]
        tool_name: String,
        #[serde(rename = "taskId", default)]
        task_id: Option<u64>,
        #[serde(rename = "parentToolCallId", default)]
        parent_tool_call_id: Option<String>,
        #[serde(rename = "parentTaskId", default)]
        parent_task_id: Option<u64>,
        #[serde(default)]
        output: Option<ToolOutputUpdate>,
        #[serde(default)]
        details: Option<serde_json::Value>,
        #[serde(default)]
        diagnostics: Option<Vec<ToolDiagnostic>>,
    },

    #[serde(rename = "tool_execution_end")]
    ToolExecutionEnd {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        #[serde(rename = "toolName")]
        tool_name: String,
        #[serde(rename = "taskId", default)]
        task_id: Option<u64>,
        #[serde(rename = "parentToolCallId", default)]
        parent_tool_call_id: Option<String>,
        #[serde(rename = "parentTaskId", default)]
        parent_task_id: Option<u64>,
        #[serde(default)]
        entry: Option<EntryRecord>,
        #[serde(default)]
        result: Option<NestedToolExecutionResult>,
    },

    #[serde(rename = "inbox_update")]
    InboxUpdate {
        #[serde(default)]
        items: Vec<QueuedItem>,
    },

    #[serde(rename = "submission")]
    Submission {
        record: SubmissionRecord,
    },

    #[serde(rename = "auto_retry_start")]
    AutoRetryStart {
        attempt: u32,
        at: u64,
        #[serde(rename = "errorMessage")]
        error_message: String,
    },

    #[serde(rename = "auto_retry_end")]
    AutoRetryEnd {
        attempt: u32,
    },

    #[serde(rename = "deferred_poll")]
    DeferredPoll {
        #[serde(rename = "pollAt")]
        poll_at: u64,
    },

    #[serde(rename = "entry_appended")]
    EntryAppended {
        entry: EntryRecord,
    },

    #[serde(rename = "agent_changed")]
    AgentChanged {
        agent: AgentState,
    },

    #[serde(rename = "usage_changed")]
    UsageChanged {
        usage: UsageState,
    },

    #[serde(rename = "task_failed")]
    TaskFailed {
        #[serde(rename = "taskId")]
        task_id: u64,
        kind: String,
        message: String,
    },

    #[serde(rename = "compaction_start")]
    CompactionStart {
        #[serde(rename = "taskId")]
        task_id: u64,
        reason: String,
        #[serde(default)]
        blocking: bool,
    },

    #[serde(rename = "compaction_end")]
    CompactionEnd {
        #[serde(rename = "taskId")]
        task_id: u64,
        reason: String,
        #[serde(default)]
        error: Option<String>,
    },

    /// Fallback for new or unknown session events.
    #[serde(other)]
    Unknown,
}
