# Changelog

All notable changes to Gestura.app will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Core Ring Abstraction** (`gestura-core-ring`): new crate defining standard hardware abstractions for the ring (`RingBackend`, `Gesture`, `RingStatus`) alongside a fully-featured `SimulatorBackend`. Easily integratable via an optional `ring-integration` feature toggle:
  - Supports BLE GATT communication using `btleplug` exclusively for the `SimulatorBackend` to connect to the Haptic Harmony Simulator app (using UUID `12345678-1234-5678-9abc-123456789abc`).
  - Smoothly parses and handles `Tap`, `DoubleTap`, `Hold`, `Slide`, and `Tilt` BLE packets naturally into our agnostic `Gesture` domains on a dedicated event monitoring routine.
- **Haptic Feedback Library** (`gestura-core-haptics`): separated primitive semantic haptic patterns out of the ring module so different devices can easily share and issue the same generic `HapticPattern`s (e.g. `Confirm`, `Tick`, `Waveform(Vec<u8>)`) independently.
- **Generic Gestures Library** (`gestura-core-gestures`): decoupled the primitive semantic representation of gestural inputs (yielding properties like `gesture_type`, `acceleration`, etc.) out from hardware adapters into its own agnostic domain crate. Ensures `SimulatorBackend` and any future wearable interfaces align to a pure shared input schema without cross-library bleeding.
- **Device Status Library** (`gestura-core-devices`): extracted standard device connection metrics like `battery` levels and `connection_state` into a shared `DeviceStatus` struct, eliminating duplicate status modeling across hardware interfaces.
- **BLE Networking Utility** (`gestura-core-ble`): centralized core bluetooth scanning routines (using `btleplug`) to simplify integration of future UUID-based GATT hardware, completely separating polling/finding connection logic from backend execution.
- **Agentic Loop Ring Streaming**: newly integrated `process_ring_stream` binds to ring streams via `subscribe_to_gestures()`, normalizing raw ring emissions into unified intents on the fly and wiring BOS1921 waveforms backward explicitly via `OrchestratorObserver::on_haptic_feedback()`.

## [0.10.0] - 2026-04-13

### Added

- **Unified intent normalization layer** (`gestura-core-intent`): new crate that converts every input modality — voice transcripts, typed chat, and Haptic Harmony ring gesture events — into a single, modality-agnostic `Intent` struct (`id`, `timestamp`, `modality`, `primary_action`, `confidence`, `context_hints`, `parameters`) before any agentic processing. The pipeline's `maybe_attach_normalized_intent` middleware attaches intent metadata as request hints for downstream routing and telemetry. Feature-gated behind `advanced-primitives`; when the feature is disabled the constant `INTENT_NORMALIZATION_ENABLED` is `false` and the middleware branch constant-folds away with zero runtime cost.
- **Dynamic model capabilities discovery** (`gestura-core-llm`): new `model_discovery` module queries provider APIs at runtime to discover model-specific context limits and output budgets rather than relying on static provider-level defaults. Supported: Gemini (`GET /v1beta/models/{id}` → `inputTokenLimit`), Anthropic (`GET /v1/models/{id}` → `max_input_tokens`), Grok/xAI (`GET /v1/language-models` → per-model context), and Ollama (`POST /api/show` → `model_info.*.context_length`). For providers without a discovery endpoint (OpenAI), limits are learned from `context_length_exceeded` error responses. All discovered limits are stored in `ModelCapabilitiesCache` (thread-safe, application-lifetime) and shared across pipeline instances.
- **Provider-optimized pipeline configuration**: `PipelineConfig::for_model` and `for_model_with_cache` produce pipeline configs sized to the specific model in use. `AgentPipeline::with_provider_optimized_config` and `with_shared_capabilities_cache` select these dynamically, and the voice processing path (`SpeechProcessor`) now uses `with_provider_optimized_config` instead of the unconfigured `AgentPipeline::new`, ensuring voice-triggered requests respect the active model's context window from the first token.
- **Context overflow recovery**: a new `AppError::ContextOverflow`, `StreamChunk::ContextOverflow`, and `ErrorClass::ContextOverflow` variant distinguish context-window exhaustion from transient network errors, stopping blind retries immediately. The pipeline compact-and-retries once on first overflow using the learned model limit; the GUI surfaces an informative "Context too large — compacting history…" status message and the CLI/TUI display a recoverable context-overflow diagnostic instead of a generic failure.

### Fixed

- `extract_primary_action` no longer truncates `Intent.primary_action` on non-sentence dots. A new `find_sentence_boundary` helper treats a `.` as a sentence terminator only when followed by ASCII whitespace or end-of-string, correctly preserving filenames (`foo.rs`, `Cargo.toml`), version numbers (`1.5`, `2.0.1`), URLs (`example.com`), and method calls (`vec.push()`).
- `strip_fillers` is now Unicode-safe and will not panic or produce corrupted output when the input contains multi-byte UTF-8 characters (e.g. `İ`, `Ñ`). A naïve `haystack.to_lowercase().find(needle)` previously returned byte offsets from the lowercased copy that were invalid in the original string; the implementation now scans the original string directly via `char_indices`.
- Context overflow recovery now correctly applies the learned model limit when retrying after a `ContextLengthExceeded` error. Previously, `capabilities_cache.learn_from_error` was invoked but its result was immediately dropped, so the retry still used the stale configured limit and overflowed again; the learned limit is now forwarded to `truncate_prompt_with_budget`.
- `ModelCapabilities` now reports correct input-token budgets for Anthropic, Gemini, and Grok models. A double-subtraction bug in discovery and heuristics caused `max_input_tokens()` to subtract output tokens from an already-output-reduced limit, yielding a budget roughly half the correct size and triggering premature context compaction.
- Aggressive context compaction now correctly handles short conversation histories (1–2 messages). The previous implementation enforced a hard floor of 2 messages to retain, making compaction a silent no-op for the shortest histories; the fix guarantees that at least one message is always removed.

## [0.9.2] - 2026-04-11

### Fixed

- **System tray "Start Listening" locks up the app** — tray menu and icon click handlers are dispatched on the macOS main thread; `toggle_listening_mode` and `is_app_configured` both call `try_get_api_key_from_keychain_sync` which uses `std::thread::spawn().join()` to block while reading the keychain, stalling the entire UI event loop. All three tray event entry points ("listen" menu item, single-click, double-click) now dispatch this work via `tauri::async_runtime::spawn` + `tokio::task::spawn_blocking` so the main thread is never blocked.

## [0.9.2] - 2026-04-12

### Fixed

- Editor text content no longer renders below line numbers on macOS in packaged release builds. CodeMirror's structural layout CSS (`display:flex` on `.cm-scroller`, sticky positioning on `.cm-gutters`) is now anchored in the static Vite CSS bundle so the correct layout is present from the first painted frame, eliminating the race between WKWebView's aggressive frame commits and JavaScript runtime style injection.
- Code folding keyboard shortcuts now function correctly in the text editor. The `codeFolding()` extension was missing alongside `foldGutter()`, causing fold actions wired into the default keymap to silently no-op.

### Changed

- Rustdoc deployment to GitHub Pages now triggers on the `release` event (`types: [published]`) instead of `workflow_run`. The previous trigger resolved `head_branch` from the commit's associated branch rather than the tag name, causing every release deploy to be rejected by environment protection rules with "Branch 'dev' is not allowed to deploy to github-pages".

## [0.9.1] - 2026-04-12

### Fixed

- Windows release packaging now restores signed sidecar CLI binaries correctly via `sync_signed_cli`, preventing unsigned binary substitution after the Tauri bundle step.

### Changed

- Bumped project version to 0.9.1 across `Cargo.toml`, `tauri.conf.json`, and `package.json`.

## [0.9.0] - 2026-04-11

### Changed

- Inline chat shell cards now report successfully finished session-backed commands as `Complete` and automatically collapse after the command finishes, reducing transcript noise while keeping failed runs expanded for inspection.
- Agent task runtime reconciliation now keeps completion and closeout decisions anchored to explicit build/test/mutation evidence instead of weak summary text or partially satisfied execution state.
- `gestura-core`'s pipeline runtime has been decomposed into focused sidecar modules so agent-loop narration, tracked-task bookkeeping, continuation handling, and shared async iteration/finalization helpers no longer live in a single oversized file.
- `tool_dispatch` now keeps execution behavior separated from its large regression suite, making the runtime path easier to review and maintain without changing behavior.
- `gestura-core-tasks` now acts as the shared task-management foundation for persistent task graphs, reusable workflow primitives, and optional advanced-planning middleware inputs across agent and UI flows.

### Fixed

- Follow-up shell requests now pre-bind reusable PTY sessions to the active streaming assistant message, so inline chat surfaces the shell immediately instead of waiting for later lifecycle/output updates.
- Inline chat now re-reconciles shared shell-session state when a new streaming message is created, preventing delayed shell card hydration when a reusable session becomes active before the message finishes materializing.
- Reused inline shell sessions now expand again when a follow-up command starts, and chat preserves fresher local in-flight shell state instead of regressing back to a stale reusable `Idle` snapshot during follow-up shell requests.
- Completed tracked roots now stay sticky during runtime bookkeeping, so agent reconciliation no longer reopens already-finished task trees just because descendant/runtime state is still being recomputed.
- Success closeout no longer terminalizes build/test verification descendants while runtime requirements remain unmet, preventing premature `Completed`/`Cancelled` task flips that later have to be reopened.
- Results-review narration no longer claims the run has crossed into closeout until the runtime snapshot is fully clear of open work, ready tasks, blocked tasks, and missing requirements.
- Requirement-breakdown materialization now reuses existing matching child tasks under the same parent instead of creating duplicate tracked subtasks during repeated planning passes.

### Added

- Regression coverage for follow-up reusable shell requests in chat and for backend PTY reuse event ordering, verifying reused sessions emit `Busy` before `Started` on subsequent commands.
- Additional frontend regression coverage for reusable shell follow-up flows, including completion→reuse re-expansion and protection against stale `Idle` shared state overwriting locally started `Starting`/`Busy` shell activity.
- Regression coverage for sticky completed roots, missing-evidence success closeout, under-scoped verification execution state, and runtime completion narration guards in `gestura-core` / `gestura-core-tasks`.
- Dedicated sidecar test modules for `agent_loop` and `tool_dispatch`, preserving broad pipeline regression coverage while keeping the production runtime modules smaller and more navigable.
- Optional advanced-primitives support in `gestura-core-tasks`, including structured advanced-plan envelopes plus feature-gated semantic-query and verification helpers for complex multi-step task flows.

## [0.8.2] - 2026-04-09

### Changed

- Agent chat and Shell Manager now consume aligned session-backed shell lifecycle state so reusable terminal sessions appear inline as soon as the session is created and continue streaming through the same live transcript.
- Session-backed shell transcript handling now uses shared reducer logic for lifecycle and output hydration, keeping command banners, output ordering, and session reuse metadata synchronized across replay and live streaming.

### Fixed

- Shell sessions no longer begin running before their inline chat card appears; chat now subscribes to session lifecycle events immediately and materializes the shell block at session startup.
- Reusable shell sessions no longer remain marked as failed in chat after a single command error; failed commands now return the session-backed inline terminal to an idle reusable state when the session itself is still healthy.
- Chat no longer renders a duplicate `shell` tool card when the same activity is already represented by an inline shell session block with a link into Shell Manager.

### Added

- Regression coverage for session-lifecycle dispatch, output-first shell hydration, reusable-session failure recovery, and duplicate inline shell/tool-card suppression.

## [0.8.1] - 2026-04-08

### Changed

- Updated the changelog to reflect `0.8.0` as the current `dev` branch release line.
- Inline chat shell cards now render through the same xterm-based terminal surface used by Shell Manager, so redraw-heavy output keeps terminal formatting instead of degrading into a plain ANSI transcript.
- Agent chat now prioritizes shell lifecycle/output delivery and materializes the streaming shell card on the first lifecycle event instead of waiting for later message-state commits.
- Agent chat now paints a neutral streaming placeholder before starting shell-backed streaming work, so the conversation responds immediately without showing a fake `Thought Process` block while the first shell session initializes.

### Fixed

- User chat bubbles now preserve white body text in light mode while keeping user-authored links black for contrast.
- Agent chat scrolling no longer feels sticky when users try to scroll up during active streaming output.
- Shell tool executions now preserve session routing metadata across normal runs, confirmed follow-up execution, and reflection retries so PTY-backed sessions keep their `shell_session_id` and remain linkable in Shell Manager.
- Inline chat shell output now shows the executed command, strips leaked control-sequence artifacts more reliably, and surfaces the Shell Manager link as soon as the shell session starts instead of after the stream finishes.
- First shell requests now register shell stream listeners in parallel, preserve `shell_session_id` on output-first events, and keep the chat timeline synchronized with live shell activity instead of showing the session only after the process is already underway.

### Added

- Shell sessions now surface early stall signals for interactive prompts and error output, providing faster feedback when a command is waiting for user input or has encountered an error.
- Migrated Windows release signing from local PFX certificate to cloud-based SSL.com eSigner, with Python/Java toolchain setup and TOTP-based authentication.

### Planned

- `gestura-core-tasks` is being positioned as the home for optional advanced primitives that activate only for complex multi-step intents.
- Upcoming work focuses on TaskRegistry-backed coordination, bounded verification loops, and semantic client flows that remain general-purpose and reusable across domains.

## [0.8.0]

### Core-First Architecture Migration (Complete)

A comprehensive refactoring to consolidate all business logic in `gestura-core`, with CLI and GUI as thin presentation layers.

#### Phase 1: A2A Consolidation
- Migrated A2A server implementation to `gestura-core/src/a2a/server.rs`
- Created core token management functions
- CLI uses core token functions via re-exports
- **GUI a2a.rs**: 1,092 → 7 lines (99.4% reduction)

#### Phase 2: Agent Session Unification
- New `gestura-core/src/agent_sessions/` module with `types.rs` and `store.rs`
- Unified session types: `AgentSession`, `AgentMessage`, `MessageRole`
- Shared `AgentSessionStore` for file-based persistence
- Both CLI and GUI use identical session management

#### Phase 3: Permissions + Security Policy
- Enhanced `gestura-core/src/tools/permissions.rs` with audit logging
- Created `gestura-core/src/tools/policy.rs` for centralized policy helpers
- GUI permissions module reduced to thin wrapper (~450 lines)

#### Phase 4: MCP Server Migration
- New `gestura-core/src/mcp/server.rs` (744 lines) with full protocol support
- Includes tool calling, lifecycle management, and JSON-RPC handling
- **GUI mcp_server.rs**: 956 → 459 lines (52% reduction)

#### Phase 5: GUI Subsystem Migration
- **Security**: `gestura-core/src/security/` with encryption.rs, storage.rs
- **Sandbox**: `gestura-core/src/sandbox/` for isolated execution
- **Scripting**: `gestura-core/src/scripting/` multi-language engine
- **NATS**: `gestura-core/src/nats_mq/` message queue integration
- **Agents**: `gestura-core/src/agents/` agent orchestration

#### Phase 6: Analytics/Recommendations/Audio
- **Audio**: `gestura-core/src/audio/` noise cancellation with DFT/IDFT
- **Analytics**: `gestura-core/src/analytics/` with privacy modes
- **Recommendations**: `gestura-core/src/recommendations/` ML-based suggestions

#### Code Reduction Summary

| GUI Module | Before | After | Reduction |
|------------|--------|-------|-----------|
| a2a.rs | 1,092 | 7 | 99.4% |
| security.rs | 265 | 18 | 93.2% |
| sandbox.rs | 326 | 7 | 97.9% |
| scripting_engine.rs | 679 | 10 | 98.5% |
| nats_mq.rs | 439 | 13 | 97.0% |
| noise_cancellation.rs | 476 | 10 | 97.9% |
| usage_analytics.rs | 772 | 10 | 98.7% |
| personalized_recommendations.rs | 649 | 10 | 98.5% |

### Added

#### Agent Markdown Improvements
- **GFM-style tables**: Proper table rendering with borders and alignment
- **Task lists**: GitHub-style `- [ ]` and `- [x]` checkbox rendering
- **Better emphasis**: Proper bold, italic, and strikethrough handling
- **Autolinks**: Automatic URL detection and linking
- **Tolerant code fences**: Support for varied fence styles
- **Copy raw markdown button**: One-click copy of original markdown content
- **Restricted-mode tool confirmation**: Pause/resume flow for tool execution

#### Modern TUI Implementation
- **Advanced TUI Architecture**: Complete ratatui-based terminal UI with professional features
  - Tabbed interface (Agent, Tools, Settings, Help) with keyboard/mouse navigation
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
- **GUI Integration**: Token display in agent interface with usage breakdown
- **CLI Integration**: Token counts in TUI status bar and streaming responses

#### Infrastructure
- **gestura-cli**: Full-featured CLI binary with all commands (agent, exec, listen, config, model, device, mcp, session, agent, privacy, health, completion, init, tools)
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
- Synchronized versions across all configuration files (currently 0.8.0)

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
- Agent interface for AI interactions with voice integration
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

## 1.0 Release Criteria

- **Intent unification** — Voice, chat, and ring gesture entry points resolve into a shared normalized intent contract.
- **Full ring integration** — Haptic Harmony ring pairing, gesture capture, action mapping, and feedback flows operate as first-class product capabilities.
- **Advanced primitives in `gestura-core-tasks`** — TaskRegistry, verification loops, and semantic client enhancements are available for complex multi-step intents.
- **Comprehensive end-to-end tests** — End-to-end validation covers modality intake, intent routing, execution, and recovery behavior.
- **Final security/community beta review** — Security validation, privacy review, and community beta feedback are completed before release sign-off.
