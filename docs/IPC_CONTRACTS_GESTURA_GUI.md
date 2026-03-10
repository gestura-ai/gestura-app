# Frontend ↔ Tauri IPC Guide (gestura-gui)

## Status

This file is now a **navigation and maintenance guide**, not a hand-maintained,
exhaustive command inventory.

The old file:line inventory became stale once the frontend moved toward shared
Tauri service wrappers and the backend command surface grew substantially.

## Canonical Sources

Use these in order when working on GUI IPC:

1. `crates/gestura-gui/frontend/src/services/tauri/invoke.ts`
   - shared frontend IPC wrapper
   - central error normalization behavior
2. `crates/gestura-gui/frontend/src/services/tauri/*.ts`
   - feature-specific frontend contract wrappers and TypeScript return types
3. `crates/gestura-gui/src/main.rs`
   - `tauri::generate_handler![...]` registration list for exposed commands
4. `crates/gestura-gui/src/api.rs` and `crates/gestura-gui/src/commands/*.rs`
   - owning Rust `#[tauri::command]` handlers
5. generated docs for crate/module reference:

```bash
cargo doc -p gestura-gui --no-deps
```

## Contract Rules

1. **Prefer service wrappers over direct `invoke()` usage.**
   - Frontend code should usually call `frontend/src/services/tauri/*.ts`
     instead of invoking commands directly from components.
2. **Command names are `snake_case`.**
   - They should match the Rust `#[tauri::command]` function name.
3. **Payload keys should be `snake_case` unless documented otherwise.**
   - Especially important when Rust uses `#[tauri::command(rename_all = "snake_case")]`.
4. **Injected Tauri parameters are not passed from JS.**
   - Examples: `tauri::State<'_, AppState>`, `AppHandle`, `Window`,
     `WebviewWindow`.
5. **Treat `serde_json::Value` / `unknown` results defensively.**
   - Prefer adding a typed frontend wrapper once the shape is stable.
6. **Do not log raw IPC payloads containing secrets.**
   - The shared wrapper intentionally avoids logging arguments.

## How to Navigate the IPC Surface

### Frontend side

Look under:

- `frontend/src/services/tauri/config.ts`
- `frontend/src/services/tauri/mcp.ts`
- `frontend/src/services/tauri/simulator.ts`
- `frontend/src/services/tauri/workflows.ts`
- `frontend/src/services/tauri/editor.ts`
- `frontend/src/services/tauri/explorer.ts`
- `frontend/src/services/tauri/voice.ts`
- `frontend/src/services/tauri/agent.ts` / `agents.ts`

These wrappers are the preferred place to document payload keys and frontend
return types for active UI usage.

### Backend side

Look under:

- `crates/gestura-gui/src/main.rs` for registration
- `crates/gestura-gui/src/api.rs` for the main command surface
- `crates/gestura-gui/src/commands/` for feature-specific command modules

## High-Risk / Weakly Typed Areas

These areas deserve extra care because the contract is intentionally looser or
more event-driven:

- commands returning `serde_json::Value`
- streaming commands that coordinate with Tauri events/windows
- commands carrying user-entered configuration or secrets
- long-running workflow/editor/shell surfaces where frontend wrappers should own
  stable TypeScript shapes

In those cases, prefer:

- a typed wrapper in `frontend/src/services/tauri/*.ts`
- Rustdoc on the command handler for payload/return semantics
- focused contract notes only when the behavior is hard to infer from source

## What This File No Longer Tries to Do

This file no longer maintains:

- a file:line inventory of every frontend callsite
- an exhaustive command table for all Tauri commands
- manually synchronized JSON shape listings for every loosely typed response

Those are better sourced from the frontend service wrappers, Rust command docs,
and generated documentation.
