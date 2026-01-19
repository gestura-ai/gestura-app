# Changelog

All notable changes to Gestura.app will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

#### Modern TUI Implementation
- **Advanced TUI Architecture**: Complete ratatui-based terminal UI with professional features
  - Tabbed interface (Chat, Tools, Settings, Help) with keyboard/mouse navigation
  - Stateful scrollable message list with visual scroll indicators
  - Real-time streaming response display with cancellation support (Escape key)
  - Enhanced input field with full cursor control and multi-line support (Shift+Enter)
  - Command palette system (`/` prefix) with fuzzy filtering and tab completion
  - Popup and modal system for help screens and confirmations
  - Mouse support for scrolling, tab switching, and message selection
  - Vim-style modal editing (optional) with Normal/Insert/Command modes
  - Syntax highlighting for code blocks using syntect
  - Theme customization (default, dark, light, high-contrast)
  - Search and filter within messages (Ctrl+F)
  - Session management UI for listing, switching, and exporting sessions
  - Responsive layout adapting to terminal size (80x24 minimum)

#### MCP Protocol Alignment (2025-11-25 Specification)
- **Lifecycle Management**: Initialize/ping/shutdown handlers with capability negotiation
- **Prompts Feature**: List and get prompts with voice command templates
- **Notifications System**: Progress tracking, structured logging, cancellation notifications
- **CLI Commands**: `gestura mcp status`, `gestura mcp prompts`, `gestura mcp capabilities`
- **Modular Architecture**: Restructured into types.rs, lifecycle.rs, prompts.rs, notifications.rs, integrator.rs

#### A2A Protocol (Agent-to-Agent)
- **AgentProfile**: Identity and authentication for cross-agent interactions
- **ProfileStore**: Thread-safe storage for agent profiles with token validation
- **Enhanced A2AServer**: Authentication enforcement, profile registration, token validation
- **Enhanced A2AClient**: Profile-aware client with registration and validation methods
- **CLI Commands**: `gestura a2a status`, `gestura a2a profiles`, `gestura a2a discover`, `gestura a2a register`, `gestura a2a token`, `gestura a2a validate`, `gestura a2a agents`, `gestura a2a send`

#### Token Tracking
- **TokenTracker Module**: Real-time tracking of prompt, completion, and total tokens
- **Usage Statistics**: Per-request and session-wide token accounting
- **GUI Integration**: Token display in chat interface with usage breakdown
- **CLI Integration**: Token counts in TUI status bar and streaming responses

#### Infrastructure
- **gestura-cli**: Full-featured CLI binary with all commands (chat, exec, listen, config, model, device, mcp, session, agent, privacy, health, completion, init, tools)
- **gestura-core**: Shared library crate with all business logic (config, error, llm_provider, mcp, gdpr, session_manager, telemetry, audio_capture, speech, tools)
- **Universal macOS binary**: CLI and GUI both build as universal binaries (Intel + Apple Silicon)
- **PKG installer**: macOS installer that places Gestura.app in /Applications and gestura CLI in /usr/local/bin
- **Signed releases**: Full code signing support for .app, .pkg, and CLI binary with notarization
- Comprehensive GitHub release workflow with multi-platform builds
- Automated package manager publishing (Homebrew, Chocolatey, Winget, Snap)
- Professional release script with version synchronization (`scripts/release.sh`)
- Release workflow documentation (`docs/RELEASE_WORKFLOW.md`)
- AppImage creation for universal Linux compatibility
- Professional release notes with comprehensive feature descriptions
- Version synchronization across Cargo.toml, package.json, and tauri.conf.json
- System tray icon generation script (`scripts/generate-tray-icons.sh`)

### Changed
- **BREAKING**: Workspace restructured to Core-First architecture
  - `crates/gestura-core/` - shared library with all business logic
  - `crates/gestura-gui/` - Tauri desktop app (thin shell)
  - `crates/gestura-cli/` - CLI binary (thin shell)
- Frontend moved from root to `crates/gestura-gui/frontend/`
- All Tauri-specific code consolidated in gestura-gui
- Build scripts updated for new workspace structure
- CI/CD workflows updated for workspace builds
- Updated package.json to reflect gestura-app instead of homepage project
- Added version field to tauri.conf.json for proper Tauri versioning
- Synchronized versions across all configuration files (currently 0.2.0)

### Fixed
- Duplicate system tray icons issue resolved through configuration cleanup
- Listening functionality working correctly with proper error handling
- System permissions monitoring and validation implemented
- Configuration persistence and state management improved
- Removed dead/uncompiled Rust modules from gestura-gui

## [0.1.0] - 2025-08-17

### Added
- Initial Tauri v2 application structure
- Basic configuration management with JSON persistence
- Voice processing interface with whisper-rs integration
- OpenAI Whisper HTTP fallback support
- LLM provider abstraction (OpenAI, Anthropic, Grok, Ollama)
- System tray integration with hide-to-tray functionality
- Global hotkey support (Ctrl+Space default)
- NATS client integration with async-nats
- Agent management scaffolding with lifecycle support
- MCP tool configuration and MDH pointer management
- UI preferences system (theme mode and accent colors)
- Cross-platform build scripts (Makefile and Justfile)
- Haptic interface scaffolding for ring integration
- BLE detector trait with mock implementation
- KV store wrapper for NATS JetStream
- Comprehensive documentation and README
- Complete speech-to-text-to-AI workflow implementation
- Professional configuration interface with organized settings
- System permissions monitoring and status display
- Chat interface for AI interactions with voice integration
- Multi-provider AI integration with fallback support

### Planned
- Faster-Whisper integration (voice-faster-whisper feature)
- Embedded NATS server with JetStream
- Agent subprocess spawning with IPC
- BLE pairing and Haptic Harmony ring integration
- MCP client/server with JSON-RPC
- MDH translation with json-ld-rs
- Dual authentication system
- AES encryption and OS keychain integration
- React frontend with theme system
- Gesture mapping and haptic feedback
- OTA update system with rollback
- GDPR compliance features
- Comprehensive testing suite
- CI/CD pipeline

### Technical Debt
- Remove scaffolding dead code warnings
- Implement proper error propagation
- Add structured logging with tracing
- Improve configuration validation
- Add comprehensive unit tests
- Implement integration tests

### Security
- API keys encrypted at rest using system keychain
- System permission validation before accessing microphone/files
- Comprehensive logging of security-relevant events
- Local processing options for privacy protection
- Clear permission requests with detailed explanations
- Local-first architecture with optional cloud integration
- No external data transmission without explicit user consent

## Development Guidelines

### Version Numbering
- Major: Breaking changes to public API or configuration format
- Minor: New features, significant enhancements
- Patch: Bug fixes, minor improvements, documentation updates

### Release Process
1. Update CHANGELOG.md with new version
2. Update version in Cargo.toml and package.json
3. Run full test suite on all platforms
4. Create release tag and GitHub release
5. Build and publish platform-specific binaries
6. Update documentation and website

### Breaking Changes
All breaking changes must be documented with migration guides and deprecated features should be supported for at least one major version before removal.
