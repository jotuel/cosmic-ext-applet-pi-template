---
id: cosmic-ext-pi
type: module-design
status: active
title: cosmic-ext-pi — libcosmic Session Controller, Widgets & Dialogs
parent: architecture
depends-on:
  - pi-rpc
---

## Responsibility

Bridges `pi-rpc` into `libcosmic` / Iced applications through a reactive session controller (`PiSessionController`), a multiplexed event subscription (`pi_subscription`), and reusable conversational and modal UI components.

- **Reactive Stream Adapter (`subscription`)**: Multiplexes `PiClient::subscribe_events()` and `PiClient::subscribe_ui_requests()` inside a single Iced `Subscription<PiAction>`, forwarding `PiClient::latest_snapshot()` immediately upon connection so initial `SnapshotEvent` state is never missed.
- **Snapshot Hydration & Native Event Reducer (`controller`)**:
  - Hydrates full controller state from `PiEvent::Snapshot(SnapshotEvent)`: reconstructs active conversation history after the latest `head`/`pi.reset` marker (`pi.user`, `pi.assistant`, `pi.tool-result`, `pi.compaction`), attaches any in-flight `generation.message` partial and running `tools` slots, restores `agent.model` and `thinkingLevel`, aggregates `usage` across model and tool buckets, populates `steering_queue` and `follow_up_queue` from `inbox`, and sets `AgentStatus` (`ExecutingTool`, `Thinking`, or `Idle`).
  - Applies streaming `MessageUpdate { usage, changes: Vec<MessageChange> }` deltas (`ThinkingStart`, `ThinkingDelta`, `TextStart`, `TextDelta`, `ToolcallStart`, `ToolcallDelta`, `Block`, `Message`) and `ToolExecutionUpdate { output: Option<ToolOutputUpdate>, .. }` updates (`set` or `trim_start` + `append`), tracking per-turn block offsets (`current_turn_block_offset`) so consecutive assistant turns within a run merge into a cohesive assistant message without overwriting earlier turns' tool/thinking blocks.
  - Surfaces model errors from failed `pi.assistant` messages (`errorMessage`) and `unanswered` `Submission` events as visible `⚠️ **Error:**` blocks in chat history while preserving `AgentStatus::Error` across `RunEnd`.
  - Intercepts client-side slash commands (`/help`, `/compact`, `/new`, `/clear`, `/abort`, `/model`, `/thinking`, `/session`, `/stats`, `/name`, `/steer`, `/followup`) while forwarding discovered `skill:`, `prompt`, and extension commands to `PiClient::prompt`.
  - Queues incoming `ExtensionUiRequest` dialogs (`Confirm`, `Select`, `Input`, `Editor`) by ID, transitions status to `AgentStatus::AwaitingUser`, and dispatches `ExtensionUiResponse` back to `PiClient::respond_ui`.
- **Widgets & Dialogs (`widget`, `dialogs`)**: Stateless `libcosmic` view builders (`ChatView`, `ThinkingBlock`, `ToolCard`, `StatusBadge`, `ConfirmDialog`, `SelectDialog`, `InputDialog`, `EditorDialog`) that emit `PiAction` messages.

## Boundary

- **Allowed dependencies**: `pi-rpc`, `libcosmic` (with `markdown` feature), `tokio`, `tokio-stream`, `futures`, `serde`, `serde_json`, `uuid`, `tracing`.
- **Forbidden reaches**: Must not depend on `cosmic-ext-applet-pi` or panel-applet shell concerns (`cosmic-config` persistence, Wayland popup management, `notify-rust` desktop notifications, `rfd` native file pickers, or local filesystem `@file` expansion).
