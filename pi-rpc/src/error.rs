use std::process::ExitStatus;
use thiserror::Error;

/// Error types for `pi-rpc` operations.
#[derive(Debug, Error)]
pub enum PiError {
    /// Failed to spawn the Pi process.
    #[error("Child process spawn failed: {0}")]
    SpawnFailed(#[source] std::io::Error),

    /// Standard I/O error occurred during communication.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Serialization or deserialization error.
    #[error("JSON serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// Pi returned a response indicating command failure.
    #[error("Command '{command}' rejected: {error}")]
    CommandRejected {
        command: String,
        error: String,
    },

    /// The Pi process exited unexpectedly.
    #[error("Child process exited unexpectedly with status {status:?}: {stderr}")]
    UnexpectedExit {
        status: ExitStatus,
        stderr: String,
    },

    /// Timed out waiting for response from child process.
    #[error("Command timed out")]
    Timeout,

    /// An internal communication channel was dropped or closed.
    #[error("Internal channel closed")]
    ChannelClosed,

    /// Client has been shut down or dropped.
    #[error("Client is shut down")]
    Shutdown,

    /// Protocol error or unexpected message structure.
    #[error("Protocol error: {0}")]
    Protocol(String),
}
