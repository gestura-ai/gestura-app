# Gestura.app Developer Guide

## Table of Contents

1. [Getting Started](#getting-started)
2. [Development Environment](#development-environment)
3. [Core-First Architecture](#core-first-architecture)
4. [Adding New Features](#adding-new-features)
5. [Scripting](#scripting)
6. [API Integration](#api-integration)
7. [Testing](#testing)
8. [Deployment](#deployment)

## Getting Started

Welcome to Gestura.app development! This guide covers the Core-First architecture and how to contribute to the codebase.

### Prerequisites

- **Rust 2024 Edition**: Install via rustup
- **Tauri CLI**: For GUI development
- **Development Tools**: VS Code with rust-analyzer, terminal, Git

### Quick Start

```bash
# Clone the repository
git clone https://github.com/gestura-ai/gestura-app.git
cd gestura-app

# Install Tauri CLI
cargo install tauri-cli

# Run quality gates
cargo fmt
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features

# Development
cargo tauri dev  # GUI development
cargo run -p gestura-cli -- --help  # CLI
```

## Development Environment

### Workspace Structure

```
gestura-app/
├── Cargo.toml              # Workspace manifest
├── crates/
│   ├── gestura-core/       # Shared business logic (source of truth)
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs      # Re-exports
│   │       ├── pipeline/   # Agent pipeline
│   │       ├── tools/      # Tool registry & implementations
│   │       ├── mcp/        # MCP server
│   │       ├── a2a/        # A2A protocol
│   │       └── ...
│   ├── gestura-cli/        # CLI binary
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── main.rs     # Entry point
│   │       └── commands/   # CLI commands
│   └── gestura-gui/        # Tauri desktop app
│       ├── Cargo.toml
│       ├── tauri.conf.json
│       ├── frontend/       # Web frontend
│       └── src/
│           ├── main.rs     # Tauri entry
│           └── ...         # Thin wrappers
├── docs/                   # Documentation
└── AGENTS.md               # AI assistant guide
```

### Build Commands

```bash
# Format code
cargo fmt

# Lint with strict warnings
cargo clippy --workspace --all-targets --all-features -- -D warnings

# Run all tests
cargo test --workspace --all-features

# Build all crates
cargo build --workspace

# Release build
cargo build --workspace --release

# GUI development
cargo tauri dev

# CLI only
cargo run -p gestura-cli -- chat
```

## Core-First Architecture

### Design Principles

1. **Single Source of Truth**: All business logic in `gestura-core`
2. **Thin Presentation Layers**: CLI and GUI delegate to core
3. **Re-export Pattern**: GUI/CLI modules re-export core types
4. **Feature Gates**: Optional functionality via Cargo features

### Module Organization (gestura-core)

| Category | Modules | Description |
|----------|---------|-------------|
| AI & Pipeline | `pipeline/`, `llm_provider.rs`, `persona.rs` | Agent execution |
| Sessions | `chat_sessions/`, `session_manager.rs`, `context/` | State management |
| Tools | `tools/`, `tool_confirmation.rs` | Tool registry |
| Protocols | `mcp/`, `a2a/`, `nats_mq/` | Communication |
| Security | `security/`, `sandbox/`, `gdpr.rs` | Safety & privacy |
| Analytics | `analytics/`, `recommendations/`, `audio/` | ML features |
| Extensibility | `scripting/`, `agents/`, `tasks/` | Plugin system |

### Re-export Pattern

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

## Adding New Features

### Step 1: Implement in Core

Add business logic to `gestura-core`:

```rust
// crates/gestura-core/src/new_feature/mod.rs
pub mod types;
mod implementation;

pub use types::*;
pub use implementation::*;
```

```rust
// crates/gestura-core/src/new_feature/types.rs
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewFeatureConfig {
    pub enabled: bool,
    pub option: String,
}

#[derive(Debug, Clone)]
pub struct NewFeature {
    config: NewFeatureConfig,
}

impl NewFeature {
    pub fn new(config: NewFeatureConfig) -> Self {
        Self { config }
    }

    pub async fn execute(&self, input: &str) -> Result<String, NewFeatureError> {
        // Implementation
        Ok(format!("Processed: {}", input))
    }
}
```

### Step 2: Export from lib.rs

```rust
// crates/gestura-core/src/lib.rs
#[cfg(feature = "new_feature")]
pub mod new_feature;
```

### Step 3: Create GUI Wrapper

```rust
// crates/gestura-gui/src/new_feature.rs
//! New feature - thin wrapper over gestura_core::new_feature
pub use gestura_core::new_feature::*;
```

### Step 4: Add CLI Command

```rust
// crates/gestura-cli/src/commands/new_feature.rs
use clap::Args;
use gestura_core::new_feature::*;

#[derive(Args)]
pub struct NewFeatureArgs {
    /// Input to process
    #[arg(short, long)]
    pub input: String,
}

pub async fn run(args: NewFeatureArgs) -> anyhow::Result<()> {
    let config = NewFeatureConfig::default();
    let feature = NewFeature::new(config);
    let result = feature.execute(&args.input).await?;
    println!("{}", result);
    Ok(())
}
```

### Error Handling

Use `thiserror` for error types:

```rust
// crates/gestura-core/src/new_feature/mod.rs
use thiserror::Error;

#[derive(Debug, Error)]
pub enum NewFeatureError {
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Execution error: {0}")]
    Execution(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}
```

### Adding Tests

```rust
// crates/gestura-core/src/new_feature/mod.rs
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_new_feature() {
        let config = NewFeatureConfig {
            enabled: true,
            option: "test".to_string(),
        };
        let feature = NewFeature::new(config);
        let result = feature.execute("input").await.unwrap();
        assert!(result.contains("Processed"));
    }
}
```

## Permission System

### Permission Levels

| Level | Description | Example Actions |
|-------|-------------|-----------------|
| `ReadOnly` | Read files, view context | `file_read`, `git_status` |
| `WriteLocal` | Write to allowed paths | `file_write`, `file_edit` |
| `Execute` | Run local commands | `shell_exec`, `script_run` |
| `Network` | External API calls | `web_fetch`, `api_call` |
| `Admin` | Full system access | `install_package`, `root_exec` |

### Checking Permissions

```rust
use gestura_core::tools::{PermissionManager, PermissionLevel};

let perms = PermissionManager::new(config);

// Check if action is allowed
if perms.is_action_allowed(&request) {
    // Execute the action
}

// Check if confirmation is required
if perms.requires_confirmation(&tool_call) {
    // Prompt user for confirmation
}
```

## Scripting Engine

The scripting engine is implemented in `gestura-core/src/scripting/` and provides sandboxed execution of user scripts.

### Script Types

| Language | Engine | Features |
|----------|--------|----------|
| Lua | mlua | Lightweight, fast startup |
| Python | PyO3 | ML libraries, data processing |
| JavaScript | deno_core | Async, web APIs |

### Creating a Script

```rust
// crates/gestura-core/src/scripting/mod.rs
use crate::scripting::{Script, ScriptEngine, ScriptContext};

// Load and execute a script
let engine = ScriptEngine::new(config)?;
let script = Script::from_file("my_script.lua")?;

let context = ScriptContext {
    input: serde_json::json!({"location": "NYC"}),
    permissions: vec!["network".to_string()],
};

let result = engine.execute(&script, context).await?;
```

### Script Permissions

Scripts run in a sandbox with explicit permissions:

| Permission | Description |
|------------|-------------|
| `network` | HTTP requests |
| `filesystem` | File read/write (scoped) |
| `system` | Execute commands |
| `clipboard` | Clipboard access |

### Script API (from Lua)

```lua
-- @name Example Script
-- @version 1.0.0
-- @permission network

function main(args)
    -- HTTP requests
    local response = http.get("https://api.example.com/data")

    -- JSON parsing
    local data = json.decode(response.body)

    -- Notifications
    gestura.notify("Title", "Message")

    return { success = true, data = data }
end
```

## MCP Integration

### MCP Server (Core Implementation)

The MCP server is implemented in `gestura-core/src/mcp/` with full 2025-11-25 spec compliance.

```rust
use gestura_core::mcp::{McpServer, ServerCapabilities};

// Create MCP server
let server = McpServer::new(config)?;

// Register tools
server.register_tool("file_read", file_read_handler);
server.register_tool("shell_exec", shell_exec_handler);

// Start server
server.serve().await?;
```

### Built-in MCP Tools

| Tool | Description | Permission |
|------|-------------|------------|
| `file_read` | Read file contents | ReadOnly |
| `file_write` | Write to file | WriteLocal |
| `file_edit` | Edit file with diff | WriteLocal |
| `shell_exec` | Execute shell command | Execute |
| `git_status` | Get git status | ReadOnly |
| `web_fetch` | Fetch URL content | Network |

### MCP CLI Commands

```bash
# Start MCP server
gestura mcp serve

# List registered tools
gestura mcp tools

# Call a tool
gestura mcp call file_read --path ./README.md

# Show capabilities
gestura mcp capabilities
```

## Testing

### Running Tests

```bash
# Run all workspace tests
cargo test --workspace --all-features

# Run specific crate tests
cargo test -p gestura-core
cargo test -p gestura-cli
cargo test -p gestura-gui

# Run with output
cargo test --workspace -- --nocapture

# Run specific test
cargo test test_session_persistence
```

### Writing Tests

```rust
// crates/gestura-core/src/chat_sessions/mod.rs
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_session_persistence() {
        let store = SessionStore::new_temp().unwrap();

        let session = ChatSession::new("test-session");
        store.save(&session).await.unwrap();

        let loaded = store.load("test-session").await.unwrap();
        assert_eq!(loaded.id, session.id);
    }

    #[tokio::test]
    async fn test_message_append() {
        let mut session = ChatSession::new("test");

        session.add_message(Message {
            role: Role::User,
            content: "Hello".to_string(),
            timestamp: Utc::now(),
        });

        assert_eq!(session.messages.len(), 1);
    }
}
```

### Integration Tests

```rust
// crates/gestura-gui/tests/integration_tests.rs
#[tokio::test]
async fn test_pipeline_integration() {
    let config = Config::default();
    let pipeline = Pipeline::new(config).await.unwrap();

    let request = AgentRequest {
        content: "Hello".to_string(),
        source: RequestSource::Cli,
        session_id: None,
    };

    let response = pipeline.process(request).await.unwrap();
    assert!(!response.content.is_empty());
}
```

## Deployment

### Building for Production

```bash
# Quality gates (required)
cargo fmt
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features

# Release build
cargo build --workspace --release

# GUI packaging
cargo tauri build
```

### Build Outputs

| Platform | Output | Location |
|----------|--------|----------|
| macOS | .dmg, .app | `target/release/bundle/dmg/` |
| Windows | .msi, .exe | `target/release/bundle/msi/` |
| Linux | .deb, .AppImage | `target/release/bundle/deb/` |

### CI/CD Pipeline

```yaml
# .github/workflows/build.yml
name: Build and Test

on: [push, pull_request]

jobs:
  quality-gates:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo fmt -- --check
      - run: cargo clippy --workspace --all-targets --all-features -- -D warnings
      - run: cargo test --workspace --all-features

  build:
    needs: quality-gates
    strategy:
      matrix:
        os: [ubuntu-latest, windows-latest, macos-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo build --workspace --release
      - uses: tauri-apps/tauri-action@v0
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
```

---

## Resources

### Documentation
- **Architecture**: See `docs/ARCHITECTURE.md`
- **API Reference**: See `docs/API.md`
- **Code Organization**: See `docs/CODE_ORGANIZATION.md`

### Community
- **GitHub**: https://github.com/gestura-ai/gestura-app
- **Issues**: https://github.com/gestura-ai/gestura-app/issues
- **Discussions**: https://github.com/gestura-ai/gestura-app/discussions

---

*Happy coding! Build with the Core-First architecture pattern.*
