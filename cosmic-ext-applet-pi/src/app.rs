// SPDX-License-Identifier: EUPL-1.2

use std::path::PathBuf;
use std::sync::Arc;

use cosmic::cosmic_config::{self, CosmicConfigEntry};
use cosmic::iced::platform_specific::shell::wayland::commands::popup::{destroy_popup, get_popup};
use cosmic::iced::{Limits, Subscription, Task, window::Id};
use cosmic::prelude::*;
use cosmic::widget;
use cosmic_ext_pi::controller::AgentStatus;
use cosmic_ext_pi::subscription::pi_subscription;
use cosmic_ext_pi::{PiAction, PiSessionController};
use pi_rpc::PiClient;
use pi_rpc::process::PiProcessConfig;
use pi_rpc::protocol::{ExtensionUiRequest, PiEvent, ThinkingLevel};

use crate::config::Config;
use crate::notifications;
use crate::popover;

/// Application model storing applet state, Pi RPC client/controller,
/// configuration, and popover window state.
#[derive(Default)]
pub struct AppModel {
    /// Runtime core managed by libcosmic.
    core: cosmic::Core,
    /// Active popover window ID, if currently open.
    popup: Option<Id>,
    /// Persisted configuration settings.
    config: Config,
    /// Running background Pi client connection.
    client: Option<Arc<PiClient>>,
    /// Active session controller managing streaming chat, tools, and UI state.
    controller: Option<PiSessionController>,
    /// Buffer for prompt text input.
    prompt_input: String,
    /// Indicates whether a background `pi` subprocess is currently launching.
    client_spawning: bool,
    /// Holds the error message if the child process failed to start.
    spawn_error: Option<String>,
    /// Whether the settings view is currently open in the popover.
    pub show_settings: bool,
}

/// Messages emitted across the applet and its widgets.
#[derive(Clone)]
pub enum Message {
    TogglePopup,
    PopupClosed(Id),
    UpdateConfig(Config),
    ToggleSettings,
    PickDefaultFolder,
    DefaultFolderSelected(Option<PathBuf>),
    ResetDefaultFolder,
    ToggleAutostart(bool),
    ToggleNotifyComplete(bool),
    ToggleNotifyApproval(bool),
    ToggleNewestFirst(bool),
    SpawnPi { cwd: PathBuf },
    PiSpawned(Result<Arc<PiClient>, String>),
    RestartPi,
    PickProjectDirectory,
    ProjectDirectorySelected(Option<PathBuf>),
    SelectRecentProject(PathBuf),
    NewSession,
    CycleThinkingLevel,
    ThinkingLevelCycled(Result<Option<ThinkingLevel>, String>),
    CycleModel,
    ModelCycled(Result<Option<String>, String>),
    SessionStateReceived(pi_rpc::protocol::RpcSessionState),
    PromptInputChanged(String),
    SubmitPrompt,
    SubmitSteer,
    SubmitFollowUp,
    Abort,
    PickFileAttachment,
    FileAttachmentSelected(Option<PathBuf>),
    PiAction(PiAction),
    Noop,
    Surface(cosmic::surface::Action<Message>),
}

impl std::fmt::Debug for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TogglePopup => write!(f, "TogglePopup"),
            Self::PopupClosed(id) => write!(f, "PopupClosed({id:?})"),
            Self::UpdateConfig(c) => write!(f, "UpdateConfig({c:?})"),
            Self::ToggleSettings => write!(f, "ToggleSettings"),
            Self::PickDefaultFolder => write!(f, "PickDefaultFolder"),
            Self::DefaultFolderSelected(p) => write!(f, "DefaultFolderSelected({p:?})"),
            Self::ResetDefaultFolder => write!(f, "ResetDefaultFolder"),
            Self::ToggleAutostart(b) => write!(f, "ToggleAutostart({b})"),
            Self::ToggleNotifyComplete(b) => write!(f, "ToggleNotifyComplete({b})"),
            Self::ToggleNotifyApproval(b) => write!(f, "ToggleNotifyApproval({b})"),
            Self::ToggleNewestFirst(b) => write!(f, "ToggleNewestFirst({b})"),
            Self::SpawnPi { cwd } => write!(f, "SpawnPi({cwd:?})"),
            Self::PiSpawned(Ok(_)) => write!(f, "PiSpawned(Ok)"),
            Self::PiSpawned(Err(e)) => write!(f, "PiSpawned(Err({e}))"),
            Self::RestartPi => write!(f, "RestartPi"),
            Self::PickProjectDirectory => write!(f, "PickProjectDirectory"),
            Self::ProjectDirectorySelected(p) => write!(f, "ProjectDirectorySelected({p:?})"),
            Self::SelectRecentProject(p) => write!(f, "SelectRecentProject({p:?})"),
            Self::NewSession => write!(f, "NewSession"),
            Self::CycleThinkingLevel => write!(f, "CycleThinkingLevel"),
            Self::ThinkingLevelCycled(res) => write!(f, "ThinkingLevelCycled({res:?})"),
            Self::CycleModel => write!(f, "CycleModel"),
            Self::ModelCycled(res) => write!(f, "ModelCycled({res:?})"),
            Self::SessionStateReceived(s) => write!(f, "SessionStateReceived({:?})", s.session_id),
            Self::PromptInputChanged(s) => write!(f, "PromptInputChanged({s})"),
            Self::SubmitPrompt => write!(f, "SubmitPrompt"),
            Self::SubmitSteer => write!(f, "SubmitSteer"),
            Self::SubmitFollowUp => write!(f, "SubmitFollowUp"),
            Self::Abort => write!(f, "Abort"),
            Self::PickFileAttachment => write!(f, "PickFileAttachment"),
            Self::FileAttachmentSelected(p) => write!(f, "FileAttachmentSelected({p:?})"),
            Self::PiAction(a) => write!(f, "PiAction({a:?})"),
            Self::Noop => write!(f, "Noop"),
            Self::Surface(a) => write!(f, "Surface({a:?}"),
        }
    }
}

impl AppModel {
    pub const APP_ID: &'static str = "io.github.jotuel.cosmic-ext-applet-pi";
    pub const ICON_NAME: &'static str = "io.github.jotuel.cosmic-ext-applet-pi-symbolic";

    /// Returns a reference to the active configuration.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Returns the active status of the agent.
    pub fn status(&self) -> AgentStatus {
        if self.client_spawning {
            return AgentStatus::Thinking;
        }
        if let Some(ref err) = self.spawn_error {
            return AgentStatus::Error(err.clone());
        }
        if let Some(ref ctrl) = self.controller {
            return ctrl.status().clone();
        }
        AgentStatus::Idle
    }

    /// Persists configuration to `cosmic-config`.
    fn save_config(&self) {
        #[cfg(test)]
        {
            if std::env::var_os("XDG_CONFIG_HOME").is_none() {
                return;
            }
        }
        if let Ok(context) = cosmic_config::Config::new(Self::APP_ID, Config::VERSION)
            && let Err(e) = self.config.write_entry(&context)
        {
            tracing::warn!("Failed to save config: {e}");
        }
    }
}

impl cosmic::Application for AppModel {
    type Executor = cosmic::executor::Default;
    type Flags = ();
    type Message = Message;

    const APP_ID: &'static str = Self::APP_ID;

    fn core(&self) -> &cosmic::Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut cosmic::Core {
        &mut self.core
    }

    fn init(
        core: cosmic::Core,
        _flags: Self::Flags,
    ) -> (Self, Task<cosmic::Action<Self::Message>>) {
        let mut config = cosmic_config::Config::new(Self::APP_ID, Config::VERSION)
            .map(|context| match Config::get_entry(&context) {
                Ok(config) => config,
                Err((_errors, config)) => config,
            })
            .unwrap_or_default();

        // Sanitize stale or non-existent directories from config
        if config.working_dir.as_ref().is_some_and(|p| !p.is_dir()) {
            config.working_dir = None;
        }
        config.recent_projects.retain(|p| p.is_dir());
        if config.default_folder.is_none() {
            config.default_folder = Some(crate::config::home_dir());
        }

        let should_autostart = config.autostart_daemon;
        let effective_cwd = config.effective_working_dir();

        let app = AppModel {
            core,
            config,
            ..Default::default()
        };

        let task = if should_autostart {
            Task::perform(
                async move { Message::SpawnPi { cwd: effective_cwd } },
                cosmic::Action::from,
            )
        } else {
            Task::none()
        };

        (app, task)
    }

    fn on_close_requested(&self, id: Id) -> Option<Message> {
        Some(Message::PopupClosed(id))
    }

    /// Renders the panel button and live status badge.
    fn view(&self) -> Element<'_, Self::Message> {
        let icon = widget::icon::from_name(Self::ICON_NAME)
            .size(16)
            .with_extra_paths(vec![PathBuf::from(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/resources/icons/scalable/apps"
            ))]);

        let button = self
            .core
            .applet
            .button_from_element(icon, true)
            .on_press(Message::TogglePopup);

        self.core
            .applet
            .applet_tooltip(
                button,
                match self.status() {
                    AgentStatus::Idle => "⌨",
                    AgentStatus::Thinking => "💡",
                    AgentStatus::ExecutingTool { .. } => "🛠",
                    AgentStatus::AwaitingUser { .. } => "⚠",
                    AgentStatus::Error(_) => "⛔",
                },
                self.popup.is_some(),
                Message::Surface,
                self.popup,
            )
            .into()
    }

    /// Renders the flyout popover window when opened from the panel.
    fn view_window(&self, _id: Id) -> Element<'_, Self::Message> {
        let current_model = self
            .controller
            .as_ref()
            .and_then(|c| c.model())
            .or(self.config.default_model.as_deref());

        let popover_content = popover::view_popover(
            &self.config,
            self.controller.as_ref(),
            self.client_spawning,
            self.spawn_error.as_deref(),
            &self.prompt_input,
            current_model,
            self.show_settings,
        );

        let limits = Limits::NONE
            .min_width(popover::POPOVER_WIDTH)
            .max_width(popover::POPOVER_WIDTH)
            .min_height(popover::POPOVER_HEIGHT)
            .max_height(popover::POPOVER_HEIGHT);

        self.core
            .applet
            .popup_container(popover_content)
            .limits(limits)
            .into()
    }

    /// Subscriptions for config changes and Pi RPC events.
    fn subscription(&self) -> Subscription<Self::Message> {
        let mut subs = Vec::new();

        if let Some(ref client) = self.client {
            subs.push(pi_subscription(Arc::clone(client)).map(Message::PiAction));
        }

        subs.push(
            self.core()
                .watch_config::<Config>(Self::APP_ID)
                .map(|update| Message::UpdateConfig(update.config)),
        );

        Subscription::batch(subs)
    }

    /// Updates application state based on incoming messages.
    fn update(&mut self, message: Self::Message) -> Task<cosmic::Action<Self::Message>> {
        match message {
            Message::TogglePopup => {
                if let Some(p) = self.popup.take() {
                    destroy_popup(p)
                } else {
                    let mut spawn_task = Task::none();
                    if self.client.is_none() && !self.client_spawning {
                        let cwd = self.config.effective_working_dir();
                        spawn_task = Task::perform(
                            async move { Message::SpawnPi { cwd } },
                            cosmic::Action::from,
                        );
                    }

                    let new_id = Id::unique();
                    self.popup.replace(new_id);
                    let mut popup_settings = self.core.applet.get_popup_settings(
                        self.core.main_window_id().unwrap(),
                        new_id,
                        None,
                        None,
                        None,
                    );
                    let limits = Limits::NONE
                        .min_width(popover::POPOVER_WIDTH)
                        .max_width(popover::POPOVER_WIDTH)
                        .min_height(popover::POPOVER_HEIGHT)
                        .max_height(popover::POPOVER_HEIGHT);
                    popup_settings.positioner.size_limits = limits;

                    Task::batch(vec![get_popup(popup_settings), spawn_task])
                }
            }

            Message::PopupClosed(id) => {
                if self.popup.as_ref() == Some(&id) {
                    self.popup = None;
                }
                Task::none()
            }

            Message::UpdateConfig(config) => {
                self.config = config;
                Task::none()
            }

            Message::SpawnPi { cwd } => {
                self.client_spawning = true;
                self.spawn_error = None;
                let model = self.config.default_model.clone();

                Task::perform(
                    async move {
                        let mut proc_cfg = PiProcessConfig::new().with_working_dir(cwd);
                        if let Some(m) = model {
                            proc_cfg = proc_cfg.with_model(m);
                        }
                        match PiClient::spawn(proc_cfg).await {
                            Ok(client) => Ok(Arc::new(client)),
                            Err(e) => Err(e.to_string()),
                        }
                    },
                    |res| cosmic::Action::from(Message::PiSpawned(res)),
                )
            }

            Message::PiSpawned(res) => {
                self.client_spawning = false;
                match res {
                    Ok(client) => {
                        self.spawn_error = None;
                        let controller = PiSessionController::new(Arc::clone(&client));
                        self.controller = Some(controller);
                        self.client = Some(Arc::clone(&client));

                        let client_clone = Arc::clone(&client);
                        Task::perform(
                            async move { client_clone.get_state().await },
                            |res| match res {
                                Ok(state) => {
                                    cosmic::Action::from(Message::SessionStateReceived(state))
                                }
                                Err(_) => cosmic::Action::from(Message::Noop),
                            },
                        )
                    }
                    Err(err) => {
                        self.spawn_error = Some(err);
                        self.client = None;
                        self.controller = None;
                        Task::none()
                    }
                }
            }

            Message::RestartPi => {
                self.show_settings = false;
                if let Some(mut old_client) = self.client.take() {
                    tokio::spawn(async move {
                        for _ in 0..10 {
                            match Arc::try_unwrap(old_client) {
                                Ok(client) => {
                                    let _ = client.shutdown().await;
                                    break;
                                }
                                Err(arc) => {
                                    old_client = arc;
                                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                                }
                            }
                        }
                    });
                }
                self.controller = None;
                let cwd = self.config.effective_working_dir();
                Task::perform(
                    async move { Message::SpawnPi { cwd } },
                    cosmic::Action::from,
                )
            }

            Message::ToggleSettings => {
                self.show_settings = !self.show_settings;
                Task::none()
            }

            Message::PickDefaultFolder => {
                let default_dir = self.config.effective_default_folder();
                Task::perform(
                    async move {
                        let handle = rfd::AsyncFileDialog::new()
                            .set_title("Select Default Folder")
                            .set_directory(&default_dir)
                            .pick_folder()
                            .await;
                        handle.map(|h| h.path().to_path_buf())
                    },
                    |path| cosmic::Action::from(Message::DefaultFolderSelected(path)),
                )
            }

            Message::DefaultFolderSelected(Some(path)) => {
                if !path.is_dir() {
                    return Task::none();
                }
                self.config.default_folder = Some(path.clone());
                if self
                    .config
                    .working_dir
                    .as_ref()
                    .is_some_and(|p| !p.is_dir())
                {
                    self.config.working_dir = None;
                }
                self.save_config();
                if self.spawn_error.is_some() || self.client.is_none() {
                    Task::perform(async { Message::RestartPi }, cosmic::Action::from)
                } else {
                    Task::none()
                }
            }

            Message::DefaultFolderSelected(None) => Task::none(),

            Message::ResetDefaultFolder => {
                let home = crate::config::home_dir();
                self.config.default_folder = Some(home);
                if self
                    .config
                    .working_dir
                    .as_ref()
                    .is_some_and(|p| !p.is_dir())
                {
                    self.config.working_dir = None;
                }
                self.save_config();
                if self.spawn_error.is_some() || self.client.is_none() {
                    Task::perform(async { Message::RestartPi }, cosmic::Action::from)
                } else {
                    Task::none()
                }
            }

            Message::ToggleAutostart(val) => {
                self.config.autostart_daemon = val;
                self.save_config();
                Task::none()
            }

            Message::ToggleNotifyComplete(val) => {
                self.config.notify_on_complete = val;
                self.save_config();
                Task::none()
            }

            Message::ToggleNotifyApproval(val) => {
                self.config.notify_on_approval = val;
                self.save_config();
                Task::none()
            }

            Message::ToggleNewestFirst(val) => {
                self.config.newest_first = val;
                self.save_config();
                Task::none()
            }

            Message::PickProjectDirectory => {
                let initial_dir = self.config.effective_working_dir();
                Task::perform(
                    async move {
                        let handle = rfd::AsyncFileDialog::new()
                            .set_title("Select Project Directory")
                            .set_directory(&initial_dir)
                            .pick_folder()
                            .await;
                        handle.map(|h| h.path().to_path_buf())
                    },
                    |path| cosmic::Action::from(Message::ProjectDirectorySelected(path)),
                )
            }

            Message::ProjectDirectorySelected(Some(path)) => {
                if !path.is_dir() {
                    return Task::none();
                }
                self.config.working_dir = Some(path.clone());
                self.config.add_recent_project(path);
                self.save_config();
                Task::perform(async { Message::RestartPi }, cosmic::Action::from)
            }

            Message::ProjectDirectorySelected(None) => Task::none(),

            Message::SelectRecentProject(path) => {
                if !path.is_dir() {
                    tracing::warn!("Selected project directory does not exist: {path:?}");
                    return Task::none();
                }
                self.config.working_dir = Some(path.clone());
                self.config.add_recent_project(path);
                self.save_config();
                Task::perform(async { Message::RestartPi }, cosmic::Action::from)
            }

            Message::NewSession => {
                if let Some(ref client) = self.client {
                    let client = Arc::clone(client);
                    let task = Task::perform(
                        async move {
                            let _ = client.new_session(None).await;
                        },
                        |_| cosmic::Action::from(Message::Noop),
                    );
                    if let Some(ref mut controller) = self.controller {
                        controller.history_mut().clear();
                    }
                    return task;
                }
                Task::none()
            }

            Message::CycleThinkingLevel => {
                if let Some(ref client) = self.client {
                    let client = Arc::clone(client);
                    return Task::perform(
                        async move { client.cycle_thinking_level().await },
                        |res| match res {
                            Ok(lvl) => cosmic::Action::from(Message::ThinkingLevelCycled(Ok(lvl))),
                            Err(e) => cosmic::Action::from(Message::ThinkingLevelCycled(Err(
                                e.to_string()
                            ))),
                        },
                    );
                }
                Task::none()
            }

            Message::ThinkingLevelCycled(Ok(Some(level))) => {
                let lvl_str = match level {
                    ThinkingLevel::Off => "off",
                    ThinkingLevel::Minimal => "minimal",
                    ThinkingLevel::Low => "low",
                    ThinkingLevel::Medium => "medium",
                    ThinkingLevel::High => "high",
                    ThinkingLevel::Xhigh => "xhigh",
                    ThinkingLevel::Max => "max",
                };
                self.config.thinking_level = Some(lvl_str.to_string());
                self.save_config();
                Task::none()
            }

            Message::ThinkingLevelCycled(_) => Task::none(),

            Message::CycleModel => {
                if let Some(ref client) = self.client {
                    let client = Arc::clone(client);
                    return Task::perform(
                        async move {
                            match client.cycle_model().await {
                                Ok(Some(data)) => Ok(Some(data.model.id)),
                                Ok(None) => Ok(None),
                                Err(e) => Err(e.to_string()),
                            }
                        },
                        |res| cosmic::Action::from(Message::ModelCycled(res)),
                    );
                }
                Task::none()
            }

            Message::ModelCycled(Ok(Some(model_id))) => {
                self.config.default_model = Some(model_id.clone());
                self.save_config();
                if let Some(ref mut controller) = self.controller {
                    controller.set_model(Some(model_id));
                    controller.set_model_context_window(None);
                }
                if let Some(ref client) = self.client {
                    let client = Arc::clone(client);
                    return Task::perform(
                        async move { client.get_state().await },
                        |res| match res {
                            Ok(state) => cosmic::Action::from(Message::SessionStateReceived(state)),
                            Err(_) => cosmic::Action::from(Message::Noop),
                        },
                    );
                }
                Task::none()
            }

            Message::ModelCycled(_) => Task::none(),

            Message::SessionStateReceived(state) => {
                if let Some(model) = state.model {
                    self.config.default_model = Some(model.id.clone());
                    if let Some(ref mut controller) = self.controller {
                        controller.set_model(Some(model.id));
                        controller.set_model_context_window(model.context_window);
                    }
                }
                Task::none()
            }

            Message::PromptInputChanged(val) => {
                self.prompt_input = val;
                Task::none()
            }

            Message::PickFileAttachment => {
                let initial_dir = self.config.effective_working_dir();
                Task::perform(
                    async move {
                        let handle = rfd::AsyncFileDialog::new()
                            .set_title("Select File to Reference")
                            .set_directory(&initial_dir)
                            .pick_file()
                            .await;
                        Message::FileAttachmentSelected(handle.map(|h| h.path().to_path_buf()))
                    },
                    cosmic::Action::from,
                )
            }

            Message::FileAttachmentSelected(Some(path)) => {
                let cwd = self.config.effective_working_dir();
                let ref_str = match path.strip_prefix(&cwd) {
                    Ok(rel) => rel.to_string_lossy().replace('\\', "/"),
                    Err(_) => path.to_string_lossy().to_string(),
                };
                let mut current = self.prompt_input.trim_end().to_string();
                if !current.is_empty() {
                    current.push(' ');
                }
                current.push_str(&format!("@{ref_str} "));
                self.prompt_input = current;
                Task::none()
            }

            Message::FileAttachmentSelected(None) => Task::none(),

            Message::SubmitPrompt => {
                let input = self.prompt_input.trim();
                if input.is_empty() {
                    return Task::none();
                }
                let raw_text = std::mem::take(&mut self.prompt_input);
                if let Some(ref mut controller) = self.controller {
                    let cwd = self.config.effective_working_dir();
                    let (processed_text, images) =
                        crate::file_reference::preprocess_prompt(&raw_text, &cwd);

                    let mut req = pi_rpc::protocol::PromptRequest::new(processed_text);
                    if !images.is_empty() {
                        req = req.with_images(images);
                    }

                    let task = controller.submit_prompt_request(req);
                    return task.map(Message::PiAction).map(cosmic::Action::from);
                }
                Task::none()
            }

            Message::SubmitSteer => {
                let input = self.prompt_input.trim();
                if input.is_empty() {
                    return Task::none();
                }
                let raw_text = std::mem::take(&mut self.prompt_input);
                if let Some(ref mut controller) = self.controller {
                    let cwd = self.config.effective_working_dir();
                    let (processed_text, images) =
                        crate::file_reference::preprocess_prompt(&raw_text, &cwd);
                    let task = if images.is_empty() {
                        controller.steer(processed_text)
                    } else {
                        controller.steer_with_images(processed_text, images)
                    };
                    return task.map(Message::PiAction).map(cosmic::Action::from);
                }
                Task::none()
            }

            Message::SubmitFollowUp => {
                let input = self.prompt_input.trim();
                if input.is_empty() {
                    return Task::none();
                }
                let raw_text = std::mem::take(&mut self.prompt_input);
                if let Some(ref mut controller) = self.controller {
                    let cwd = self.config.effective_working_dir();
                    let (processed_text, images) =
                        crate::file_reference::preprocess_prompt(&raw_text, &cwd);
                    let task = if images.is_empty() {
                        controller.follow_up(processed_text)
                    } else {
                        controller.follow_up_with_images(processed_text, images)
                    };
                    return task.map(Message::PiAction).map(cosmic::Action::from);
                }
                Task::none()
            }

            Message::Abort => {
                if let Some(ref mut controller) = self.controller {
                    let task = controller.abort();
                    return task.map(Message::PiAction).map(cosmic::Action::from);
                }
                Task::none()
            }

            Message::PiAction(action) => {
                match &action {
                    PiAction::Event(PiEvent::AgentSettled { aborted: false }) => {
                        if self.config.notify_on_complete && self.popup.is_none() {
                            notifications::notify_task_completed(
                                "Task Completed",
                                "Agent has finished running instructions.",
                            );
                        }
                    }
                    PiAction::UiRequest(req)
                        if self.config.notify_on_approval && req.is_dialog() =>
                    {
                        let (title, message) = match req {
                            ExtensionUiRequest::Confirm { title, message, .. } => {
                                (title.as_str(), message.as_str())
                            }
                            ExtensionUiRequest::Select { title, .. } => {
                                (title.as_str(), "Please select an option to proceed.")
                            }
                            ExtensionUiRequest::Input { title, .. } => {
                                (title.as_str(), "Input is required to proceed.")
                            }
                            ExtensionUiRequest::Editor { title, .. } => {
                                (title.as_str(), "Please review and edit content.")
                            }
                            _ => ("Action Required", "Pi extension requires user interaction."),
                        };
                        notifications::notify_approval_needed(title, message);
                    }
                    PiAction::UiRequest(ExtensionUiRequest::Notify {
                        message,
                        notify_type,
                        ..
                    }) if self.popup.is_none()
                        && (self.config.notify_on_complete
                            || matches!(
                                notify_type,
                                Some(pi_rpc::protocol::NotifyType::Error)
                            )) =>
                    {
                        notifications::notify_task_completed("Message from Pi", message);
                    }
                    _ => {}
                }

                if let Some(ref mut controller) = self.controller {
                    let task = controller.update(action);
                    return task.map(Message::PiAction).map(cosmic::Action::from);
                }
                Task::none()
            }

            Message::Noop => Task::none(),

            Message::Surface(a) => cosmic::task::message(cosmic::Action::Surface(a)),
        }
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        Some(cosmic::applet::style())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmic::Application;

    #[test]
    fn test_app_model_default_status() {
        let app = AppModel::default();
        assert_eq!(app.status(), AgentStatus::Idle);
    }

    #[test]
    fn test_app_model_spawning_and_error_status() {
        let mut app = AppModel {
            client_spawning: true,
            ..Default::default()
        };
        assert_eq!(app.status(), AgentStatus::Thinking);

        app.client_spawning = false;
        app.spawn_error = Some("No such binary: pi".into());
        assert_eq!(
            app.status(),
            AgentStatus::Error("No such binary: pi".into())
        );
    }

    #[test]
    fn test_app_model_prompt_input_update() {
        let mut app = AppModel::default();
        let _ = app.update(Message::PromptInputChanged("Implement feature X".into()));
        assert_eq!(app.prompt_input, "Implement feature X");

        // SubmitPrompt clears the prompt input field
        let _ = app.update(Message::SubmitPrompt);
        assert_eq!(app.prompt_input, "");
    }

    #[test]
    fn test_app_model_file_attachment_selected() {
        let mut app = AppModel::default();
        let cwd = app.config.effective_working_dir();
        let file = cwd.join("src/main.rs");

        let _ = app.update(Message::PromptInputChanged("Please check".into()));
        let _ = app.update(Message::FileAttachmentSelected(Some(file)));
        assert_eq!(app.prompt_input, "Please check @src/main.rs ");
    }

    #[test]
    fn test_app_model_select_recent_project() {
        let mut app = AppModel::default();
        let proj = std::env::temp_dir();
        let _ = app.update(Message::SelectRecentProject(proj.clone()));
        assert_eq!(app.config.working_dir, Some(proj.clone()));
        assert_eq!(app.config.recent_projects, vec![proj]);
    }

    #[test]
    fn test_message_debug_formatting() {
        let msg = Message::TogglePopup;
        assert_eq!(format!("{msg:?}"), "TogglePopup");

        let msg_steer = Message::SubmitSteer;
        assert_eq!(format!("{msg_steer:?}"), "SubmitSteer");

        let msg_follow_up = Message::SubmitFollowUp;
        assert_eq!(format!("{msg_follow_up:?}"), "SubmitFollowUp");

        let msg_abort = Message::Abort;
        assert_eq!(format!("{msg_abort:?}"), "Abort");

        let msg_toggle_newest = Message::ToggleNewestFirst(true);
        assert_eq!(format!("{msg_toggle_newest:?}"), "ToggleNewestFirst(true)");

        let msg_cycle_model = Message::CycleModel;
        assert_eq!(format!("{msg_cycle_model:?}"), "CycleModel");

        let msg_model_cycled = Message::ModelCycled(Ok(Some("anthropic/claude-3-7-sonnet".into())));
        assert_eq!(
            format!("{msg_model_cycled:?}"),
            "ModelCycled(Ok(Some(\"anthropic/claude-3-7-sonnet\")))"
        );
    }

    #[test]
    fn test_app_model_cycle_model_update() {
        let mut app = AppModel::default();
        let _ = app.update(Message::ModelCycled(Ok(Some(
            "anthropic/claude-3-7-sonnet".into(),
        ))));
        assert_eq!(
            app.config.default_model,
            Some("anthropic/claude-3-7-sonnet".into())
        );
    }

    #[test]
    fn test_app_model_toggle_newest_first() {
        let mut app = AppModel::default();
        assert!(app.config.newest_first);

        let _ = app.update(Message::ToggleNewestFirst(false));
        assert!(!app.config.newest_first);

        let _ = app.update(Message::ToggleNewestFirst(true));
        assert!(app.config.newest_first);
    }

    #[test]
    fn test_popover_view_generation_does_not_panic() {
        let config = Config {
            default_model: Some("openai/gpt-4o".into()),
            ..Default::default()
        };
        let element = popover::view_popover(
            &config,
            None,
            false,
            None,
            "test prompt",
            Some("anthropic/claude-3-7-sonnet"),
            false,
        );
        // Ensure element tree evaluates cleanly without panic
        let _ = element;

        // Test with spawning
        let element_spawning = popover::view_popover(&config, None, true, None, "", None, false);
        let _ = element_spawning;

        // Test with error
        let element_error = popover::view_popover(
            &config,
            None,
            false,
            Some("Failed to launch pi binary"),
            "",
            None,
            false,
        );
        let _ = element_error;

        // Test settings view
        let element_settings = popover::view_popover(&config, None, false, None, "", None, true);
        let _ = element_settings;
    }

    #[test]
    fn test_app_model_pi_action_events() {
        let mut app = AppModel::default();
        let turn_end_event = PiEvent::TurnEnd {
            message: pi_rpc::protocol::AgentMessage {
                role: "assistant".into(),
                content: serde_json::json!("Tool executed"),
                timestamp: None,
                extra: Default::default(),
            },
            tool_results: vec![],
        };
        let _ = app.update(Message::PiAction(cosmic_ext_pi::PiAction::Event(
            turn_end_event,
        )));

        let settled_event = PiEvent::AgentSettled { aborted: false };
        let _ = app.update(Message::PiAction(cosmic_ext_pi::PiAction::Event(
            settled_event,
        )));
    }

    #[test]
    fn test_app_model_ui_request_notify_does_not_trigger_approval() {
        let mut app = AppModel::default();
        app.config.notify_on_approval = true;

        // An informational notification from an extension must not trigger approval needed
        let notify_req = ExtensionUiRequest::Notify {
            id: Some("id1".into()),
            message: "radius: connected, 123 tools (codemode)".into(),
            notify_type: Some(pi_rpc::protocol::NotifyType::Info),
        };
        let _ = app.update(Message::PiAction(cosmic_ext_pi::PiAction::UiRequest(
            notify_req,
        )));
    }
}
