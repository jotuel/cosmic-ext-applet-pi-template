---
id: pi-rpc
type: module-design
status: active
title: pi-rpc — Headless Pi Durable RPC Client & Process Supervisor
parent: architecture
---

## Responsibility

Headless, pure-Rust client (`PiClient`), child process supervisor (`PiProcess`), bundled `@earendil-works/pi-durable` Node host (`host/durable_host.ts` → `src/durable_host.cjs`), JSONL transport (`JsonLinesReader` / `JsonLinesWriter`), and native `pi-durable` wire protocol definitions (`protocol`).

- **Bundled Pi Durable Host & Process Supervision (`process`, `host/durable_host.ts`)**:
  - Materializes `durable_host.cjs` atomically to `$XDG_CACHE_HOME/cosmic-ext-applet-pi/durable_host.cjs` (guarded by an in-process mutex and UUID temp file to avoid parallel spawn races) and launches it under `node --no-warnings`.
  - Resolves one SQLite database per canonical project directory under `$XDG_DATA_HOME/cosmic-ext-applet-pi/sessions/<slug>-<hash>.sqlite` (or `--session-dir` / `--no-session` `MemoryStorage` for tests).
  - Loads credentials, `models.json`, `models-store.json`, provider extensions (such as `antigravity` from `~/.pi/agent/npm`), and `enabledModels` (`scopedModels`) from `~/.pi/agent` via `@earendil-works/pi-coding-agent`'s `createAgentSessionServices` and `resolveModelScopeWithDiagnostics`, passing the `ModelRuntime` instance to `Harness.open` so `ModelRuntime.prepareRequest` resolves provider auth and headers (falling back to `@earendil-works/pi-ai` `fauxProvider` in isolated test environments).
  - Resolves bare `--model <id>` and `set_model` references against `scopedModels` first when `enabledModels` is configured, and repairs persisted session models that fall outside `scopedModels`.
  - Installs `CodingTools` (`read`, `write`, `edit`, `bash`) with `NodeExecutionEnv`, attaches `watchEvents`, emits the initial `SnapshotEvent` (with inbox items enriched with their input text), and calls `harness.resume()`.
- **Transport Framing (`transport`)**: Reads and writes single-line JSON records over arbitrary `AsyncRead` / `AsyncWrite` streams, splitting strictly on byte `0x0A` (`\n`, with optional `0x0D`) so embedded Unicode line/paragraph separators (`U+2028`, `U+2029`) are preserved intact.
- **Multiplexing, Snapshot Caching & Correlation (`client`, `protocol`)**:
  - Assigns a UUID to each outbound `RpcCommand` (`Submit`, `Steer`, `FollowUp`, `Abort`, `ClearQueue`, `Reset`, `Configure`, `GetState`, `SetModel`, `CycleModel`, `GetAvailableModels`, `SetThinkingLevel`, `CycleThinkingLevel`, `GetAvailableThinkingLevels`, `Compact`, `Bash`, `GetSessionStats`, `SetSessionName`, `GetMessages`, `GetCommands`) and demultiplexes incoming stdout records (`IncomingRecord`) into `Response`, `ExtensionUi`, and `Event(PiEvent)`.
  - Caches the most recent `SnapshotEvent` on `PiClient::latest_snapshot()` so subscribers attaching after process startup immediately receive the initial durable snapshot.

## Boundary

- **Allowed dependencies**: `tokio`, `serde`, `serde_json`, `uuid`, `thiserror`, `tracing`.
- **Forbidden reaches**: Must remain completely headless and runtime-agnostic regarding GUI toolkits — never import `libcosmic`, `iced`, desktop notification crates, or applet configuration crates.
