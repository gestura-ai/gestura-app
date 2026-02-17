# gestura-core-agents

Agent lifecycle management, spawning, and task delegation for Gestura.

## What belongs here

- Agent data model: info, status, commands, envelopes
- `AgentManager` — spawn, track, and shut down agent tasks
- `AgentSpawner` — create agent processes with IPC channels
- Task delegation types (`DelegatedTask`, `TaskResult`, `OrchestratorToolCall`)

GUI-specific orchestration (e.g., Tauri `AppHandle` integration) remains in
the GUI crate. The orchestrator that wires agents to `AgentPipeline` stays in
`gestura-core`.

## Key types

| Type | Description |
|------|-------------|
| `AgentManager` | Manages running agent tasks |
| `AgentSpawner` | Creates new agent processes |
| `AgentInfo` | Agent metadata (id, name, capabilities) |
| `AgentStatus` | `Running` or `Stopped` |
| `AgentCommand` | Commands sent to agents (`Shutdown`, `Event`) |
| `AgentEnvelope` | IPC message envelope (agent_id, subject, payload) |
| `DelegatedTask` / `TaskResult` | Task delegation and result types |
| `OrchestratorToolCall` | Tool call routing for the orchestrator |

## Stable import paths

Most code should import through the facade:

- `gestura_core::agents::*`

The facade in `crates/gestura-core/src/lib.rs` re-exports this crate.

## Development

```bash
cargo test -p gestura-core-agents
cargo clippy -p gestura-core-agents --all-targets --all-features -- -D warnings
```

