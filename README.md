# gestura.app

An always‑ready, local‑first companion app for the Gestura Haptic Harmony ring. Built in Rust, it integrates voice (local ASR), MCP agents, NATS MQ, and haptic/gesture tooling with a focus on privacy, performance, and extensibility.

## Features (Current)

### Voice & LLM
- **Local‑first voice**: whisper.cpp via whisper‑rs (feature `voice-local`), with OpenAI Whisper HTTP fallback
- **LLM provider abstraction**: OpenAI, Anthropic (Claude), Grok (xAI), Ollama (local); choose via config
- **Token tracking**: Real-time tracking of prompt, completion, and total tokens with usage statistics

### CLI (gestura-cli)
- **Modern TUI**: Professional ratatui-based terminal interface
  - Tabbed views: Chat, Tools, Settings, Help
  - Streaming responses with real-time token display
  - Vim-style modal editing (optional)
  - Command palette (`/`) with fuzzy search
  - Syntax highlighting for code blocks
  - Mouse support and responsive layouts
- **Commands**: chat, exec, listen, config, model, device, mcp, a2a, session, agent, privacy, health, completion, init, tools

### MCP (Model Context Protocol)
- **Full 2025-11-25 spec compliance**: lifecycle, prompts, notifications, capabilities
- **MCP Server**: Embedded server with tool registration and execution
- **CLI commands**: `gestura mcp status`, `gestura mcp prompts`, `gestura mcp capabilities`

### A2A (Agent-to-Agent Protocol)
- **Agent discovery**: via Agent Cards with skills and authentication info
- **Profile management**: Identity propagation with bearer token authentication
- **Task communication**: JSON-RPC 2.0 based task create/status/cancel
- **CLI commands**: `gestura a2a status`, `gestura a2a profiles`, `gestura a2a discover`, `gestura a2a token`

### Infrastructure
- **Embedded MQ (client)**: async‑nats with JetStream KV wrappers; event subjects scaffolded
- **Agents**: lightweight task manager with shutdown + persisted state to KV
- **System Tray + Global Hotkey**: background ready, hotkey triggers
- **UI Preferences**: theme mode (system/light/dark) + accent via Tauri commands
- **Cross-platform**: macOS (Intel + Apple Silicon), Windows, Linux builds

## Features (Planned / Roadmap)
- Faster‑Whisper (feature `voice-faster-whisper`) preferred when enabled
- Agents in isolated processes with IPC envelopes; grace shutdown and reinit from KV
- NATS JetStream subjects & dispatcher: events.voice, events.hotkey, events.mcp, agents.*; flush on exit
- BLE/Haptics: pairing/reconnect, battery/CPT, gesture mapping, waveform editor, OTA w/ rollback
- MCP client/server + dual auth (app approval + MCP tokens); MDH JSON‑LD translation with json-ld-rs
- LLM providers: Gemini/Bedrock/Cohere/Mistral; rate‑limit/backoff; model parameters in UI
- Security/Compliance: AES storage (OS keychain), GDPR prompts/exports, audit logs
- Frontend: React UI (chat-lite, settings, BLE/haptic panels); full theme controller

## Supported Platforms
- Desktop: macOS 12+, Windows 10+, Linux (Ubuntu 20+)
- Mobile: iOS/Android (Tauri mobile alpha; constraints apply)

## Build & Run
- Prereqs: Rust stable, Cargo. For `voice-local`, install `cmake`.
- Makefile (POSIX):
  - `make build` / `make test` / `make clean`
  - `make build-voice-local` / `make run-voice-local` (requires cmake)
  - `make test-nats`
- Justfile (cross‑platform convenience):
  - `just build` / `just test` / `just clean`
  - `just build-voice-local` / `just run-voice-local`
  - `just check-nats` / `just package` / `just doctor`

## Configuration
Config is stored at platform config dir (e.g., `~/Library/Application Support/Gestura/config.json`). Defaults are sensible and local‑first.

Key sections:
- `hotkey_listen`: e.g., "Ctrl+Space"
- `grace_period_secs`: agent shutdown grace
- `ui`: `{ theme_mode: "system"|"light"|"dark", accent: "blue" | hex }`
- `voice`: `{ provider: "local"|"openai"|"none", input_path, local_model_path, openai_* }`
- `llm.primary`: "openai"|"anthropic"|"grok"|"ollama"|"echo"
- `llm.anthropic.thinking_budget_tokens`: optional number; enables Claude "extended thinking" (provider-native) when supported by the selected model
- `mcp_tools`: array of `{name, endpoint}`
- `mdh_pointers`: map of dataset aliases to URIs
- `nats_url`: default `nats://127.0.0.1:4222`

Tauri commands:
- Config: `get_config`, `save_config`
- UI: `get_ui_prefs`, `set_ui_prefs`
- MCP/MDH: `list_mcp_tools`, `add_mcp_tool`, `remove_mcp_tool`, `get_mdh_pointers`, `set_mdh_pointer`, `remove_mdh_pointer`
- Voice: `test_voice`, `run_voice_once`
- LLM: `test_llm`

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

We welcome contributions! Please read our contributing guidelines below.

### Development Setup
1. Install Rust stable toolchain
2. Install Node.js 18+ for frontend development
3. Install platform-specific dependencies:
   - **macOS**: Xcode Command Line Tools
   - **Windows**: Visual Studio Build Tools
   - **Linux**: build-essential, cmake, pkg-config

### Code Standards
- **Rust**: Use `cargo fmt` and `cargo clippy` before submitting
- **Frontend**: Use Prettier and ESLint configurations
- **Documentation**: All public APIs must have doc comments
- **Testing**: New features require unit tests, integration tests for complex flows

### Feature Development
1. Create feature branch from `main`
2. Implement feature behind appropriate feature flags
3. Add comprehensive tests
4. Update documentation and CHANGELOG.md
5. Ensure all platforms build successfully
6. Submit PR with detailed description

### Code Organization
- **Backend**: Modular crates in `src-tauri/src/`
- **Frontend**: React components in `src/` (when added)
- **Tests**: Unit tests alongside code, integration tests in `tests/`
- **Documentation**: Inline docs + markdown files

### Pull Request Process
1. Fork the repository
2. Create a feature branch
3. Make your changes with tests
4. Run `just test` and `just build` to verify
5. Update documentation as needed
6. Submit PR with clear description
7. Address review feedback promptly

### Issue Reporting
- Use GitHub issue templates
- Provide minimal reproduction steps
- Include system information and logs
- Tag appropriately (bug, feature, documentation)

### Security
- Report security issues privately to security@gestura.ai
- Do not include sensitive data in public issues
- Follow responsible disclosure practices

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
