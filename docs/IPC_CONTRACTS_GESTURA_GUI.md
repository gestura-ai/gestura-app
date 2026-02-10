<!--
NOTE: This doc is intentionally verbose and is meant to be the durable artifact for
Phase 1.1 (IPC inventory). Keep it up to date whenever adding/modifying GUI IPC.
-->

# Frontend ↔ Tauri IPC Contracts (gestura-gui)

Last updated: 2026-02-10

This document inventories all `invoke()` calls in `crates/gestura-gui/frontend/src/**` and maps them to their Rust `#[tauri::command]` handlers in `crates/gestura-gui/src/**`.

## Key rules / conventions

1. **Command names are `snake_case`** and must match the Rust function name annotated with `#[tauri::command]`.
2. **Payload keys should be `snake_case`** unless the Rust command explicitly documents otherwise.
   - Many commands opt into `#[tauri::command(rename_all = "snake_case")]`, which means JS keys must be snake_case.
3. **Injected parameters are not sent from JS.** Tauri injects values like `tauri::State<'_, AppState>`, window handles, etc.
4. **`serde_json::Value` return types are “shape varies”.** Where practical, this doc records the observed/implemented JSON shape.

## Inventory (from frontend `invoke()` usage)

Columns:
- **frontend call sites**: file:line (multiple call sites separated with `<br/>`)
- **rust command**: the Rust source file containing the `#[tauri::command]` function
- **js args (keys)**: keys expected in the JS payload object (excluding injected args)
- **returns**: Rust success type (JS receives the serialized JSON value)

| command | frontend call sites | rust command | js args (keys) | returns | notes |
|---|---|---|---|---|---|
| `add_mcp_tool` | crates/gestura-gui/frontend/src/components/McpPanel.tsx:120<br/>crates/gestura-gui/frontend/src/components/McpPanel.tsx:140 | `crates/gestura-gui/src/api.rs` | `tool` | `null` |  |
| `call_mcp_tool` | crates/gestura-gui/frontend/src/components/McpPanel.tsx:173 | `crates/gestura-gui/src/api.rs` | `server, tool, arguments` | `serde_json::Value` | JSON value (see “Known JSON shapes”) |
| `cancel_task` | crates/gestura-gui/frontend/src/components/WorkflowsPanel.tsx:77 | `crates/gestura-gui/src/api.rs` | `task_id` | `null` |  |
| `check_system_permissions` | crates/gestura-gui/frontend/src/components/OnboardingWizard.tsx:103 | `crates/gestura-gui/src/api.rs` | `—` | `serde_json::Value` | JSON value (see “Known JSON shapes”) |
| `connect_mcp_server` | crates/gestura-gui/frontend/src/components/McpPanel.tsx:150 | `crates/gestura-gui/src/api.rs` | `name` | `Vec<String>` |  |
| `delegate_task` | crates/gestura-gui/frontend/src/components/WorkflowsPanel.tsx:66 | `crates/gestura-gui/src/api.rs` | `task` | `String` |  |
| `disconnect_mcp_server` | crates/gestura-gui/frontend/src/components/McpPanel.tsx:161 | `crates/gestura-gui/src/api.rs` | `name` | `null` |  |
| `get_config` | crates/gestura-gui/frontend/src/App.tsx:60 | `crates/gestura-gui/src/api.rs` | `—` | `AppConfig` | attr: (rename_all = "snake_case") |
| `get_mcp_server_status` | crates/gestura-gui/frontend/src/components/McpPanel.tsx:71 | `crates/gestura-gui/src/api.rs` | `—` | `Vec<McpServerStatus>` |  |
| `get_nats_status` | crates/gestura-gui/frontend/src/components/StatusBar.tsx:19 | `crates/gestura-gui/src/api.rs` | `—` | `bool` |  |
| `get_ring_status` | crates/gestura-gui/frontend/src/components/RingPanel.tsx:49<br/>crates/gestura-gui/frontend/src/components/StatusBar.tsx:44 | `crates/gestura-gui/src/api.rs` | `device_id` | `Option<crate::ble::RingStatus>` | attr: (rename_all = "snake_case") |
| `get_simulator_logs` | crates/gestura-gui/frontend/src/components/SimulatorPanel.tsx:112 | `crates/gestura-gui/src/commands/simulator.rs` | `device_id` | `Vec<String>` |  |
| `get_simulators` | crates/gestura-gui/frontend/src/components/SimulatorPanel.tsx:44 | `crates/gestura-gui/src/commands/simulator.rs` | `—` | `HashMap<String, SimulatorInfo>` |  |
| `is_developer_mode_enabled` | crates/gestura-gui/frontend/src/components/SimulatorPanel.tsx:59 | `crates/gestura-gui/src/commands/simulator.rs` | `—` | `bool` |  |
| `list_active_tasks` | crates/gestura-gui/frontend/src/components/WorkflowsPanel.tsx:43 | `crates/gestura-gui/src/api.rs` | `—` | `Vec<crate::orchestrator::DelegatedTask>` |  |
| `list_agents` | crates/gestura-gui/frontend/src/components/StatusBar.tsx:18<br/>crates/gestura-gui/frontend/src/components/WorkflowsPanel.tsx:44 | `crates/gestura-gui/src/api.rs` | `—` | `serde_json::Value` | JSON value (see “Known JSON shapes”) |
| `list_builtin_tools` | crates/gestura-gui/frontend/src/components/ToolsPanel.tsx:40 | `crates/gestura-gui/src/api.rs` | `—` | `Vec<ToolInfo>` |  |
| `list_connected_mcp_servers` | crates/gestura-gui/frontend/src/components/McpPanel.tsx:72 | `crates/gestura-gui/src/api.rs` | `—` | `Vec<String>` |  |
| `list_mcp_client_tools` | crates/gestura-gui/frontend/src/components/McpPanel.tsx:73 | `crates/gestura-gui/src/api.rs` | `—` | `Vec<McpClientToolInfo>` |  |
| `list_mcp_tools` | crates/gestura-gui/frontend/src/components/McpPanel.tsx:70<br/>crates/gestura-gui/frontend/src/components/ToolsPanel.tsx:41 | `crates/gestura-gui/src/api.rs` | `—` | `Vec<crate::config::McpServerEntry>` |  |
| `open_system_preferences` | crates/gestura-gui/frontend/src/components/OnboardingWizard.tsx:135 | `crates/gestura-gui/src/api.rs` | `pane` | `null` |  |
| `pair_ring` | crates/gestura-gui/frontend/src/components/OnboardingWizard.tsx:322<br/>crates/gestura-gui/frontend/src/components/RingPanel.tsx:38 | `crates/gestura-gui/src/api.rs` | `device_id` | `null` | attr: (rename_all = "snake_case") |
| `register_consent` | crates/gestura-gui/frontend/src/components/OnboardingWizard.tsx:393 | `crates/gestura-gui/src/api.rs` | `user_id, category, purpose` | `null` |  |
| `remove_mcp_tool` | crates/gestura-gui/frontend/src/components/McpPanel.tsx:130 | `crates/gestura-gui/src/api.rs` | `name` | `null` |  |
| `request_permission` | crates/gestura-gui/frontend/src/components/OnboardingWizard.tsx:126 | `crates/gestura-gui/src/api.rs` | `permission` | `null` |  |
| `reset_simulator` | crates/gestura-gui/frontend/src/components/SimulatorPanel.tsx:86 | `crates/gestura-gui/src/commands/simulator.rs` | `device_id` | `null` |  |
| `run_simulator_test` | crates/gestura-gui/frontend/src/components/SimulatorPanel.tsx:103 | `crates/gestura-gui/src/commands/simulator.rs` | `device_id` | `TestResults` |  |
| `run_voice_once` | crates/gestura-gui/frontend/src/components/VoicePanel.tsx:33 | `crates/gestura-gui/src/api.rs` | `—` | `String` |  |
| `save_config` | crates/gestura-gui/frontend/src/App.tsx:71 | `crates/gestura-gui/src/api.rs` | `cfg` | `null` | attr: (rename_all = "snake_case") |
| `scan_for_rings` | crates/gestura-gui/frontend/src/components/OnboardingWizard.tsx:306<br/>crates/gestura-gui/frontend/src/components/RingPanel.tsx:22<br/>crates/gestura-gui/frontend/src/components/StatusBar.tsx:20 | `crates/gestura-gui/src/api.rs` | `—` | `Vec<String>` |  |
| `scan_for_simulators` | crates/gestura-gui/frontend/src/components/SimulatorPanel.tsx:74 | `crates/gestura-gui/src/commands/simulator.rs` | `—` | `Vec<String>` |  |
| `send_haptic_feedback` | crates/gestura-gui/frontend/src/components/RingPanel.tsx:60 | `crates/gestura-gui/src/api.rs` | `device_id, pattern, intensity, duration_ms` | `null` | attr: (rename_all = "snake_case") |
| `send_test_haptic` | crates/gestura-gui/frontend/src/components/SimulatorPanel.tsx:95 | `crates/gestura-gui/src/commands/simulator.rs` | `device_id, pattern_type` | `null` |  |
| `set_ui_prefs` | crates/gestura-gui/frontend/src/App.tsx:80 | `crates/gestura-gui/src/api.rs` | `ui` | `null` |  |
| `spawn_subagent` | crates/gestura-gui/frontend/src/components/WorkflowsPanel.tsx:88 | `crates/gestura-gui/src/api.rs` | `agent_id, name` | `null` |  |
| `start_gesture_monitoring` | crates/gestura-gui/frontend/src/components/RingPanel.tsx:78 | `crates/gestura-gui/src/api.rs` | `device_id` | `null` | attr: (rename_all = "snake_case") |
| `stop_gesture_monitoring` | crates/gestura-gui/frontend/src/components/RingPanel.tsx:76 | `crates/gestura-gui/src/api.rs` | `device_id` | `null` | attr: (rename_all = "snake_case") |
| `test_voice` | crates/gestura-gui/frontend/src/components/OnboardingWizard.tsx:246<br/>crates/gestura-gui/frontend/src/components/VoicePanel.tsx:19 | `crates/gestura-gui/src/api.rs` | `—` | `String` |  |
| `toggle_developer_mode` | crates/gestura-gui/frontend/src/components/SimulatorPanel.tsx:121 | `crates/gestura-gui/src/commands/simulator.rs` | `enabled` | `null` |  |

Total frontend commands inventoried: **39**

## Known JSON shapes (for `serde_json::Value` returns)

### `list_agents`

Rust implementation returns:

```json
{ "agents": <array>, "count": <number> }
```

Notes:
- The exact element shape of `agents` is determined by the Rust agent model serialization.

### `check_system_permissions`

Rust implementation returns an object:

```json
{
  "permissions": [
    {
      "id": "microphone" | "accessibility" | "bluetooth" | "screen_recording",
      "name": "...",
      "description": "...",
      "status": "granted" | "denied" | "not_determined" | "...",
      "required": true | false,
      "instructions": "..."
    }
  ],
  "total_count": <number>,
  "granted_count": <number>,
  "required_count": <number>,
  "required_granted_count": <number>,
  "missing_required_count": <number>,
  "summary": {
    "total": <number>,
    "granted": <number>,
    "required": <number>,
    "required_granted": <number>,
    "missing_required": <number>
  }
}
```

Notes:
- The `summary.total/granted/required` keys are intentionally kept for **back-compat** with the existing UI.

### `call_mcp_tool`

Returns arbitrary JSON produced by the called tool (serialized from the Rust-side tool result). Frontend should treat this as `unknown`/untyped JSON and render defensively.
