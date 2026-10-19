// SPDX-License-Identifier: EUPL-1.2

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::fl;

pub const DEFAULT_SESSION_ID: &str = "default";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionRecord {
    pub id: String,
    pub name: String,
    pub created_at_secs: u64,
    pub updated_at_secs: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct SessionIndex {
    #[serde(default)]
    sessions: Vec<SessionRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionInfo {
    pub id: String,
    pub name: String,
    pub is_active: bool,
    pub updated_at_secs: u64,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn fnv1a_64_hex(input: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in input.as_bytes() {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn sanitize_folder_name(path: &Path) -> String {
    let raw = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("root");
    let sanitized: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.is_empty() {
        "root".to_string()
    } else {
        sanitized
    }
}

/// Returns the root directory where Pi Durable stores session data.
pub fn base_sessions_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("share")))
        .unwrap_or_else(std::env::temp_dir)
        .join("cosmic-ext-applet-pi")
        .join("sessions")
}

/// Returns the per-project directory used for additional durable sessions and metadata.
pub fn project_sessions_dir(cwd: &Path) -> PathBuf {
    let canonical = fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    let folder = sanitize_folder_name(&canonical);
    let hash = fnv1a_64_hex(&canonical.to_string_lossy());
    base_sessions_dir().join(format!("{folder}-{hash}"))
}

/// Returns the `--session-dir` override for `session_id`, or `None` for the default session.
pub fn session_dir_for_id(cwd: &Path, session_id: &str) -> Option<PathBuf> {
    if session_id.is_empty() || session_id == DEFAULT_SESSION_ID {
        None
    } else {
        Some(project_sessions_dir(cwd).join(session_id))
    }
}

/// Returns the `--session-dir` override for the currently active session of `cwd`.
pub fn active_session_dir(config: &Config, cwd: &Path) -> Option<PathBuf> {
    session_dir_for_id(cwd, config.active_session_id(cwd))
}

fn index_path(cwd: &Path) -> PathBuf {
    project_sessions_dir(cwd).join("sessions.json")
}

fn load_index(cwd: &Path) -> SessionIndex {
    let path = index_path(cwd);
    match fs::read_to_string(&path) {
        Ok(raw) => serde_json::from_str(&raw).unwrap_or_default(),
        Err(_) => SessionIndex::default(),
    }
}

fn save_index(cwd: &Path, index: &SessionIndex) -> std::io::Result<()> {
    let dir = project_sessions_dir(cwd);
    fs::create_dir_all(&dir)?;
    let path = dir.join("sessions.json");
    let json = serde_json::to_string_pretty(index).map_err(std::io::Error::other)?;
    fs::write(path, json)
}

fn dir_mtime_secs(dir: &Path) -> u64 {
    fs::metadata(dir)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Lists all enduring sessions for `cwd`, sorted with most recently updated first.
pub fn list_sessions(config: &Config, cwd: &Path) -> Vec<SessionInfo> {
    let active_id = config.active_session_id(cwd);
    let mut index = load_index(cwd);
    let proj_dir = project_sessions_dir(cwd);

    // Discover any on-disk session subdirectories missing from sessions.json.
    if let Ok(entries) = fs::read_dir(&proj_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(id) = path.file_name().and_then(|n| n.to_str()).map(str::to_string) else {
                continue;
            };
            if !index.sessions.iter().any(|s| s.id == id) {
                let mtime = dir_mtime_secs(&path);
                index.sessions.push(SessionRecord {
                    id,
                    name: fl!("new-session"),
                    created_at_secs: mtime,
                    updated_at_secs: mtime,
                });
            }
        }
    }

    // Ensure the default session is always represented.
    if !index.sessions.iter().any(|s| s.id == DEFAULT_SESSION_ID) {
        index.sessions.push(SessionRecord {
            id: DEFAULT_SESSION_ID.to_string(),
            name: fl!("default-session"),
            created_at_secs: 0,
            updated_at_secs: 0,
        });
    }

    let mut result: Vec<SessionInfo> = index
        .sessions
        .into_iter()
        .map(|rec| {
            let is_active = rec.id == active_id;
            let name = if rec.name.trim().is_empty() {
                if rec.id == DEFAULT_SESSION_ID {
                    fl!("default-session")
                } else {
                    fl!("new-session")
                }
            } else {
                rec.name
            };
            SessionInfo {
                id: rec.id,
                name,
                is_active,
                updated_at_secs: rec.updated_at_secs,
            }
        })
        .collect();

    result.sort_by(|a, b| {
        b.updated_at_secs
            .cmp(&a.updated_at_secs)
            .then_with(|| b.is_active.cmp(&a.is_active))
            .then_with(|| a.id.cmp(&b.id))
    });

    result
}

/// Creates a new durable session for `cwd`, sets it active in `config`, and returns its ID.
pub fn create_session(config: &mut Config, cwd: &Path) -> std::io::Result<String> {
    let proj_dir = project_sessions_dir(cwd);
    let mut id = format!("s-{}", now_millis());
    let mut suffix = 1u32;
    while proj_dir.join(&id).exists() {
        id = format!("s-{}-{suffix}", now_millis());
        suffix += 1;
    }

    fs::create_dir_all(proj_dir.join(&id))?;

    let mut index = load_index(cwd);
    let ts = now_secs();
    index.sessions.push(SessionRecord {
        id: id.clone(),
        name: fl!("new-session"),
        created_at_secs: ts,
        updated_at_secs: ts,
    });
    save_index(cwd, &index)?;

    config.set_active_session(cwd, &id);
    Ok(id)
}

/// Updates the session's auto-generated title if it still has a placeholder name,
/// and bumps `updated_at_secs`.
pub fn note_prompt_activity(cwd: &Path, session_id: &str, prompt_text: &str) {
    let first_line = prompt_text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    let snippet = if first_line.chars().count() > 40 {
        let truncated: String = first_line.chars().take(38).collect();
        format!("{truncated}…")
    } else {
        first_line.to_string()
    };

    let mut index = load_index(cwd);
    let ts = now_secs();
    let default_name = fl!("default-session");
    let new_name = fl!("new-session");

    if let Some(rec) = index.sessions.iter_mut().find(|s| s.id == session_id) {
        rec.updated_at_secs = ts;
        let is_placeholder = rec.name.trim().is_empty()
            || rec.name == default_name
            || rec.name == new_name
            || rec.name == "Default Session"
            || rec.name == "New Session";
        if is_placeholder && !snippet.is_empty() {
            rec.name = snippet;
        }
    } else {
        let name = if !snippet.is_empty() {
            snippet
        } else if session_id == DEFAULT_SESSION_ID {
            default_name
        } else {
            new_name
        };
        index.sessions.push(SessionRecord {
            id: session_id.to_string(),
            name,
            created_at_secs: ts,
            updated_at_secs: ts,
        });
    }

    let _ = save_index(cwd, &index);
}

/// Deletes a non-default session for `cwd`. Returns `Ok(true)` if the deleted
/// session was currently active (and switches `config` back to another session).
pub fn delete_session(config: &mut Config, cwd: &Path, session_id: &str) -> std::io::Result<bool> {
    if session_id == DEFAULT_SESSION_ID {
        return Ok(false);
    }

    let dir = project_sessions_dir(cwd).join(session_id);
    if dir.exists() {
        fs::remove_dir_all(&dir)?;
    }

    let mut index = load_index(cwd);
    index.sessions.retain(|s| s.id != session_id);
    save_index(cwd, &index)?;

    let was_active = config.active_session_id(cwd) == session_id;
    if was_active {
        let next_id = index
            .sessions
            .first()
            .map(|s| s.id.clone())
            .unwrap_or_else(|| DEFAULT_SESSION_ID.to_string());
        config.set_active_session(cwd, next_id);
    }
    Ok(was_active)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_lifecycle_create_list_title_delete() {
        let temp = tempfile_dir("lifecycle");
        let mut config = Config::default();

        // Initially only default session exists
        let initial = list_sessions(&config, &temp);
        assert_eq!(initial.len(), 1);
        assert_eq!(initial[0].id, DEFAULT_SESSION_ID);
        assert!(initial[0].is_active);
        assert_eq!(active_session_dir(&config, &temp), None);

        // Create a new session
        let s1 = create_session(&mut config, &temp).expect("create session");
        assert!(s1.starts_with("s-"));
        assert_eq!(config.active_session_id(&temp), s1);
        assert!(active_session_dir(&config, &temp).is_some());

        // Auto-name on first prompt
        note_prompt_activity(&temp, &s1, "Implement session switcher\nwith extra details");
        let listed = list_sessions(&config, &temp);
        assert_eq!(listed.len(), 2);
        let active_entry = listed.iter().find(|s| s.id == s1).expect("s1 listed");
        assert!(active_entry.is_active);
        assert_eq!(active_entry.name, "Implement session switcher");

        // Delete active session falls back to default
        let was_active = delete_session(&mut config, &temp, &s1).expect("delete session");
        assert!(was_active);
        assert_eq!(config.active_session_id(&temp), DEFAULT_SESSION_ID);

        let _ = fs::remove_dir_all(&temp);
    }

    fn tempfile_dir(suffix: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "cosmic-ext-applet-pi-session-test-{suffix}-{}",
            now_millis()
        ));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }
}
