# gestura.app

An always‑ready, local‑first companion app for the Gestura Haptic Harmony ring. Built in Rust with a **Core-First Architecture**, it integrates voice (local ASR), MCP agents, NATS MQ, and haptic/gesture tooling with a focus on privacy, performance, and extensibility.

## Architecture

Gestura uses a **Core-First** architecture organized as a Rust workspace:

```
gestura-app/
├── crates/
│   ├── gestura-core/     # Shared business logic (source of truth)
│   ├── gestura-cli/      # CLI binary (thin presentation layer)
│   └── gestura-gui/      # Tauri desktop app (thin presentation layer)
```

**Design Principles:**
- All business logic lives in `gestura-core`
- CLI and GUI are thin presentation layers that delegate to core
- GUI modules are thin re-export wrappers (7-18 lines each)
- Optional features controlled via Cargo feature flags

## Features (Current)

### Voice & LLM
- **Local‑first voice**: Whisper via whisper-rs with OpenAI Whisper HTTP fallback
- **LLM provider abstraction**: OpenAI, Anthropic (Claude), Grok (xAI), Ollama (local)
- **Token tracking**: Real-time tracking with usage statistics
- **Streaming responses**: Real-time token display during generation

### CLI (gestura-cli)
- **Modern TUI**: Professional ratatui-based terminal interface
  - Tabbed views: Agent, Tools, Settings, Help
  - Streaming responses with real-time token display
  - Vim-style modal editing (optional)
  - Syntax highlighting for code blocks
- **Commands**: agent, exec, listen, config, model, device, mcp, a2a, session, agent, privacy, health, completion, init, tools

### MCP (Model Context Protocol)
- **Full 2025-11-25 spec compliance**: lifecycle, prompts, notifications, capabilities
- **Core implementation**: `gestura-core/src/mcp/` with server, types, lifecycle
- **Built-in tools**: file_read, file_write, file_edit, shell_exec, git_status, web_fetch
- **CLI commands**: `gestura mcp serve`, `gestura mcp tools`, `gestura mcp call`

### A2A (Agent-to-Agent Protocol)
- **Core implementation**: `gestura-core/src/a2a/` with server, client, types
- **Agent Cards**: Discovery with skills and authentication info
- **Task communication**: JSON-RPC 2.0 based task create/status/cancel
- **CLI commands**: `gestura a2a serve`, `gestura a2a discover`, `gestura a2a send`

### Core Modules
| Module | Description |
|--------|-------------|
| `pipeline/` | Agent request/response pipeline |
| `agent_sessions/` | Session persistence |
| `tools/` | Tool registry, permissions, built-in tools |
| `mcp/` | MCP server (2025-11-25) |
| `a2a/` | Agent-to-Agent protocol |
| `security/` | Encryption and secure storage |
| `sandbox/` | Sandboxed execution |
| `scripting/` | Multi-language scripting engine |
| `analytics/` | Usage tracking with privacy modes |
| `recommendations/` | ML-based recommendations |
| `audio/` | Noise cancellation |
| `agents/` | Agent orchestration |
| `nats_mq/` | NATS message queue |

### Infrastructure
- **Embedded MQ (client)**: async‑nats with JetStream KV wrappers
- **Agents**: lightweight task manager with persisted state
- **System Tray + Global Hotkey**: background ready, hotkey triggers
- **Cross-platform**: macOS (Intel + Apple Silicon), Windows, Linux

## Supported Platforms
- Desktop: macOS 12+, Windows 10+, Linux (Ubuntu 20+)
- Mobile: iOS/Android (Tauri mobile alpha; constraints apply)

## Installation

See `docs/INSTALL.md` for:

- **Full** install (GUI + CLI) via PKG/DEB/RPM/MSI
- **CLI-only** install via non-interactive bootstrap scripts
- Automation-friendly examples (curl|bash, PowerShell)

## Build & Run

### Quick Start

```bash
# Clone repository
git clone https://github.com/gestura-ai/gestura-app.git
cd gestura-app

# Quality gates (required before committing)
cargo fmt
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features

# Build all crates
cargo build --workspace

# Run CLI
cargo run -p gestura-cli -- --help
cargo run -p gestura-cli -- agent

# Run GUI (Tauri)
cargo tauri dev
```

### Build Commands

| Command | Description |
|---------|-------------|
| `cargo fmt` | Format code |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | Lint |
| `cargo test --workspace --all-features` | Run tests (462+) |
| `cargo build --workspace` | Debug build |
| `cargo build --workspace --release` | Release build |
| `cargo tauri dev` | GUI development |
| `cargo tauri build` | GUI release |

## Configuration
Config is stored at `~/.gestura/config.yaml` (older versions used `config.json`). Defaults are sensible and local-first.

Key sections:
- `hotkey_listen`: e.g., "Ctrl+Space"
- `grace_period_secs`: agent shutdown grace
- `ui`: `{ theme_mode: "system"|"light"|"dark", accent: "blue" | hex }`
- `voice`: `{ provider: "local"|"openai"|"none", input_path, local_model_path, openai_* }`
- `llm.primary`: "openai"|"anthropic"|"grok"|"ollama"|"echo"
- `llm.anthropic.thinking_budget_tokens`: optional number; enables Claude "extended thinking" (provider-native) when supported by the selected model
- `prompt_enhancement`: `{ auto_enhance: bool, style: "concise"|"detailed"|"technical", max_length_multiplier_x10: u8 }`
- `mcp_tools`: array of `{name, endpoint}`
- `mdh_pointers`: map of dataset aliases to URIs
- `nats_url`: default `nats://127.0.0.1:4222`

Tauri commands:
- Config: `get_config`, `save_config`
- UI: `get_ui_prefs`, `set_ui_prefs`
- MCP/MDH: `list_mcp_tools`, `add_mcp_tool`, `remove_mcp_tool`, `get_mdh_pointers`, `set_mdh_pointer`, `remove_mdh_pointer`
- Voice: `test_voice`, `run_voice_once`
- LLM: `test_llm`
- Prompt Enhancement: `enhance_prompt`

## Voice Engines
- Local whisper (whisper-rs): build with `--features voice-local`; set `voice.local_model_path` and `voice.input_path` (WAV)
- OpenAI Whisper: set `voice.openai_api_key` and `voice.input_path`
- Planned: Faster‑Whisper (`voice-faster-whisper`) with selection preference

## MCP & MDH
- **MCP Server**: Full 2025-11-25 specification compliance
  - Lifecycle: initialize, ping, shutdown with capability negotiation
  - Prompts: List and retrieve prompt templates
  - Notifications: Progress tracking, logging, cancellation
  - Tools: Register and execute MCP tools
- **CLI Commands**:
  - `gestura mcp status` - Show server status and connections
  - `gestura mcp prompts` - List available prompts
  - `gestura mcp capabilities` - Show server capabilities
- Add tools via config; MDH pointers map JSON‑LD datasets
- Dual auth: app approval + MCP token per tool

## A2A (Agent-to-Agent Protocol)
- **Protocol**: Google's Agent2Agent open protocol (Linux Foundation)
- **Agent Cards**: Discovery with skills, authentication requirements, I/O modes
- **Authentication**: Bearer token with expiration and profile propagation
- **Task Communication**: JSON-RPC 2.0 for task/create, task/status, task/cancel
- **CLI Commands**:
  - `gestura a2a status` - Show protocol status and features
  - `gestura a2a profiles` - List registered agent profiles
  - `gestura a2a discover <url>` - Discover remote agent
  - `gestura a2a register` - Register new agent profile
  - `gestura a2a token <agent_id>` - Generate auth token
  - `gestura a2a send` - Send task to remote agent

## Agents & NATS
- AgentManager spawns agent tasks; events forwarded from NATS subjects (scaffold)
- Persist agent state via JetStream KV (`agents/<id>`) when available
- Graceful quit via tray Quit; flush NATS and persist state

## BLE & Haptics (Planned)
- Pair, reconnect, battery, CPT status
- Gesture mapping for tap/tilt; waveform editor (clicks/pulses/ramps; 0.5–3.3G pk)
- OTA with rollback
- Dual‑auth gate to haptic commands in MCP

## Security & Compliance (Planned)
- AES-128+ encrypted storage and OS keychain for secrets
- GDPR prompts/exports; privacy‑by‑design
- Logs and audit trails

## Frontend: Theme & Accessibility
- Honor system color scheme; user can override (system/light/dark) + accent
- WCAG 2.1 contrast; keyboard navigation; no color‑only cues

## Contributing

We welcome contributions! Please follow the Core-First architecture pattern.

### Development Setup
1. Install Rust stable toolchain (2024 edition)
2. Install Tauri CLI: `cargo install tauri-cli`
3. Platform-specific:
   - **macOS**: Xcode Command Line Tools
   - **Windows**: Visual Studio Build Tools
   - **Linux**: build-essential, cmake, pkg-config, libwebkit2gtk-4.1-dev

### Quality Gates (Required)

```bash
cargo fmt
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

### Core-First Development

1. **Business logic** → `crates/gestura-core/`
2. **CLI commands** → `crates/gestura-cli/` (thin wrapper calling core)
3. **GUI commands** → `crates/gestura-gui/` (re-export core types)

```rust
// Example: GUI module is a thin wrapper
// crates/gestura-gui/src/new_module.rs
pub use gestura_core::new_module::*;
```

### Pull Request Process
1. Create feature branch from `main`
2. Implement in `gestura-core` first
3. Add tests alongside implementation
4. Run all quality gates
5. Submit PR with clear description

### Issue Reporting
- Use GitHub issue templates
- Provide minimal reproduction steps
- Include: OS, Rust version, error logs

## License

Gestura is licensed under the **Gestura Prosperity License 1.0** (GPL-1.0), a source-available license that balances open access with sustainable development.

### Free Use (No Cost)
- ✅ Personal, hobby, and educational use
- ✅ Non-profit organizations
- ✅ Small businesses (<$1M annual revenue)
- ✅ Commercial evaluation (90 days)
- ✅ Contributing improvements back to the project

### Commercial Use (>$1M Revenue)
- 3% of revenue attributable to Gestura, OR
- 0.5% of total annual revenue (alternative calculation)
- Enterprise flat-fee licenses available

See [LICENSE](LICENSE) for full terms and [LICENSE-FAQ.md](LICENSE-FAQ.md) for detailed guidance.

**Contact**: licensing@gestura.ai
