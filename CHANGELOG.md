# Changelog

All notable changes to Gestura.app will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

## [0.1.0] - TBD

### Added
- Initial release with core functionality
- Local voice processing
- Basic agent management
- MCP tool configuration
- System integration (tray, hotkeys)
- Cross-platform support

### Security
- Local-first architecture
- No external data transmission without consent
- Encrypted local storage (planned)

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
