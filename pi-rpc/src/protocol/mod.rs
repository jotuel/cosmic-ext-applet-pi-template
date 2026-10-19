pub mod commands;
pub mod events;
pub mod extension_ui;

pub use commands::*;
pub use events::*;
pub use extension_ui::*;

use serde::Deserialize;
use crate::error::PiError;

/// An incoming JSONL record received from Pi's stdout.
#[derive(Debug, Clone, PartialEq)]
pub enum IncomingRecord {
    /// A command response corresponding to a request sent by the client.
    Response(RawCommandResponse),
    /// An extension UI request requiring interaction or notification.
    ExtensionUi(ExtensionUiRequest),
    /// A streaming session lifecycle, message, tool, or queue event.
    Event(PiEvent),
}

impl<'de> Deserialize<'de> for IncomingRecord {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        let probe = value.get("type").and_then(|t| t.as_str());
        match probe {
            Some("response") => {
                let resp = RawCommandResponse::deserialize(value)
                    .map_err(serde::de::Error::custom)?;
                Ok(IncomingRecord::Response(resp))
            }
            Some("extension_ui_request") => {
                let req = ExtensionUiRequest::deserialize(value)
                    .map_err(serde::de::Error::custom)?;
                Ok(IncomingRecord::ExtensionUi(req))
            }
            _ => {
                let event = PiEvent::deserialize(value)
                    .map_err(serde::de::Error::custom)?;
                Ok(IncomingRecord::Event(event))
            }
        }
    }
}

impl IncomingRecord {
    /// Parses an incoming JSON byte buffer into an `IncomingRecord`.
    pub fn parse(bytes: &[u8]) -> Result<Self, PiError> {
        Ok(serde_json::from_slice(bytes)?)
    }
}
