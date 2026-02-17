# Code Organization and Structure

## Workspace Architecture

Gestura.app uses a **Core-First Architecture** organized as a Rust workspace.

`gestura-core` remains the **public facade** and source-of-truth API surface, while focused
domain crates hold independently-workable subsystems (e.g., tools) and shared primitives.

```
gestura-app/
├── Cargo.toml                     # Workspace manifest
├── crates/
│   ├── gestura-core/              # Public facade + remaining core domains
│   ├── gestura-core-foundation/   # Shared primitives used across domain crates
│   ├── gestura-core-tools/        # Built-in tools + tool policy/permissions/registry
│   ├── gestura-core-mcp/          # MCP protocol domain crate (implementation)
│   ├── gestura-core-llm/          # LLM providers domain crate (implementation)
│   ├── gestura-cli/               # CLI binary (thin presentation layer)
│   └── gestura-gui/               # Tauri desktop app (thin presentation layer)
├── docs/                          # Documentation
└── AGENTS.md                      # AI assistant guide
```

## Core-First Design Principles

1. **Single Source of Truth**: All business logic lives in `gestura-core`
2. **Thin Presentation Layers**: CLI and GUI delegate to core
3. **Re-export Pattern**: GUI/CLI modules re-export core types
4. **Feature Gates**: Optional functionality via Cargo features

## gestura-core (Shared Library)

The core crate contains all business logic organized by domain:

### AI & Pipeline
| Module | Files | Description |
|--------|-------|-------------|
| `pipeline/` | `mod.rs`, `types.rs` | Agent request/response pipeline |
| `llm_provider.rs` | - | **Facade** preserving `gestura_core::llm_provider::*` (implementation lives in `gestura-core-llm`) |
| `persona.rs` | - | System persona configuration |
| `speech.rs` | - | Text-to-speech |
| `stt_provider.rs` | - | Speech-to-text (Whisper) |

### Session Management
| Module | Files | Description |
|--------|-------|-------------|
| `agent_sessions/` | `mod.rs`, `types.rs`, `store.rs` | Session persistence |
| `session_manager.rs` | - | Active session lifecycle |
| `context/` | `mod.rs`, `types.rs`, `manager.rs`, `analyzer.rs`, `cache.rs` | Context window management |
| `memory_bank/` | `mod.rs` | Long-term memory storage |

### Tools & Permissions
| Module | Files | Description |
|--------|-------|-------------|
| `tools/` | `mod.rs`, `registry.rs`, `permissions.rs`, `policy.rs`, `schemas.rs` | **Facade** re-exporting the tools domain and providing core-owned adapters |
| `gestura-core-tools` | `src/file.rs`, `src/shell.rs`, `src/git.rs`, `src/web.rs`, `src/code.rs`, ... | Built-in tool implementations + registry/policy/permissions |
| `tool_confirmation.rs` | - | User confirmation flow with pause/resume |
| `tool_inspection.rs` | - | Tool introspection |

## Domain crates (focused work)

### gestura-core-foundation (Shared primitives)

Small, dependency-light building blocks used by multiple domains.

- `error` — shared `AppError` / `Result`
- `permissions` — shared permission primitives
- `execution_mode` — execution-mode model/policy (re-exported via `gestura_core::execution_mode`)

### gestura-core-tools (Tools domain)

All built-in tool implementations and tool policy live here.

Developers working on “tools” should start in `crates/gestura-core-tools/`.

### Protocols
| Module | Files | Description |
|--------|-------|-------------|
| `mcp/` | `mod.rs` | **Facade** preserving `gestura_core::mcp::*` (implementation lives in `gestura-core-mcp`) |
| `a2a/` | `mod.rs`, `server.rs`, `client.rs`, `types.rs` | Agent-to-Agent protocol |
| `nats_mq/` | `mod.rs` | NATS message queue |

### gestura-core-mcp (MCP domain)

The MCP (Model Context Protocol) implementation lives in `crates/gestura-core-mcp/`.

- Stable import path: `gestura_core::mcp::*` (facade in `crates/gestura-core/src/mcp/mod.rs`)
- MCP configuration types (e.g., `.mcp.json` helpers) are re-exported from `gestura_core::config::*`
  but owned by `gestura-core-mcp` to ensure a single source of truth.

### gestura-core-llm (LLM providers domain)

LLM provider implementations live in `crates/gestura-core-llm/`.

- Stable import path: `gestura_core::llm_provider::*` (facade in `crates/gestura-core/src/llm_provider.rs`)
- Provider implementations are feature-gated in the domain crate (e.g., `openai`, `anthropic`, `grok`, `ollama`; plus `dev` for dev/test-only providers)

### Security & Sandboxing
| Module | Files | Description |
|--------|-------|-------------|
| `security/` | `mod.rs`, `encryption.rs`, `storage.rs` | Encryption & secure storage |
| `sandbox/` | `mod.rs` | Sandboxed execution |
| `gdpr.rs` | - | Privacy compliance |

### Analytics & AI
| Module | Files | Description |
|--------|-------|-------------|
| `analytics/` | `mod.rs` | Usage analytics with privacy modes |
| `recommendations/` | `mod.rs` | ML-based recommendations |
| `audio/` | `mod.rs` | Noise cancellation (spectral subtraction) |

### Extensibility
| Module | Files | Description |
|--------|-------|-------------|
| `scripting/` | `mod.rs`, `runtime.rs` | Multi-language scripting engine |
| `agents/` | `mod.rs` | Agent orchestration and spawning |
| `tasks/` | `mod.rs` | Task management |

### Infrastructure
| Module | Files | Description |
|--------|-------|-------------|
| `config.rs` | - | Configuration management |
| `error.rs` | - | Error types (thiserror) |
| `telemetry.rs` | - | OpenTelemetry integration |
| `knowledge/` | `mod.rs`, `types.rs`, `store.rs` | Knowledge base |
| `streaming.rs` | - | Streaming responses |
| `retry.rs` | - | Retry logic |

## gestura-cli (CLI Binary)

Thin command-line interface:

```
gestura-cli/src/
├── main.rs              # Entry point with clap
├── tool_registry.rs     # CLI tool registration
└── commands/
    ├── mod.rs           # Command routing
    ├── agent/            # Interactive agent TUI
    │   ├── mod.rs
    │   └── tui/         # Ratatui TUI implementation
    ├── exec.rs          # One-shot execution
    ├── listen.rs        # Voice capture
    ├── session.rs       # Session management
    ├── mcp.rs           # MCP commands
    ├── a2a.rs           # A2A protocol
    ├── agent.rs         # Agent management
    ├── config.rs        # Configuration
    ├── context.rs       # Context commands
    ├── device.rs        # Device commands
    ├── health.rs        # Health check
    ├── init.rs          # Initialize project
    ├── knowledge.rs     # Knowledge base
    ├── model.rs         # Model selection
    ├── privacy.rs       # Privacy settings
    ├── completion.rs    # Shell completions
    └── tools/           # Tool subcommands
```

## gestura-gui (Tauri Desktop App)

Thin Tauri wrapper with platform-specific integrations:

```
gestura-gui/src/
├── main.rs              # Tauri entry point
├── lib.rs               # Module exports
├── api.rs               # Tauri commands
│
├── # Thin Re-export Wrappers (delegate to gestura-core)
├── a2a.rs               # Re-exports gestura_core::a2a
├── security.rs          # Re-exports gestura_core::security
├── sandbox.rs           # Re-exports gestura_core::sandbox
├── scripting_engine.rs  # Re-exports gestura_core::scripting
├── nats_mq.rs           # Re-exports gestura_core::nats_mq
├── agents.rs            # Re-exports gestura_core::agents
├── noise_cancellation.rs # Re-exports gestura_core::audio
├── usage_analytics.rs   # Re-exports gestura_core::analytics
├── personalized_recommendations.rs # Re-exports gestura_core::recommendations
├── permissions.rs       # Re-exports gestura_core::tools::permissions
│
├── # Platform-specific (remain in GUI)
├── window_manager.rs    # Window management
├── tray.rs              # System tray
├── hotkeys.rs           # Global shortcuts
├── haptics.rs           # Haptic feedback
├── ble.rs               # Bluetooth (platform-specific)
├── mcp_server.rs        # MCP transport adapter
│
├── # UI Components
├── ui/                  # UI utilities
├── commands/            # Additional Tauri commands
├── features/            # Feature modules
├── integrations/        # External integrations
└── utils/               # Utility modules
```

## Re-export Pattern

GUI modules are thin wrappers that re-export core functionality:

```rust
// crates/gestura-gui/src/security.rs (18 lines)
//! Security primitives - thin wrapper over gestura_core::security
pub use gestura_core::security::{
    Encryptor, McpToken, SecureStorage, create_secure_storage,
};
```

```rust
// crates/gestura-gui/src/a2a.rs (7 lines)
//! A2A protocol - thin wrapper over gestura_core::a2a
pub use gestura_core::a2a::*;
```

## Feature Gates

Optional functionality controlled via Cargo features in `gestura-core`:

| Feature | Description | Dependencies |
|---------|-------------|--------------|
| `security` | Encryption & secure storage | `keyring`, `aes-gcm` |
| `nats` | NATS message queue | `async-nats` |
| `ble` | Bluetooth support | `btleplug` |
| `analytics` | Usage analytics | - |

## Adding New Features

### 1. Implement in Core
```rust
// gestura-core/src/new_feature/mod.rs
pub mod types;
mod implementation;

pub use types::*;
pub use implementation::*;
```

### 2. Export from lib.rs
```rust
// gestura-core/src/lib.rs
#[cfg(feature = "new_feature")]
pub mod new_feature;
```

### 3. Create GUI Wrapper
```rust
// gestura-gui/src/new_feature.rs
//! New feature - thin wrapper over gestura_core::new_feature
pub use gestura_core::new_feature::*;
```

### 4. Add CLI Command (if needed)
```rust
// gestura-cli/src/commands/new_feature.rs
use gestura_core::new_feature::*;

pub async fn run(args: Args) -> Result<()> {
    // Implementation using core types
}
```

## Migration Status

The Core-First architecture migration was completed in 6 phases:

| Phase | Description | Status |
|-------|-------------|--------|
| Phase 1 | A2A consolidation | ✅ Complete |
| Phase 2 | Agent session unification | ✅ Complete |
| Phase 3 | Permissions + security policy | ✅ Complete |
| Phase 4 | MCP domain extraction (`gestura-core-mcp`) | ✅ Complete |
| Phase 5 | GUI subsystems migration | ✅ Complete |
| Phase 6 | Analytics/Recommendations/Audio | ✅ Complete |

### Code Reduction Results

| GUI Module | Before | After | Reduction |
|------------|--------|-------|-----------|
| a2a.rs | 1,092 lines | 7 lines | 99.4% |
| security.rs | 265 lines | 18 lines | 93.2% |
| sandbox.rs | 326 lines | 7 lines | 97.9% |
| scripting_engine.rs | 679 lines | 10 lines | 98.5% |
| nats_mq.rs | 439 lines | 13 lines | 97.0% |
| usage_analytics.rs | 772 lines | 10 lines | 98.7% |
| personalized_recommendations.rs | 649 lines | 10 lines | 98.5% |
| noise_cancellation.rs | 476 lines | 10 lines | 97.9% |
