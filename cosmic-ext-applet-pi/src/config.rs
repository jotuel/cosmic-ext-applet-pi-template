// SPDX-License-Identifier: EUPL-1.2

use cosmic::cosmic_config::{self, CosmicConfigEntry, cosmic_config_derive::CosmicConfigEntry};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const MAX_RECENT_PROJECTS: usize = 10;

fn default_newest_first() -> bool {
    true
}

#[derive(Debug, Clone, CosmicConfigEntry, Serialize, Deserialize, PartialEq, Eq)]
#[version = 1]
pub struct Config {
    /// List of recently used project workspace directory paths.
    pub recent_projects: Vec<PathBuf>,
    /// Default workspace directory path (defaults to $HOME).
    pub default_folder: Option<PathBuf>,
    /// Default model identifier (e.g. "anthropic/claude-3-7-sonnet").
    pub default_model: Option<String>,
    /// Default thinking / reasoning effort level ("off", "low", "medium", "high").
    pub thinking_level: Option<String>,
    /// Whether to autostart the Pi process on login / applet init.
    pub autostart_daemon: bool,
    /// Whether to trigger a desktop notification when an agent task completes.
    pub notify_on_complete: bool,
    /// Whether to trigger a desktop notification when user confirmation is needed.
    pub notify_on_approval: bool,
    /// Active working directory override.
    pub working_dir: Option<PathBuf>,
    /// Whether to display newest messages at the top of the chat view.
    #[serde(default = "default_newest_first")]
    pub newest_first: bool,
}

/// Returns the user's home directory path or current directory as fallback.
pub fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

impl Default for Config {
    fn default() -> Self {
        Self {
            recent_projects: Vec::new(),
            default_folder: Some(home_dir()),
            default_model: None,
            thinking_level: Some("medium".to_string()),
            autostart_daemon: true,
            notify_on_complete: true,
            notify_on_approval: true,
            working_dir: None,
            newest_first: true,
        }
    }
}

impl Config {
    /// Adds a project directory to the front of `recent_projects`, deduplicating
    /// and capping at `MAX_RECENT_PROJECTS`.
    pub fn add_recent_project(&mut self, path: PathBuf) {
        self.recent_projects.retain(|p| p != &path);
        self.recent_projects.insert(0, path);
        if self.recent_projects.len() > MAX_RECENT_PROJECTS {
            self.recent_projects.truncate(MAX_RECENT_PROJECTS);
        }
    }

    /// Returns the effective default folder: configured `default_folder` (if it exists)
    /// or fallback to `$HOME`.
    pub fn effective_default_folder(&self) -> PathBuf {
        self.default_folder
            .clone()
            .filter(|p| p.is_dir())
            .unwrap_or_else(home_dir)
    }

    /// Resolves the effective working directory:
    /// `working_dir` (if existing dir) -> first existing `recent_projects` -> `effective_default_folder()`.
    pub fn effective_working_dir(&self) -> PathBuf {
        if let Some(ref dir) = self.working_dir
            && dir.is_dir()
        {
            return dir.clone();
        }
        if let Some(first) = self.recent_projects.iter().find(|p| p.is_dir()) {
            return first.clone();
        }
        self.effective_default_folder()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_defaults() {
        let config = Config::default();
        assert!(config.recent_projects.is_empty());
        assert_eq!(config.default_folder, Some(home_dir()));
        assert_eq!(config.effective_default_folder(), home_dir());
        assert_eq!(config.thinking_level.as_deref(), Some("medium"));
        assert!(config.autostart_daemon);
        assert!(config.notify_on_complete);
        assert!(config.notify_on_approval);
        assert!(config.working_dir.is_none());
        assert!(config.newest_first);
    }

    #[test]
    fn test_recent_projects_dedup_and_cap() {
        let mut config = Config::default();
        let p1 = PathBuf::from("/tmp/proj1");
        let p2 = PathBuf::from("/tmp/proj2");

        config.add_recent_project(p1.clone());
        config.add_recent_project(p2.clone());
        assert_eq!(config.recent_projects, vec![p2.clone(), p1.clone()]);

        // Re-adding p1 moves it to the front
        config.add_recent_project(p1.clone());
        assert_eq!(config.recent_projects, vec![p1.clone(), p2]);

        // Capping at MAX_RECENT_PROJECTS
        for i in 0..15 {
            config.add_recent_project(PathBuf::from(format!("/tmp/proj_{i}")));
        }
        assert_eq!(config.recent_projects.len(), MAX_RECENT_PROJECTS);
        assert_eq!(config.recent_projects[0], PathBuf::from("/tmp/proj_14"));
    }

    #[test]
    fn test_effective_working_dir() {
        let mut config = Config::default();
        assert_eq!(config.effective_working_dir(), home_dir());

        let temp_dir = std::env::temp_dir();
        config.add_recent_project(temp_dir.clone());
        assert_eq!(config.effective_working_dir(), temp_dir);

        let explicit_dir = home_dir();
        config.working_dir = Some(explicit_dir.clone());
        assert_eq!(config.effective_working_dir(), explicit_dir);
    }

    #[test]
    fn test_effective_working_dir_ignores_nonexistent() {
        let mut config = Config::default();
        config.default_folder = Some(home_dir());
        config.working_dir = Some(PathBuf::from("/tmp/integration-test-dir-nonexistent-12345"));
        assert_eq!(config.effective_working_dir(), home_dir());

        config.recent_projects = vec![PathBuf::from("/tmp/nonexistent-recent-12345")];
        assert_eq!(config.effective_working_dir(), home_dir());
    }

    #[test]
    fn test_config_serialization() {
        let mut config = Config::default();
        config.add_recent_project(PathBuf::from("/tmp/foo"));
        config.default_model = Some("openai/gpt-4o".into());

        let json = serde_json::to_string(&config).expect("serialize");
        let deserialized: Config = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(config, deserialized);

        // Verify backward compatibility when newest_first is absent in JSON
        let legacy_json = r#"{
            "recent_projects": [],
            "default_folder": null,
            "default_model": null,
            "thinking_level": null,
            "autostart_daemon": false,
            "notify_on_complete": false,
            "notify_on_approval": false,
            "working_dir": null
        }"#;
        let legacy_config: Config = serde_json::from_str(legacy_json).expect("deserialize legacy");
        assert!(legacy_config.newest_first);
    }
}
