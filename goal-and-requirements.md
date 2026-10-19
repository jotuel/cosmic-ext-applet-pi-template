---
id: goal-and-requirements
type: goal-and-requirements
status: active
title: COSMIC Pi Durable Applet & RPC Stack — Goal and Requirements
---

## Goal

Provide a functional, self-evolving COSMIC Desktop panel/dock applet (`cosmic-ext-applet-pi`) and reusable Rust/libcosmic libraries (`pi-rpc`, `cosmic-ext-pi`) that supervise and interact with a durable `@earendil-works/pi-durable` coding agent runtime so sessions and in-flight turns survive panel applet reloads seamlessly.

## Problem & Audience

Developers using the System76 COSMIC desktop environment who want persistent, panel-accessible AI coding agent access without keeping a dedicated terminal window open. Because moving applets on `cosmic-panel` reloads every applet instance, sessions must persist their state and automatically resume unfinished turns across applet restarts without requiring the user to manually resume and prompt `"continue"`.

## Capabilities

- **Durable Per-Project Sessions & Auto-Resume (`pi-rpc`)**: Launches a bundled `@earendil-works/pi-durable` Node host backed by one WAL SQLite database per project directory under `$XDG_DATA_HOME/cosmic-ext-applet-pi/sessions/`, reuses `~/.pi/agent` model/auth configuration, emits an initial `SnapshotEvent` on attachment, and calls `harness.resume()` so interrupted runs continue automatically.
- **COSMIC / Iced Session Controller & Widgets (`cosmic-ext-pi`)**: Bridges `PiClient` into `libcosmic` via an Iced `Subscription`, hydrates active chat/tool transcripts and in-flight runs from `SnapshotEvent`, applies streaming `MessageChange` and tool output deltas, handles built-in and discovered slash commands, and renders native modal dialogs (`confirm`, `select`, `input`, `editor`) for Extension UI requests.
- **COSMIC Panel Applet (`cosmic-ext-applet-pi`)**: Runs as a Wayland panel/dock applet with live status tooltip, a fixed-bounds flyout popover (`420x580`), project working directory selection and recent-projects history (`cosmic-config`), client-side `@path` file/image attachment preprocessing and autocomplete chips, model and thinking-level cycling with context-window progress, desktop notifications (`notify-rust`), and Fluent (`i18n-embed`) localization.

## Non-Goals

- **Turnkey feature completeness out of the box**: The repository intentionally omits opinionated workflow features so each user (together with the applet) can evolve their own fork from a clean baseline.
- **Embedding a model runtime in Rust**: All model inference, durable task scheduling, tool execution (`CodingTools`), and context compaction are delegated to the `@earendil-works/pi-durable` Node host supervised over JSONL stdio.
