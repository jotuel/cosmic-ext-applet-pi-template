use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::process::ExitStatus;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

use crate::error::PiError;

/// Configuration for launching the `pi` child process in RPC mode.
#[derive(Debug, Clone)]
pub struct PiProcessConfig {
    /// Path to the `pi` executable. Defaults to `"pi"`.
    pub binary_path: PathBuf,
    /// Working directory for the process.
    pub working_dir: Option<PathBuf>,
    /// Model provider override (e.g. "anthropic", "openai", "radius").
    pub provider: Option<String>,
    /// Model identifier override (e.g. "claude-sonnet-4-20250514").
    pub model: Option<String>,
    /// Directory where sessions should be stored.
    pub session_dir: Option<PathBuf>,
    /// Run without persisting sessions (`--no-session`).
    pub no_session: bool,
    /// Additional CLI arguments passed to `pi`.
    pub extra_args: Vec<String>,
    /// Additional environment variables for the child process.
    pub env: HashMap<String, String>,
}

impl Default for PiProcessConfig {
    fn default() -> Self {
        Self {
            binary_path: PathBuf::from("pi"),
            working_dir: None,
            provider: None,
            model: None,
            session_dir: None,
            no_session: false,
            extra_args: Vec::new(),
            env: HashMap::new(),
        }
    }
}

impl PiProcessConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_binary_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.binary_path = path.into();
        self
    }

    pub fn with_working_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.working_dir = Some(dir.into());
        self
    }

    pub fn with_provider(mut self, provider: impl Into<String>) -> Self {
        self.provider = Some(provider.into());
        self
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    pub fn with_session_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.session_dir = Some(dir.into());
        self
    }

    pub fn with_no_session(mut self, no_session: bool) -> Self {
        self.no_session = no_session;
        self
    }

    pub fn with_extra_args(mut self, args: Vec<String>) -> Self {
        self.extra_args = args;
        self
    }

    pub fn with_env(mut self, key: impl Into<String>, val: impl Into<String>) -> Self {
        self.env.insert(key.into(), val.into());
        self
    }
}

/// Handle to the spawned `pi` child process.
pub struct PiProcess {
    child: Child,
    recent_stderr: Arc<Mutex<VecDeque<String>>>,
}

impl PiProcess {
    /// Launches `pi --mode rpc` according to the provided configuration.
    ///
    /// Returns the captured `stdin` and `stdout` pipe streams alongside the supervisor handle.
    pub fn spawn(config: &PiProcessConfig) -> Result<(ChildStdin, ChildStdout, Self), PiError> {
        let mut cmd = Command::new(&config.binary_path);
        cmd.arg("--mode").arg("rpc");

        if config.no_session {
            cmd.arg("--no-session");
        }
        if let Some(ref provider) = config.provider {
            cmd.arg("--provider").arg(provider);
        }
        if let Some(ref model) = config.model {
            cmd.arg("--model").arg(model);
        }
        if let Some(ref session_dir) = config.session_dir {
            cmd.arg("--session-dir").arg(session_dir);
        }
        for arg in &config.extra_args {
            cmd.arg(arg);
        }
        if let Some(ref cwd) = config.working_dir {
            cmd.current_dir(cwd);
        }
        for (k, v) in &config.env {
            cmd.env(k, v);
        }

        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        // Ensure child process is killed if handle drops unexpectedly
        cmd.kill_on_drop(true);

        let mut child = cmd.spawn().map_err(PiError::SpawnFailed)?;

        let stdin = child.stdin.take().ok_or_else(|| {
            PiError::Protocol("Failed to capture child stdin pipe".to_string())
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            PiError::Protocol("Failed to capture child stdout pipe".to_string())
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            PiError::Protocol("Failed to capture child stderr pipe".to_string())
        })?;

        let recent_stderr = Arc::new(Mutex::new(VecDeque::with_capacity(50)));
        let stderr_buf = Arc::clone(&recent_stderr);

        // Background stderr pipe forwarding
        tokio::spawn(async move {
            let mut reader = BufReader::new(stderr);
            let mut line = String::new();
            while let Ok(n) = reader.read_line(&mut line).await {
                if n == 0 {
                    break;
                }
                let trimmed = line.trim_end();
                tracing::debug!(target: "pi_rpc::stderr", "{}", trimmed);
                {
                    if let Ok(mut buf) = stderr_buf.lock() {
                        if buf.len() >= 50 {
                            buf.pop_front();
                        }
                        buf.push_back(trimmed.to_string());
                    }
                }
                line.clear();
            }
        });

        let process = Self {
            child,
            recent_stderr,
        };

        Ok((stdin, stdout, process))
    }

    /// Returns the captured recent lines of stderr.
    pub fn stderr_dump(&self) -> String {
        if let Ok(buf) = self.recent_stderr.lock() {
            buf.iter().cloned().collect::<Vec<_>>().join("\n")
        } else {
            String::new()
        }
    }

    /// Checks if the child process has exited, returning its status if so.
    pub fn try_wait(&mut self) -> Result<Option<ExitStatus>, PiError> {
        self.child.try_wait().map_err(PiError::Io)
    }

    /// Gracefully closes child process, falling back to kill on timeout.
    pub async fn shutdown(mut self, timeout_dur: Duration) -> Result<(), PiError> {
        match tokio::time::timeout(timeout_dur, self.child.wait()).await {
            Ok(Ok(_status)) => Ok(()),
            Ok(Err(e)) => Err(PiError::Io(e)),
            Err(_) => {
                tracing::warn!("Pi child did not exit within {:?}; force killing", timeout_dur);
                let _ = self.child.start_kill();
                let _ = self.child.wait().await;
                Ok(())
            }
        }
    }
}
