use serde::{Deserialize, Serialize};

/// Notification level for Extension UI notifications.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NotifyType {
    Info,
    Warning,
    Error,
}

/// Placement for extension custom widgets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WidgetPlacement {
    AboveEditor,
    BelowEditor,
}

/// Incoming requests emitted by extensions requesting UI interaction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "method")]
pub enum ExtensionUiRequest {
    #[serde(rename = "select")]
    Select {
        id: String,
        title: String,
        options: Vec<String>,
        #[serde(default)]
        timeout: Option<u64>,
    },

    #[serde(rename = "confirm")]
    Confirm {
        id: String,
        title: String,
        message: String,
        #[serde(default)]
        timeout: Option<u64>,
    },

    #[serde(rename = "input")]
    Input {
        id: String,
        title: String,
        #[serde(default)]
        placeholder: Option<String>,
        #[serde(default)]
        timeout: Option<u64>,
    },

    #[serde(rename = "editor")]
    Editor {
        id: String,
        title: String,
        #[serde(default)]
        prefill: Option<String>,
    },

    #[serde(rename = "notify")]
    Notify {
        #[serde(default)]
        id: Option<String>,
        message: String,
        #[serde(rename = "notifyType", default)]
        notify_type: Option<NotifyType>,
    },

    #[serde(rename = "setStatus")]
    SetStatus {
        #[serde(default)]
        id: Option<String>,
        #[serde(rename = "statusKey")]
        status_key: String,
        #[serde(rename = "statusText", default)]
        status_text: Option<String>,
    },

    #[serde(rename = "setWidget")]
    SetWidget {
        #[serde(default)]
        id: Option<String>,
        #[serde(rename = "widgetKey")]
        widget_key: String,
        #[serde(rename = "widgetLines", default)]
        widget_lines: Option<Vec<String>>,
        #[serde(rename = "widgetPlacement", default)]
        widget_placement: Option<WidgetPlacement>,
    },

    #[serde(rename = "setTitle")]
    SetTitle {
        #[serde(default)]
        id: Option<String>,
        title: String,
    },

    #[serde(rename = "set_editor_text")]
    SetEditorText {
        #[serde(default)]
        id: Option<String>,
        text: String,
    },
}

impl ExtensionUiRequest {
    /// Returns the request ID, if present.
    pub fn id(&self) -> Option<&str> {
        match self {
            Self::Select { id, .. }
            | Self::Confirm { id, .. }
            | Self::Input { id, .. }
            | Self::Editor { id, .. } => Some(id.as_str()),
            Self::Notify { id, .. }
            | Self::SetStatus { id, .. }
            | Self::SetWidget { id, .. }
            | Self::SetTitle { id, .. }
            | Self::SetEditorText { id, .. } => id.as_deref(),
        }
    }

    /// Returns `true` if this is an interactive dialog method that requires an `ExtensionUiResponse`.
    pub fn is_dialog(&self) -> bool {
        matches!(
            self,
            Self::Select { .. } | Self::Confirm { .. } | Self::Input { .. } | Self::Editor { .. }
        )
    }
}

/// Internal payload for `ExtensionUiResponse`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ExtensionUiResponsePayload {
    Value { value: String },
    Confirm { confirmed: bool },
    Cancel { cancelled: bool },
}

/// Response sent to Pi on stdin resolving an `ExtensionUiRequest`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtensionUiResponse {
    #[serde(rename = "type")]
    pub record_type: String, // "extension_ui_response"
    pub id: String,
    #[serde(flatten)]
    pub payload: ExtensionUiResponsePayload,
}

impl ExtensionUiResponse {
    /// Create a value response (for `select`, `input`, or `editor`).
    pub fn value(id: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            record_type: "extension_ui_response".to_string(),
            id: id.into(),
            payload: ExtensionUiResponsePayload::Value {
                value: value.into(),
            },
        }
    }

    /// Create a confirmation response (for `confirm`).
    pub fn confirm(id: impl Into<String>, confirmed: bool) -> Self {
        Self {
            record_type: "extension_ui_response".to_string(),
            id: id.into(),
            payload: ExtensionUiResponsePayload::Confirm { confirmed },
        }
    }

    /// Create a cancellation response (for any interactive dialog).
    pub fn cancel(id: impl Into<String>) -> Self {
        Self {
            record_type: "extension_ui_response".to_string(),
            id: id.into(),
            payload: ExtensionUiResponsePayload::Cancel { cancelled: true },
        }
    }
}
