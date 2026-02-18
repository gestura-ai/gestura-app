# gestura-core

Public **facade crate** for the Gestura voice-first AI assistant. This crate is the single stable API surface — it re-exports types from all domain crates and owns the integration logic that ties them together.

## Role

`gestura-core` is **not** where most new business logic should be added. Instead:

1. Domain logic lives in the appropriate `gestura-core-*` crate.
2. `gestura-core` re-exports those types so downstream consumers (CLI, GUI) use a single, stable import path: `gestura_core::*`.
3. Integration code that bridges multiple domain crates (e.g., the agent pipeline, config loading with keychain, orchestrator) lives here.

## Core-owned modules

These modules contain integration logic that depends on multiple domain crates and therefore cannot live in any single one:

| Module | Description |
|--------|-------------|
| `pipeline/` | Agentic loop, streaming execution, tool dispatch |
| `agent_sessions/` | Session persistence and conversation history |
| `config.rs` | Config loading/saving with keychain bridge (security-dependent) |
| `llm_provider.rs` | LLM provider selection facade |
| `llm_validation.rs` | Provider configuration validation |
| `llm_overrides.rs` | Runtime LLM parameter overrides |
| `openai_compat.rs` | OpenAI-compatible API parameter quirks |
| `orchestrator.rs` | Subagent coordination and task delegation |
| `checkpoints/` | Session state snapshots with retention policies |
| `compaction.rs` | Context compaction (history trimming within token limits) |
| `guardrails/` | Project-specific instruction file discovery and injection |
| `prompt_enhancement.rs` | Auto-enhance prompts with configurable styles |
| `speech.rs` | Speech-to-text processing (Whisper local + OpenAI) |
| `streaming.rs` | Shell streaming and cancellation tokens |
| `tools/` | Tool facade — re-exports + schema builders for MCP integration |

## Re-exported domain crates

All domain crate types are surfaced as inline modules so consumers import from `gestura_core::*`:

| Inline module | Source crate |
|---------------|-------------|
| `error`, `events`, `execution_mode`, `interaction`, `model_display`, `platform`, `stream_error`, `stream_health`, `stream_reconnect`, `telemetry`, `secrets` | `gestura-core-foundation` |
| `default_models`, `model_listing`, `token_tracker` | `gestura-core-llm` |
| `mcp` | `gestura-core-mcp` |
| `session_manager`, `session_workspace` | `gestura-core-sessions` |
| `stream_cancellation` | `gestura-core-streaming` |
| `tasks`, `workflows` | `gestura-core-tasks` |
| `security`, `gdpr`, `sandbox` | `gestura-core-security` |
| `audio`, `audio_capture`, `stt_provider` | `gestura-core-audio` |
| `tool_inspection`, `tool_confirmation` | `gestura-core-tools` |
| `config_env` | `gestura-core-config` |
| `memory_bank` | `gestura-core-memory-bank` |
| `a2a` | `gestura-core-a2a` |
| `explorer` | `gestura-core-explorer` |
| `knowledge` | `gestura-core-knowledge` |
| `nats_mq` | `gestura-core-nats` |
| `hotkey_ipc` | `gestura-core-ipc` |
| `analytics`, `recommendations` | `gestura-core-analytics` |
| `hooks` | `gestura-core-hooks` |
| `scripting` | `gestura-core-scripting` |
| `plugin_system` | `gestura-core-plugins` |
| `retry` | `gestura-core-retry` |
| `agents` | `gestura-core-agents` |
| `context` | `gestura-core-context` |

## Feature flags

| Feature | Default | Description |
|---------|---------|-------------|
| `voice-local` | ✅ | Local Whisper speech-to-text via whisper-rs |
| `nats` | | NATS messaging integration |
| `json-ld` | | JSON-LD processing for MDH |
| `security` | | Encryption, keychain, secure config |
| `macos-permissions` | | macOS TCC permission dialogs (objc/cocoa) |
| `linux-permissions` | | Linux xdg-desktop-portal integration |
| `windows-permissions` | | Windows permission stubs |

## Usage

Downstream crates (CLI, GUI) depend on `gestura-core` and import through the facade:

```rust
use gestura_core::{AppConfig, AgentPipeline, AgentRequest, AgentResponse};
use gestura_core::tools::{all_tools, ToolDefinition};
use gestura_core::mcp::McpClientRegistry;
```

## Development

```bash
cargo test -p gestura-core
cargo clippy -p gestura-core --all-targets --all-features -- -D warnings
```

