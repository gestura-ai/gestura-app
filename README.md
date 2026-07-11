# gestura.app

An always‑ready, local‑first, intent-first AI assistant for desktop workflows. Built in Rust with a **Core-First Architecture**, Gestura.app accepts intent from voice, chat, Haptic Harmony ring gestures, and future inputs, then turns those inputs into unified actions with a focus on privacy, performance, and extensibility.

## Architecture

Gestura uses a **Core-First** architecture organized as a Rust workspace with 27 domain crates:

```
gestura-app/
├── crates/
│   ├── gestura-core/              # Public facade — stable API surface (re-exports domain crates)
│   ├── gestura-core-foundation/   # Shared primitives (AppError, permissions, events, telemetry)
│   ├── gestura-core-tools/        # Built-in tools + tool policy/permissions
│   ├── gestura-core-mcp/          # MCP protocol domain crate (client, server, discovery)
│   ├── gestura-core-llm/          # LLM provider implementations (feature-gated)
│   ├── gestura-core-pipeline/     # Pipeline types and persona
│   ├── gestura-core-sessions/     # Session management (agent sessions, workspaces)
│   ├── gestura-core-config/       # Configuration types, validation, file watching
│   ├── gestura-core-streaming/    # Streaming LLM support with cancellation
│   ├── gestura-core-audio/        # Audio capture, noise cancellation, STT providers
│   ├── gestura-core-security/     # Encryption, keychain, sandboxing, GDPR
│   ├── gestura-core-a2a/          # A2A (Agent-to-Agent) protocol
│   ├── gestura-core-agents/       # Agent lifecycle management & orchestration types
│   ├── gestura-core-tasks/        # Task management and workflow primitives
│   ├── gestura-core-context/      # Smart context management (analysis, caching, resolution)
│   ├── gestura-core-knowledge/    # Knowledge base with built-in expert documents
│   ├── gestura-core-memory-bank/  # Persistent memory bank (conversation history)
│   ├── gestura-core-explorer/     # File system explorer utility
│   ├── gestura-core-hooks/        # Hooks system (event-driven command templates)
│   ├── gestura-core-scripting/    # Scripting engine (Lua, Python, JavaScript sandbox)
│   ├── gestura-core-plugins/      # Plugin system (discovery, lifecycle, sandboxed execution)
│   ├── gestura-core-analytics/    # Usage analytics and ML-based recommendations
│   ├── gestura-core-nats/         # NATS messaging queue
│   ├── gestura-core-ipc/          # Hotkey inter-process communication
│   ├── gestura-core-retry/        # Retry strategies (exponential backoff, jitter)
│   ├── gestura-cli/               # CLI binary (thin presentation layer)
│   └── gestura-gui/               # Tauri desktop app (thin presentation layer)
```

**Design Principles:**
- **Single Source of Truth**: All business logic in `gestura-core` (and its domain crates)
- **Thin Presentation Layers**: CLI and GUI delegate to core
- **Re-export Pattern**: `gestura-core` re-exports domain crates as stable public API paths
- **Feature Gates**: Optional functionality via Cargo features (`voice-local`, `nats`, `security`, etc.)

## Intent-First Architecture

Gestura.app treats user requests as intent rather than as modality-specific commands. Voice, chat, Haptic Harmony ring gestures, and future input adapters all feed a shared normalization path before entering the core loop.

- **Unified intent normalization**: Input adapters translate modality-specific events into a single `Intent` representation with shared context, confidence, and action metadata.
- **One execution model**: Normalized intents flow through the same policy, context, tool, streaming, and observability layers across GUI, CLI, and automation surfaces.
- **Thin modality adapters**: Capture remains specific to voice, chat, or ring gesture sources, while business logic stays centralized in `gestura-core`.
- **Optional advanced primitives**: `gestura-core-tasks` can attach richer coordination only when an intent becomes complex or multi-step, preserving the direct path for straightforward requests.

### Roadmap to 1.0

1. **Intent unification** — Voice, chat, and ring gesture entry points emit the same normalized intent shape and shared execution metadata.
2. **Full ring integration** — Haptic Harmony ring pairing, gesture capture, mapping, and response feedback work as first-class flows across the product.
3. **Advanced primitives in `gestura-core-tasks`** — TaskRegistry, verification loops, and semantic client flows are available behind conditional middleware for complex intents.
4. **Comprehensive end-to-end tests** — End-to-end coverage validates modality intake, intent routing, execution, and recovery behavior across desktop and CLI surfaces.
5. **Final security/community beta review** — Security validation, privacy review, and community beta feedback are completed before production-stability sign-off.

## Documentation Strategy

The long-term direction is to make crate-level and module-level Rustdoc the
canonical architecture and API reference for the project.

### Generated Docs Quick Start

Generated Rustdoc is built in CI and uploaded as a downloadable GitHub Actions
artifact.

The workflow lives at `.github/workflows/rustdoc-pages.yml` and uploads the
generated docs as the `rustdoc-site` artifact.

If you want a single local generated-doc entry point, start with the public facade:

```bash
cargo doc -p gestura-core --no-deps --open
```

If you want the full workspace reference locally:

```bash
cargo doc --workspace --no-deps
```

If you do not use `--open`, the main facade landing page is:

- `target/doc/gestura_core/index.html`

- Primary API and architecture docs should live in `crates/*/src/lib.rs` and
  public module docs.
- Hosted Rustdoc should remain the canonical public API destination for the
  core-library surface.
- `cargo doc --workspace --no-deps` should still produce a useful entry point for the
  full local workspace.
- The `docs/` directory should gradually shrink toward operational content such
  as install, packaging, release, and troubleshooting guides.

Suggested reading path inside generated docs:

1. Start with `gestura-core`
2. From there, jump to the key facade modules:
   - `gestura_core::pipeline`
   - `gestura_core::tools`
   - `gestura_core::config`
   - `gestura_core::llm_provider`
   - `gestura_core::mcp`
   - `gestura_core::a2a`
   - `gestura_core::knowledge`
   - `gestura_core::memory_bank`
   - `gestura_core::agents`
   - `gestura_core::tasks`
3. Follow those links into the owning `gestura-core-*` domain crates when you
   need implementation-domain detail

High-signal generated-doc crate entry points:

- `gestura-core`
- `gestura-core-tools`
- `gestura-core-config`
- `gestura-core-mcp`
- `gestura-core-pipeline`
- `gestura-core-sessions`
- `gestura-core-context`
- `gestura-core-security`
- other `gestura-core-*` crates as their public docs mature

## Core Features

### Multimodal Input
- **Intent-first input layer**: Voice, chat, Haptic Harmony ring gestures, and future input adapters converge on the same core execution path
- **Voice as a first-class modality**: Local-first speech capture with cloud fallback where configured
- **Chat as a first-class modality**: Text conversations flow through the same agent pipeline and tool policies as spoken requests
- **Haptic Harmony ring gestures as a first-class modality**: Gesture-driven intents can trigger the same unified actions and feedback flows as voice and chat

### Voice & LLM
- **Local‑first voice**: Whisper via whisper-rs with OpenAI Whisper HTTP fallback
- **LLM provider abstraction**: OpenAI, Anthropic (Claude), Grok (xAI), Gemini, Ollama (local)
- **Token tracking**: Real-time tracking with usage statistics and budget monitoring
- **Streaming responses**: Real-time token display during generation with cancellation support
- **Extended thinking**: Anthropic Claude thinking budget support (provider-native)
- **Prompt enhancement**: Auto-enhance prompts with configurable styles (concise/detailed/technical)

### Agent Pipeline
- **Agentic loop**: Multi-turn tool-use pipeline with streaming and shared execution for normalized intents
- **Context compaction**: Automatic history trimming and summarization within token limits
- **Checkpoints**: Session state snapshots for safe "rewind" with retention policies
- **Guardrails**: Project-specific instruction files (`.gestura/guardrails`, `AGENTS.md`)
- **Orchestrator**: Subagent coordination with task delegation across agents
- **Knowledge system**: Built-in expert knowledge (Rust, Tauri, MCP, A2A, CLI, Voice)
- **Smart context**: Request analysis, entity extraction, context caching and resolution
- **Conditional advanced primitives**: Optional `gestura-core-tasks` middleware can activate richer task coordination only for complex multi-step intents

### CLI (gestura-cli)
- **Modern TUI**: Professional ratatui-based terminal interface
  - Tabbed views: Agent, Tools, Settings, Help
  - Streaming responses with real-time token display
  - Vim-style modal editing (optional)
  - Syntax highlighting for code blocks
- **Interactive slash UX**:
  - Quick actions like `/help`, `/clear`, `/save`, `/history`, `/summarize`, `/listen`, `/voice`, `/init`
  - Managed root shells like `/config`, `/context`, `/a2a`, `/privacy`, `/agent`, `/workflow`, `/mcp`, `/tasks`, `/hooks`, `/memory`, `/session`, `/knowledge`
  - Root shells open navigable browsers in TUI/basic mode; explicit subcommands still work directly (for example `/config get llm.primary`)
- **Commands**: agent, exec, listen, config, model, device, mcp, a2a, knowledge, context, session, agent-info, tools, privacy, health, completion, init

### Built-in Tools (12)
| Tool | Description | Permission |
|------|-------------|------------|
| `file` | Read/write/list/search/tree files and directories | ReadOnly / WriteLocal |
| `shell` | Run shell commands in controlled environment | Execute |
| `git` | Git status/diff/log/commit/branch/stash/blame/conflicts | ReadOnly / WriteLocal |
| `code` | Code analysis (map, symbols, references, definitions, lint, test, deps, stats) | ReadOnly |
| `web` | Fetch web pages and convert to text | Network |
| `web_search` | Search the web (Local/SerpAPI/DuckDuckGo/Brave) | Network |
| `a2a` | Delegate tasks to remote agents via A2A protocol | Network |
| `permissions` | Check/request OS-level permissions (mic, accessibility) | System |
| `mcp` | Invoke tools from connected MCP servers | Depends on MCP tool |
| `task` | Create/update/list/organize tasks for current session | WriteLocal |
| `screenshot` | Capture screenshots (full screen or region) | System |
| `screen_record` | Record screen video with start/stop controls | System |

### MCP (Model Context Protocol)
- **Full 2025-11-25 spec compliance**: lifecycle, prompts, notifications, capabilities
- **Domain crate**: `gestura-core-mcp` with client, server, discovery, lifecycle, integrator
- **MCP client registry**: Connect to external MCP servers (stdio, HTTP, SSE transports)
- **Tool discovery**: Auto-discover and cache tools from connected servers
- **CLI commands**: `gestura mcp list|add|remove|enable|disable|connect|disconnect|tools|call|status|prompts|capabilities`

### A2A (Agent-to-Agent Protocol)
- **Domain crate**: `gestura-core-a2a` with server, client, types
- **Agent Cards**: Discovery with skills, authentication, and I/O modes
- **Authentication**: Bearer token with expiration and profile propagation
- **Task communication**: JSON-RPC 2.0 for task create/status/cancel
- **CLI commands**: `gestura a2a status|profiles|discover|register|token|validate|agents|send`

### Infrastructure
- **Embedded MQ**: async‑nats with JetStream KV wrappers (`gestura-core-nats`)
- **Agent lifecycle**: Spawning, task delegation, orchestration (`gestura-core-agents`)
- **System Tray + Global Hotkey**: Background ready, hotkey triggers
- **Hooks system**: Safe event-driven command templates (`gestura-core-hooks`)
- **Plugin system**: Discovery, lifecycle, sandboxed execution (`gestura-core-plugins`)
- **Retry strategies**: Exponential backoff with jitter (`gestura-core-retry`)
- **Cross-platform**: macOS (Intel + Apple Silicon universal), Windows, Linux

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

# Canonical Just workflows
just help
just doctor
just validate-quick
just show-version
```

### Build Commands

| Command | Description |
|---------|-------------|
| `cargo fmt` | Format code |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | Lint |
| `cargo test --workspace --all-features` | Run all tests |
| `cargo build --workspace` | Debug build |
| `cargo build --workspace --release` | Release build |
| `cargo tauri dev` | GUI development |
| `cargo tauri build` | GUI release |
| `just help` | Show standardized local workflows |
| `just doctor` | Check build/release readiness |
| `just validate` | Run production validation pipeline |
| `just validate-quick` | Quick validation (format, clippy, test) |
| `just show-version` | Check Cargo/Tauri/frontend version parity |
| `just set-version X.Y.Z` | Update release version across sources |
| `just test-ui` | Run UI tests |

## Configuration
Config is stored at `~/.gestura/config.yaml` (older versions used `config.json`). Defaults are sensible and local-first.

Key sections:
- `hotkey_listen`: e.g., "Ctrl+Space"
- `grace_period_secs`: agent shutdown grace
- `ui`: `{ theme_mode: "system"|"light"|"dark", accent: "blue" | hex }`
- `voice`: `{ provider: "local"|"openai"|"none", input_path, local_model_path, openai_* }`
- `llm.primary`: "openai"|"anthropic"|"gemini"|"grok"|"ollama"|"echo"
- `llm.fallback`: optional fallback provider (used when primary fails)
- `llm.anthropic.thinking_budget_tokens`: optional; enables Claude "extended thinking"
- `prompt_enhancement`: `{ auto_enhance: bool, style: "concise"|"detailed"|"technical", max_length_multiplier_x10: u8 }`
- `mcp_servers`: array of MCP server entries (Claude Code compatible)
- `mdh_pointers`: map of dataset aliases to URIs
- `nats_url`: default `nats://127.0.0.1:4222`
- `permissions`: `{ default_level: "sandbox"|"restricted"|"full", default_enabled_tools: {...} }`
- `pipeline`: `{ max_history_messages, auto_compact_threshold_percent, compaction_strategy, max_context_tokens, log_token_usage, reflection: { enabled, quality_threshold_percent, max_injected, max_retry_attempts, promotion_confidence_percent }, project_guardrails: { enabled, max_chars } }`
- `web_search`: `{ provider: "local"|"serpapi"|"duckduckgo"|"brave", serpapi_key, brave_key }`
- `notifications`: `{ sound_enabled, haptic_enabled, sound_volume }`
- `hooks`: Hook/event-driven command templates configuration

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
  - Transports: stdio, HTTP, SSE
- **MCP Client Registry**: Connect to external MCP servers with tool discovery and caching
- **CLI Commands**:
  - `gestura mcp list` — List configured MCP servers
  - `gestura mcp add` — Add an MCP server (Claude Code compatible)
  - `gestura mcp add-json` — Add MCP server from raw JSON string
  - `gestura mcp get <name>` — Get detailed info for a server
  - `gestura mcp remove <name>` — Remove a server
  - `gestura mcp enable|disable <name>` — Toggle server
  - `gestura mcp status` — Show protocol status and capabilities
  - `gestura mcp connect|disconnect <name>` — Manage server connections
  - `gestura mcp tools [server]` — List tools from connected servers
  - `gestura mcp call <server> <tool> [args]` — Call a tool on a server
- Add tools via config; MDH pointers map JSON‑LD datasets
- Dual auth: app approval + MCP token per tool

## A2A (Agent-to-Agent Protocol)
- **Protocol**: Google's Agent2Agent open protocol (Linux Foundation)
- **Agent Cards**: Discovery with skills, authentication requirements, I/O modes
- **Authentication**: Bearer token with expiration and profile propagation
- **Task Communication**: JSON-RPC 2.0 for task/create, task/status, task/cancel
- **CLI Commands**:
  - `gestura a2a status` — Show protocol status and features
  - `gestura a2a profiles` — List registered agent profiles
  - `gestura a2a discover <url>` — Discover remote agent
  - `gestura a2a register --id <id> --name <name>` — Register new agent profile
  - `gestura a2a token <agent_id>` — Generate auth token
  - `gestura a2a validate <token>` — Validate a token
  - `gestura a2a agents` — List known remote agents
  - `gestura a2a send --url <url> <message>` — Send task to remote agent

## Agents, Orchestrator & NATS
- **AgentManager** spawns agent tasks; events forwarded from NATS subjects
- **Orchestrator**: Subagent coordination with task delegation (`OrchestratorAgentManager`, `OrchestratorObserver` traits)
- **Hooks**: Event-driven command templates triggered by agent lifecycle events (`gestura-core-hooks`)
- **Plugins**: Sandboxed plugin discovery, lifecycle management, and execution (`gestura-core-plugins`)
- Persist agent state via JetStream KV (`agents/<id>`) when available
- Graceful quit via tray Quit; flush NATS and persist state

## BLE & Haptics (Planned)
- Pair, reconnect, battery, CPT status
- Gesture mapping for tap/tilt; waveform editor (clicks/pulses/ramps; 0.5–3.3G pk)
- OTA with rollback
- Dual‑auth gate to haptic commands in MCP

## Security & Compliance
- **Encryption**: AES-128+ encrypted storage (`gestura-core-security`)
- **Keychain**: OS keychain integration for API keys and secrets
- **Sandboxing**: Workspace-scoped tool execution with configurable policies
- **GDPR**: Privacy prompts/exports; privacy‑by‑design
- **Audit**: Structured logging and audit trails

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

1. **New domain logic** → Add to the appropriate `crates/gestura-core-*/` domain crate
2. **Facade/re-exports** → Expose stable API in `crates/gestura-core/src/lib.rs`
3. **CLI commands** → `crates/gestura-cli/` (thin wrapper calling core)
4. **GUI commands** → `crates/gestura-gui/` (re-export core types)

```rust
// Example: GUI module is a thin wrapper
// crates/gestura-gui/src/new_module.rs
pub use gestura_core::new_module::*;
```

### Pull Request Process
1. Create feature branch from `main`
2. Implement in the appropriate domain crate first
3. Add tests alongside implementation
4. Run all quality gates
5. Submit PR with clear description

### Issue Reporting
- Use GitHub issue templates
- Provide minimal reproduction steps
- Include: OS, Rust version, error logs

## License

Gestura is licensed under the **Gestura Prosperity Software License 1.1** (GPSL-1.1), a source-available license that balances open access with sustainable development.

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
