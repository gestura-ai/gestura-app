# AGENT.md - Gestura.app Development Guide

> **For AI Agents**: This document provides context, architecture, and development guidelines for working on gestura-app. Read this first before making changes.

---

## Project Overview

**Gestura.app** is a desktop voice and gesture control application for macOS, Windows, and Linux. It provides seamless AI-powered voice commands and haptic device integration through both a GUI (Tauri desktop app) and CLI (command-line interface).

### Key Characteristics
- **Desktop-first**: Not a mobile app - uses Tauri v2, not React Native
- **Rust-powered**: Core logic in Rust with shared `gestura-core` library
- **Dual interface**: GUI (system tray + windows) and CLI (terminal commands)
- **Local-first**: Supports local Whisper STT and Ollama LLMs for privacy
- **Multi-provider**: OpenAI, Anthropic (Claude), Grok (xAI), Ollama support

---

## Repository Structure

```
gestura-app/
├── Cargo.toml              # Workspace definition
├── crates/
│   ├── gestura-core/       # Shared library (business logic)
│   │   └── src/
│   │       ├── config.rs       # Configuration management
│   │       ├── speech.rs       # Speech processing
│   │       ├── llm_provider.rs # LLM integrations
│   │       ├── mcp.rs          # MCP client
│   │       ├── session.rs      # Session management
│   │       ├── telemetry.rs    # Metrics collection
│   │       ├── gdpr.rs         # GDPR compliance
│   │       ├── tools/          # System tools (file, shell, git, etc.)
│   │       └── streaming.rs    # LLM response streaming
│   │
│   ├── gestura-cli/        # CLI binary
│   │   └── src/
│   │       ├── main.rs         # clap argument parsing
│   │       ├── commands/       # Subcommand implementations
│   │       │   ├── chat.rs, exec.rs, listen.rs, config.rs
│   │       │   ├── model.rs, device.rs, mcp.rs, session.rs
│   │       │   ├── privacy.rs, health.rs, init.rs, agent.rs
│   │       │   └── tools/      # System tools commands
│   │       └── tool_registry.rs
│   │
│   └── gestura-gui/        # Tauri desktop app
│       ├── tauri.conf.json
│       ├── src/                # Rust backend
│       │   ├── main.rs, api.rs, tray.rs
│       │   ├── agents.rs, streaming.rs
│       │   └── ...
│       └── frontend/           # React frontend
│           └── src/
│
├── docs/                   # Documentation
│   ├── SRS-gestura-app.md  # 📌 Primary SRS (v2.3) - Source of Truth
│   ├── ARCHITECTURE.md
│   ├── DEVELOPER_GUIDE.md
│   ├── API.md
│   └── REQUIREMENTS_TRACKING.md
│
├── TODO.md                 # ✅ All items complete
├── CHANGELOG.md            # Version history
├── COMPLETION_SUMMARY.md   # Implementation status
└── Justfile / Makefile     # Build commands
```

---

## Primary Documentation

| Document | Purpose | Priority |
|----------|---------|----------|
| `docs/SRS-gestura-app.md` | **Primary SRS** - Complete requirements spec | 🔴 Read first |
| `docs/REQUIREMENTS_TRACKING.md` | Tracks alignment with bm-agents | 🟡 Reference |
| `docs/ARCHITECTURE.md` | System architecture details | 🟡 Reference |
| `docs/DEVELOPER_GUIDE.md` | Development setup and workflow | 🟢 As needed |
| `CHANGELOG.md` | Version history and changes | 🟢 As needed |

---

## Technology Stack

| Component | Technology | Notes |
|-----------|------------|-------|
| **Desktop Framework** | Tauri v2 | Cross-platform, Rust backend |
| **Core Language** | Rust | All business logic |
| **CLI Framework** | clap v4 | Derive macros, subcommands |
| **TUI Framework** | ratatui | Interactive terminal UI |
| **Frontend** | React + TypeScript | Vite bundler |
| **Speech-to-Text** | whisper-rs, OpenAI Whisper | Local + cloud |
| **LLM Providers** | OpenAI, Anthropic, Grok, Ollama | Multi-provider |
| **Message Queue** | NATS + async-nats | Agent communication |
| **Device Communication** | btleplug | BLE for Haptic Harmony |

---

## Development Commands

```bash
# Build
just build              # Build all crates
just build-cli          # Build CLI only
just build-gui          # Build GUI only
cargo build -p gestura-cli --release

# Test
just test               # Run all tests
cargo test -p gestura-core
cargo test -p gestura-cli

# Run
just run-cli            # Run CLI
just run-gui            # Run GUI (Tauri dev mode)
cargo run -p gestura-cli -- --help

# Format & Lint
cargo fmt
cargo clippy -D warnings
```

---

## Architecture Principles

### 1. Core-First Design
All business logic lives in `gestura-core`. Both GUI and CLI are thin shells that:
- Parse user input (Tauri commands / clap args)
- Call `gestura-core` functions
- Present results (UI / terminal output)

### 2. Shared Configuration
Single `AppConfig` struct used by both interfaces:
```rust
// gestura-core/src/config.rs
pub struct AppConfig {
    pub voice_settings: VoiceSettings,
    pub llm_settings: LlmSettings,
    pub mcp_tools: Vec<McpTool>,
    pub mdh_pointers: HashMap<String, String>,
    // ...
}
```

### 3. Multi-Provider Abstraction
LLM providers implement a common trait:
```rust
pub enum LlmProvider {
    OpenAi, Anthropic, Grok, Ollama, Echo
}
```

### 4. Streaming Responses
All LLM responses stream token-by-token:
```rust
// gestura-core/src/streaming.rs
pub enum StreamChunk {
    Text(String),
    Done,
    Cancelled,
    Error(String),
}
```

---

## CLI Command Reference

```
gestura [OPTIONS] <COMMAND>

Commands:
  chat        Interactive AI chat session
  exec        Execute a single prompt (non-interactive)
  listen      Voice input mode
  config      Configuration management
  model       Model management (Whisper, LLM)
  device      Haptic device management
  mcp         MCP server management
  session     Session management (list, resume, fork)
  agent       Agent interaction
  privacy     GDPR compliance commands
  health      System health and metrics
  completion  Generate shell completions
  init        First-time setup wizard
  tools       System tools (file, shell, git, code, web)

Global Options:
  -c, --config <FILE>  Path to config file
  -v, --verbose        Enable verbose output
  -q, --quiet          Suppress non-essential output
  --no-color           Disable colored output
  --json               Output in JSON format
```

---

## Feature Parity Matrix (GUI vs CLI)

| Feature | GUI | CLI | Notes |
|---------|-----|-----|-------|
| Voice Input | ✅ | ✅ | `gestura listen` |
| AI Chat | ✅ | ✅ | `gestura chat` |
| Single Prompt | ❌ | ✅ | `gestura exec` (CLI-only) |
| Streaming | ✅ | ✅ | Token-by-token |
| Configuration | ✅ | ✅ | Settings panel / `gestura config` |
| Session Resume | ✅ | ✅ | `gestura session resume` |
| Session Fork | ❌ | ✅ | `gestura session fork` (CLI-only) |
| System Tray | ✅ | ❌ | GUI-only |
| Pipe/Redirect | ❌ | ✅ | CLI-only |
| JSON Output | ❌ | ✅ | `--json` flag (CLI-only) |

---

## Current Status

### ✅ Completed (as of January 2026)
- Core voice processing pipeline (GUI + CLI)
- Multi-provider AI integration (OpenAI, Anthropic, Grok, Ollama)
- Local Whisper STT
- Streaming LLM responses with cancellation
- All CLI commands implemented
- System tools (file, shell, git, code, web, permissions)
- Tool registry and capabilities introspection
- Session management (create, list, resume, fork)
- GDPR compliance (export, delete, consent)
- Agent manager and orchestrator
- Message bus (NATS + in-memory fallback)

### 🔄 In Progress
- CLI v1.0 polish for release (Q2 2026 target)

### 📋 Planned
- Haptic Harmony Ring full integration (Q2 2026)
- MCP ecosystem expansion (Q3 2026)

---

## Guidelines for AI Agents

### When Adding Features
1. **Check the SRS first**: `docs/SRS-gestura-app.md` defines all requirements
2. **Add to core first**: Business logic goes in `gestura-core`
3. **Update both interfaces**: If GUI has it, CLI should too (and vice versa)
4. **Write tests**: Core library should have 90%+ coverage

### When Fixing Bugs
1. **Check existing tests**: Understand the expected behavior
2. **Fix in core if possible**: Avoid duplicating fixes in GUI and CLI
3. **Update CHANGELOG.md**: Document the fix

### When Modifying Architecture
1. **Discuss first**: Major changes should be documented in an implementation plan
2. **Maintain shared core**: Don't move logic from core to GUI/CLI
3. **Keep interfaces thin**: GUI and CLI should be presentation layers only

### Code Style
- Run `cargo fmt` before committing
- Zero `clippy` warnings (`-D warnings`)
- Document public APIs with doc comments
- Use `tracing` for logging, not `println!`

---

## Integration with bm-agents

This repository is monitored by the bm-agents framework. See:
- `bm-agents/repositories.yaml` for monitoring configuration
- `bm-agents/docs/SRS_GESTURA_APP.md` for the external SRS (**NOTE: May be outdated**)

The **authoritative SRS** is `docs/SRS-gestura-app.md` in this repository.

---

## Quick Reference

| Need | Resource |
|------|----------|
| Full requirements | `docs/SRS-gestura-app.md` |
| Build commands | `Justfile` |
| CLI help | `gestura --help` |
| Development setup | `docs/DEVELOPER_GUIDE.md` |
| API documentation | `docs/API.md` |
| Troubleshooting | `docs/TROUBLESHOOTING.md` |

---

*Last updated: January 17, 2026*
