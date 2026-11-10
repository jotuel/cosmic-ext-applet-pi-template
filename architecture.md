---
id: architecture
type: architecture-design
status: active
title: Workspace Architecture & Crate Boundaries
parent: goal-and-requirements
---

## Drivers

- Seamless session and turn continuity when `cosmic-panel` reloads applets or when switching between project directories.
- Clean separation between async stdio process supervision (`tokio`), Iced/COSMIC reactive state management, and Wayland panel applet lifecycle (`cosmic-config`, notifications, native file dialogs).
- Licensing flexibility: permissive library crates paired with a copyleft desktop applet binary.

## Decisions

### Three-Crate Workspace Split & Licensing

The Cargo workspace is divided into three crates with strict, unidirectional dependencies:

```
cosmic-ext-applet-pi (EUPL-1.2 binary/applet shell)
        │
        ▼
  cosmic-ext-pi      (MIT OR Apache-2.0 libcosmic controller & widgets)
        │
        ▼
     pi-rpc          (MIT OR Apache-2.0 headless Pi Durable JSONL client & supervisor)
```

- **`pi-rpc`** owns the bundled `@earendil-works/pi-durable` Node host (`durable_host.cjs`), child process lifecycle, JSONL transport framing, request-ID correlation, and native `pi-durable` wire protocol types (`SnapshotEvent`, `PiEvent`, `EntryRecord`, `MessageChange`, `UsageState`, `AgentState`).
- **`cosmic-ext-pi`** owns the `libcosmic`/Iced state machine (`PiSessionController`), `Subscription` stream adapter, reusable chat/tool/thinking widgets, and Extension UI dialogs.
- **`cosmic-ext-applet-pi`** owns the COSMIC panel button, Wayland popup window, `cosmic-config` persistence, `@file` workspace autocomplete/preprocessing, desktop notifications, and Fluent localization under `EUPL-1.2`.

### Native Pi Durable Supervision over Stdio JSONL

Rather than running a stateless CLI child process that loses in-flight work on exit, `pi-rpc` supervises a bundled `@earendil-works/pi-durable` Node host over piped `stdin`/`stdout`/`stderr`. Each canonical project working directory maps to an isolated WAL SQLite database under `$XDG_DATA_HOME/cosmic-ext-applet-pi/sessions/<slug>-<sha256>.sqlite` (keeping project git repositories clean of `.sqlite` files). On startup, the host attaches `watchEvents(harness, root.id, context)` to emit a full `SnapshotEvent` and calls `harness.resume()` so any unfinished turn or tool execution continues automatically across applet reloads.

## Invariants

- **Strict Layering**: `pi-rpc` never imports `libcosmic`, `iced`, or desktop platform crates. `cosmic-ext-pi` depends only on `pi-rpc` and `libcosmic` (no `cosmic-config`, `notify-rust`, `rfd`, or applet popup APIs).
- **Single Active Subprocess per Applet Session**: Only one host process owns a project's SQLite database at a time; when the applet restarts or switches project directories, the previous `PiClient` is shut down (closing `stdin` and waiting up to 5 seconds before `SIGKILL`, plus `kill_on_drop(true)` on the child handle) before opening the replacement session.
- **Strict LF Framing**: JSONL transport splits records strictly on `0x0A` (`\n`, stripping optional `0x0D`), never on Unicode line/paragraph separators (`U+2028`/`U+2029`).
- **Non-blocking UI Thread**: All child I/O, `rfd` folder/file dialogs, and `notify-rust` DBus calls execute on background tasks or blocking thread-pool workers, communicating back to the Iced state machine via typed `Message` / `PiAction` values.

## Out of scope

- Loading interactive CLI/TUI extensions or MCP servers from `~/.pi/agent` into `pi-durable` (provider extensions registered via `createAgentSessionServices`, `enabledModels` scoping, model/auth configuration from `~/.pi/agent`, and built-in `CodingTools` are loaded).
- Multi-applet shared background daemon multiplexing across separate applet instances.
