// SPDX-License-Identifier: EUPL-1.2

use notify_rust::{Notification, Urgency};

const APP_NAME: &str = "Assistant";
const ICON_NAME: &str = "io.github.jotuel.cosmic-ext-applet-pi-symbolic";

/// Dispatches a desktop notification when a background agent task finishes.
pub fn notify_task_completed(summary: &str, body: &str) {
    let summary = summary.to_string();
    let body = body.to_string();
    let run = move || {
        let _ = Notification::new()
            .appname(APP_NAME)
            .summary(&summary)
            .body(&body)
            .icon(ICON_NAME)
            .urgency(Urgency::Normal)
            .show();
    };
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        handle.spawn_blocking(run);
    } else {
        std::thread::spawn(run);
    }
}

/// Dispatches an urgent desktop notification and audio alert cue when user confirmation is requested.
pub fn notify_approval_needed(title: &str, message: &str) {
    let title = format!("Action Required: {title}");
    let message = message.to_string();
    let run = move || {
        let _ = Notification::new()
            .appname(APP_NAME)
            .summary(&title)
            .body(&message)
            .icon(ICON_NAME)
            .urgency(Urgency::Critical)
            .sound_name("message-new-instant")
            .show();
    };
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        handle.spawn_blocking(run);
    } else {
        std::thread::spawn(run);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_notification_formatting() {
        let title = "Bash Execution";
        let formatted = format!("Action Required: {title}");
        assert_eq!(formatted, "Action Required: Bash Execution");
    }
}
