---
id: cosmic-ext-applet-pi
type: module-design
status: active
title: cosmic-ext-applet-pi — COSMIC Panel Applet & Popover Shell
parent: architecture
depends-on:
  - cosmic-ext-pi
  - pi-rpc
---

## Responsibility

The top-level COSMIC Desktop panel/dock applet executable (`io.github.jotuel.cosmic-ext-applet-pi`) that hosts a `PiSessionController` inside a Wayland flyout popover and integrates with desktop services.

- **Panel Lifecycle & Durable Subprocess Management (`app`)**:
  - Implements `cosmic::Application` (`AppModel`), rendering the panel icon button with a live status tooltip (`⌨` Idle, `💡` Thinking, `🛠` ExecutingTool, `⚠` AwaitingUser, `⛔` Error) and managing the Wayland popup surface (`get_popup` / `destroy_popup`).
  - Spawns `PiClient` for `effective_working_dir()` on applet initialization when `autostart_daemon` is enabled, or lazily when the user first opens the popover. When `cosmic-panel` reloads the applet or the user switches projects, the newly spawned `PiClient` opens that project's SQLite session, hydrates the UI from `SnapshotEvent`, and automatically resumes any unfinished run.
- **Popover Layout & Controls (`popover`)**:
  - Enforces fixed popover dimensions (`POPOVER_WIDTH = 420.0`, `POPOVER_HEIGHT = 580.0`, `<= 600.0` height) with an anchored project/settings header and an anchored bottom input/status/footer section.
  - Provides interactive slash-command (`/`) and workspace file (`@`) suggestion chips, context-aware Send vs. Steer/Follow-up/Abort buttons (pressing Enter while busy submits a `Steer` message rather than a new prompt), and a footer displaying model cycling, circular context-window usage (`tokens_in / context_window`), and thinking-level cycling.
- **Client-Side `@file` Preprocessing & Autocomplete (`file_reference`)**:
  - Intercepts `@path` tokens client-side before sending `Submit`, `Steer`, or `FollowUp` commands, deduplicating referenced paths, inlining non-empty UTF-8 text files up to 128 KB as `<file name="...">...</file>` blocks, and base64-encoding supported image files into `ImageContent` attachments.
- **Configuration Persistence (`config`)**:
  - Persists `Config` (version 1) via `cosmic-config`, resolving the effective working directory in priority order: existing `working_dir` → first existing entry in `recent_projects` (deduplicated, capped at 10) → existing `default_folder` → `$HOME`.
- **Desktop Notifications & Localization (`notifications`, `i18n`)**:
  - Dispatches normal-urgency notifications on `PiEvent::RunEnd` (or extension `Notify`) only when the popover is closed (`popup.is_none()`), and critical-urgency notifications with sound (`message-new-instant`) whenever an interactive Extension UI dialog (`req.is_dialog()`) requires approval.
  - Loads embedded Fluent (`i18n/en/cosmic_ext_applet_pi.ftl`) translations via `rust-embed` and `i18n-embed`.

## Boundary

- **Allowed dependencies**: `cosmic-ext-pi`, `pi-rpc`, `libcosmic` (`applet`, `applet-token`, `dbus-config`, `multi-window`, `tokio`, `wayland`, `winit`), `tokio`, `notify-rust`, `rfd`, `walkdir`, `base64`, `i18n-embed`, `i18n-embed-fl`, `rust-embed`, `serde`, `serde_json`, `tracing`.
- **Forbidden reaches**: Must not bypass `PiClient` or `PiSessionController` to parse raw JSONL protocol frames directly; reusable widget or controller state transitions that are independent of the applet shell belong in `cosmic-ext-pi`.
