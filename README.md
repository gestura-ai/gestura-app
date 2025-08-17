# gestura.app

An always‑ready, local‑first companion app for the Gestura Haptic Harmony ring. Built in Rust with Tauri 2, it integrates voice (local ASR), MCP agents, NATS MQ, and haptic/gesture tooling with a focus on privacy, performance, and extensibility.

## Features (Current)
- Local‑first voice: whisper.cpp via whisper‑rs (feature `voice-local`), with OpenAI Whisper HTTP fallback
- LLM provider abstraction: OpenAI, Anthropic (Claude), Grok (xAI), Ollama (local); choose via config
- Embedded MQ (client): async‑nats with JetStream KV wrappers; event subjects scaffolded
- Agents: lightweight task manager with shutdown + persisted state to KV (scaffold)
- MCP/MDH: config hooks for tools and MDH pointers; local translation stub
- System Tray + Global Hotkey: background ready, hotkey triggers
- UI Preferences: theme mode (system/light/dark) + accent via Tauri commands
- Packaged scripts: Makefile + Justfile for build/run across platforms

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
- Add tools via config; MDH pointers map JSON‑LD datasets
- Agents can call MDH translate to expose MCP URIs
- Dual auth planned: app approval + MCP token per tool

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
MIT - see LICENSE file for details
