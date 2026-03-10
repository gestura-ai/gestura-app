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

### Workspace Navigation

Treat the workspace as three layers:

- `gestura-core` — stable public facade and cross-domain integration points
- `gestura-core-*` — focused domain crates that own most business logic
- `gestura-cli` / `gestura-gui` — thin presentation layers and platform wiring

For the canonical crate/module map, prefer generated Rustdoc:

```bash
cargo doc --workspace --no-deps
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
cargo run -p gestura-cli -- agent
```

## Core-First Architecture

For the canonical crate and module reference, prefer generated Rustdoc via:

```bash
cargo doc --workspace --no-deps
```

Use this guide for contributor workflow and operational development practices;
use crate/module Rustdoc for the source-of-truth architecture and API surface.

### Design Principles

1. **Single Source of Truth**: Business logic lives in `gestura-core` and the owning `gestura-core-*` domain crates
2. **Thin Presentation Layers**: CLI and GUI delegate to core
3. **Re-export Pattern**: `gestura-core` exposes stable public paths over domain crates
4. **Feature Gates**: Optional functionality via Cargo features

### Ownership Rule of Thumb

- If a feature belongs to one domain, put it in the owning `gestura-core-*` crate.
- If it defines a stable public entry point or cross-domain orchestration, wire it through `gestura-core`.
- If it is mostly UI, transport, or platform integration, keep it in `gestura-cli` or `gestura-gui`.

### Re-export Pattern

The facade and presentation layers should stay thin over the owning core domain:

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

### Step 1: Implement in the Owning Domain Crate

Add business logic to the relevant `gestura-core-*` crate first:

```rust
// crates/gestura-core-new-feature/src/lib.rs
pub mod types;
mod implementation;

pub use types::*;
pub use implementation::*;
```

```rust
// crates/gestura-core-new-feature/src/types.rs
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

### Step 2: Re-export Through `gestura-core`

```rust
// crates/gestura-core/src/lib.rs
pub mod new_feature {
    pub use gestura_core_new_feature::*;
}
```

### Step 3: Add Presentation-Layer Wiring Only If Needed

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

The scripting domain lives in `gestura-core-scripting` and is surfaced through
`gestura_core::scripting::*` in generated Rustdoc.

Use this guide for workflow-level expectations; use generated docs for the exact
API surface.

### Script Types

| Language | Engine | Features |
|----------|--------|----------|
| Lua | mlua | Lightweight, fast startup |
| Python | PyO3 | ML libraries, data processing |
| JavaScript | deno_core | Async, web APIs |

### Creating a Script

```rust
use gestura_core::scripting::{ScriptContext, ScriptPermission, ScriptingEngine};

// Load and execute a script
let engine = ScriptingEngine::new(script_directory);
engine.initialize().await?;
let script_id = engine.load_script(&script_path).await?;

let context = ScriptContext {
    script_id: script_id.clone(),
    user_id: "user-1".into(),
    session_id: "session-1".into(),
    variables: std::collections::HashMap::new(),
    permissions: vec![ScriptPermission::Network("api.example.com".into())],
    execution_timeout: std::time::Duration::from_secs(30),
};

let result = engine.execute_script(&script_id, context).await?;
```

### Script Permissions

Treat the exact scripting API surface as generated-doc material.

For contributor work, the important rules are:

- request only the permissions a script actually needs
- keep execution timeouts explicit
- document triggers and side effects clearly
- prefer generated Rustdoc for exact types, enums, and runtime methods

## MCP Integration

### MCP Server (Core Implementation)

The MCP protocol domain lives in `gestura-core-mcp` and is surfaced through
`gestura_core::mcp::*`.

Use generated Rustdoc for the protocol/API surface and `gestura mcp --help` for
the CLI contract.

Built-in tool implementation details now belong in generated docs for:

- `gestura_core::tools::*`
- `gestura-core-tools`
- `gestura_core::mcp::*`
- `gestura-core-mcp`

### MCP CLI Commands

```bash
gestura mcp list
gestura mcp status
gestura mcp tools
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
// crates/gestura-core/src/agent_sessions/mod.rs
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_session_persistence() {
        let store = SessionStore::new_temp().unwrap();

        let session = AgentSession::new("test-session");
        store.save(&session).await.unwrap();

        let loaded = store.load("test-session").await.unwrap();
        assert_eq!(loaded.id, session.id);
    }

    #[tokio::test]
    async fn test_message_append() {
        let mut session = AgentSession::new("test");

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
- **Generated Rustdoc**: Run `cargo doc --workspace --no-deps`
- **Architecture Transition Guide**: See `docs/ARCHITECTURE.md`
- **API Routing Guide**: See `docs/API.md`
- **Code Organization Map**: See `docs/CODE_ORGANIZATION.md`

### Community
- **GitHub**: https://github.com/gestura-ai/gestura-app
- **Issues**: https://github.com/gestura-ai/gestura-app/issues
- **Discussions**: https://github.com/gestura-ai/gestura-app/discussions

---

*Happy coding! Build with the Core-First architecture pattern.*
