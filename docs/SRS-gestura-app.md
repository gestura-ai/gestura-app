# Software Requirements Specification (SRS)
## gestura-app - Desktop Voice & Gesture Control Application
### Gestura LLC Development Project

---

**Document Version:** 2.3
**Date:** January 17, 2026
**Repository:** gestura-app
**Component:** Desktop Application (GUI) & Command-Line Interface (CLI)
**Status:** Production (GUI) / Planned (CLI)
**Business Area:** Voice AI & Human-Computer Interaction

---

## Table of Contents

1. [Introduction](#1-introduction)
2. [System Overview](#2-system-overview)
3. [GUI Functional Requirements](#3-gui-functional-requirements)
4. [CLI Functional Requirements](#4-cli-functional-requirements)
   - 4.11 [System Tools Requirements](#411-system-tools-requirements)
   - 4.12 [System Tools Configuration](#412-system-tools-configuration)
   - 4.13 [Tool Registry & Introspection Requirements](#413-tool-registry--introspection-requirements)
   - 4.14 [Streaming LLM Response Requirements](#414-streaming-llm-response-requirements)
   - 4.15 [Subagent Orchestration Requirements](#415-subagent-orchestration-requirements)
5. [Non-Functional Requirements](#5-non-functional-requirements)
6. [Technical Architecture](#6-technical-architecture)
7. [Integration Requirements](#7-integration-requirements)
8. [Business Alignment](#8-business-alignment)
9. [Quality Assurance](#9-quality-assurance)
10. [Risk Assessment](#10-risk-assessment)
11. [Success Metrics](#11-success-metrics)

---

## 1. Introduction

### 1.1 Purpose
Gestura provides seamless voice and gesture control capabilities for macOS, Windows, and Linux systems through two complementary interfaces:

1. **Gestura Desktop (GUI)**: A Tauri-based desktop application with system tray integration, providing a visual interface for voice commands, AI chat, and device management.

2. **Gestura CLI**: A command-line interface providing feature parity with the GUI, enabling terminal-based workflows, automation, scripting, and headless operation.

Both interfaces share a common Rust core library, ensuring consistent behavior and feature availability across interaction modes.

### 1.2 Scope
This component covers:
- **GUI Mode**: System tray-based voice activation, chat windows, configuration UI, onboarding
- **CLI Mode**: Terminal-based voice commands, interactive chat, configuration management, scripting
- Multi-provider speech-to-text processing (OpenAI Whisper, Local Whisper)
- AI-powered conversation and command processing (OpenAI GPT, Anthropic Claude, Grok, Ollama)
- Haptic device integration and management
- MCP (Model Context Protocol) server integration
- Multi-Device Harmony (MDH) coordination
- System permissions management and security controls
- Configuration management (GUI settings panel / CLI config commands)
- Session management (resume, fork, history)
- GDPR compliance (data export, deletion, consent management)

This component does NOT cover:
- Cloud infrastructure or backend services
- Mobile applications
- Web-based interfaces
- Hardware manufacturing

### 1.3 Business Context
Gestura serves as the flagship application demonstrating Gestura's core value proposition: making human-computer interaction more natural and intuitive through voice and gesture controls. The addition of a CLI interface expands the target audience to include:
- Developers who prefer terminal-based workflows
- DevOps engineers requiring automation and scripting
- Power users seeking keyboard-driven interaction
- Headless server environments
- CI/CD pipeline integration

### 1.4 Stakeholders
- **Primary Users**: Desktop users seeking voice and gesture control capabilities
- **Secondary Users**: Developers and power users preferring CLI workflows
- **Tertiary Users**: Developers integrating with Gestura's MCP ecosystem
- **Business Owners**: Gestura LLC executive team and product management
- **Technical Owners**: Gestura development team and system architects

### 1.5 Reference Implementations
The CLI design is informed by industry-leading AI coding assistants:
- **OpenAI Codex CLI** (Rust): Subcommand architecture, session management (resume/fork), sandbox execution
- **Aider** (Python): Config file support, model switching, git integration, voice input
- **Claude Code** (Node.js): Plugin architecture, hooks, specialized agents

---

## 2. System Overview

### 2.1 Component Role
Gestura serves as the primary desktop client in the Gestura ecosystem, providing:
- **GUI Mode**: Visual interface for voice command activation, chat, and configuration
- **CLI Mode**: Terminal-based interface for scripting, automation, and headless operation
- Integration hub for multiple AI providers and speech services
- Device management for haptic feedback devices
- Configuration and system management interface
- Local processing capabilities with cloud service integration

### 2.2 Key Features

#### 2.2.1 Shared Features (GUI & CLI)
| Feature | Description |
|---------|-------------|
| Voice Processing Pipeline | Complete speech-to-text-to-AI workflow with multiple provider support |
| Multi-Provider AI Integration | OpenAI, Anthropic, Grok, Ollama with automatic fallback |
| Local Whisper STT | Privacy-focused local speech-to-text processing |
| MCP Server Integration | Extensible architecture through Model Context Protocol |
| MDH Coordination | Multi-Device Harmony for device synchronization |
| Session Management | Create, resume, fork, and list conversation sessions |
| Configuration Management | Provider settings, API keys, model selection |
| GDPR Compliance | Data export, deletion, consent management |
| Telemetry & Metrics | System health monitoring and usage analytics |

#### 2.2.2 GUI-Specific Features
| Feature | Description |
|---------|-------------|
| System Tray Integration | Unobtrusive presence with comprehensive menu system |
| Chat Windows | Visual conversation interface with markdown rendering |
| Configuration UI | Professional settings panel with organized sections |
| Onboarding Flow | Guided first-time setup with permission requests |
| Device Management UI | Visual interface for haptic device pairing |
| Light/Dark Mode Icons | Automatic system appearance detection |

#### 2.2.3 CLI-Specific Features
| Feature | Description |
|---------|-------------|
| Interactive Mode | TUI-based chat interface with real-time streaming |
| Non-Interactive Mode | Single-command execution for scripting |
| Config File Support | YAML/TOML configuration with environment variable overrides |
| Shell Completions | Bash, Zsh, Fish, PowerShell completion scripts |
| Pipe/Redirect Support | Unix-style input/output for automation |
| Session Resume/Fork | Continue or branch from previous conversations |

### 2.3 Technology Stack
```yaml
Primary Technologies:
  - Tauri v2: Cross-platform desktop application framework (GUI)
  - clap: Command-line argument parsing (CLI)
  - Rust: Backend logic and system integration (shared core)
  - HTML/CSS/JavaScript: Frontend user interface (GUI)
  - ratatui: Terminal UI framework (CLI interactive mode)

Core Dependencies:
  - tauri: ^2.1.0 (Desktop application framework)
  - clap: ^4.0 (CLI argument parsing with derive macros)
  - tokio: ^1.0 (Async runtime)
  - serde: ^1.0 (Serialization)
  - tracing: ^0.1 (Logging and observability)
  - whisper-rs: Local Whisper STT bindings
  - reqwest: HTTP client for API calls

CLI-Specific Dependencies:
  - ratatui: ^0.26 (Terminal UI)
  - crossterm: ^0.27 (Terminal manipulation)
  - indicatif: ^0.17 (Progress bars)
  - dialoguer: ^0.11 (Interactive prompts)
  - console: ^0.15 (Terminal styling)

Integration Points:
  - OpenAI API: Speech-to-text (Whisper) and language model (GPT)
  - Anthropic API: Claude language model integration
  - Grok API: xAI language model integration
  - Local AI Services: Ollama integration
  - NATS: Message queuing for device communication
  - MCP Servers: Model Context Protocol integration
```

---

## 3. GUI Functional Requirements

### 3.1 Core GUI Functionality

#### FR-GUI-001: System Tray Management
**Requirement**: Application must provide a single, persistent system tray icon with comprehensive menu system
- **Input**: User interactions with system tray icon and menu items
- **Output**: Menu display, application state changes, window creation
- **Behavior**:
  - Single-click opens quick actions menu
  - Menu displays "Start Listening" / "Stop Listening" based on state
  - Light/dark mode icon variants based on system appearance
  - Menu items: Start/Stop Listening, Open Chat, Settings, Quit
- **Priority**: Critical
- **Business Impact**: Primary user interface for application access and control
- **CLI Equivalent**: `gestura listen` command

#### FR-GUI-002: Voice Processing Pipeline
**Requirement**: Complete speech-to-text-to-AI processing workflow with visual feedback
- **Input**: Audio input from system microphone
- **Output**: Transcribed text, AI responses, visual state indicators
- **Behavior**:
  - Capture audio when listening mode activated (tray or chat button)
  - Visual recording indicator in chat window
  - Process speech through configured STT provider (OpenAI or Local Whisper)
  - Route text to appropriate AI provider
  - Display AI responses with markdown rendering
  - Emit `listening-state-changed` events for UI synchronization
- **Priority**: Critical
- **Business Impact**: Core value proposition of voice-controlled computing
- **CLI Equivalent**: `gestura listen`, `gestura chat --voice`

#### FR-GUI-003: Chat Window Interface
**Requirement**: Conversational interface for AI interactions with voice integration
- **Input**: Text input, voice transcriptions, AI responses
- **Output**: Chat messages, conversation history, session management
- **Behavior**:
  - Create new chat sessions from voice input or text
  - Display transcribed speech as user messages
  - Show AI responses with markdown formatting
  - Voice recording button with state synchronization to tray
  - Session persistence and restoration
  - Multiple concurrent chat windows
  - **Tool/Capabilities Introspection**: Respond to `/tools`, `/capabilities`, or natural language questions about available tools and system configuration with deterministic, accurate responses (see FR-GUI-010)
- **Priority**: High
- **Business Impact**: User engagement and conversation continuity
- **CLI Equivalent**: `gestura chat` (interactive mode)

#### FR-GUI-004: Configuration Settings Panel
**Requirement**: Professional configuration interface with organized settings management
- **Input**: User configuration changes, system permission requests
- **Output**: Updated application settings, system permission status
- **Sections**:
  - **System Permissions**: Microphone, accessibility status with "Open System Preferences" button
  - **Voice & Audio**: Provider selection (OpenAI/Local), device selection, VAD settings
  - **AI Providers**: OpenAI, Anthropic, Grok, Ollama configuration with API keys
  - **Whisper Models**: Download, status, and selection for local STT
  - **Device Management**: Haptic ring scanning, pairing, status
  - **MCP Integration**: Add/remove MCP tools, server configuration
  - **MDH Settings**: Multi-Device Harmony pointers
  - **Security & Privacy**: GDPR controls, data export/deletion
  - **Advanced Settings**: Debug logging, telemetry opt-out
- **Priority**: High
- **Business Impact**: User experience and system reliability
- **CLI Equivalent**: `gestura config` subcommands

#### FR-GUI-005: Onboarding Flow
**Requirement**: Guided first-time setup experience for new users
- **Input**: First run detection, user permission grants
- **Output**: Configured application ready for use
- **Behavior**:
  - Detect first run via `is_first_run` check
  - Display onboarding window with setup steps
  - Request microphone permission with explanation
  - Guide API key configuration
  - Complete onboarding and close window
- **Priority**: Medium
- **Business Impact**: User activation and retention
- **CLI Equivalent**: `gestura init` (interactive setup wizard)

### 3.2 GUI Integration Features

#### FR-GUI-INT-001: Haptic Device Management UI
**Requirement**: Visual interface for haptic device discovery and management
- **Input**: Device scan requests, pairing commands, haptic feedback triggers
- **Output**: Device list, connection status, feedback confirmation
- **Behavior**:
  - Scan for available rings via `scan_for_rings`
  - Display device status and battery level
  - Pair/unpair devices with visual feedback
  - Test haptic feedback patterns
  - Start/stop gesture monitoring
- **Priority**: Medium
- **Business Impact**: Differentiation through haptic feedback integration
- **CLI Equivalent**: `gestura device` subcommands

#### FR-GUI-INT-002: MCP Server Management UI
**Requirement**: Visual interface for MCP server configuration
- **Input**: MCP tool configurations, server URLs
- **Output**: Tool list, connection status
- **Behavior**:
  - List available MCP tools via `list_mcp_tools`
  - Add new MCP tools with configuration
  - Remove existing tools
  - Test tool connectivity
- **Priority**: Medium
- **Business Impact**: Extensibility and ecosystem growth
- **CLI Equivalent**: `gestura mcp` subcommands

#### FR-GUI-INT-003: Tool Registry & Capabilities Introspection
**Requirement**: In-chat introspection of available tools and system capabilities
- **Input**: Natural language questions or slash commands (`/tools`, `/capabilities`)
- **Output**: Deterministic, formatted responses listing tools and configuration
- **Behavior**:
  - Detect tool-related questions via heuristic matching (e.g., "what tools do we have?", "list tools")
  - Detect capabilities questions via heuristic matching (e.g., "what can you do?", "mcp servers", "system status")
  - Return **Tool Overview** (`/tools`): Built-in tools (file, shell, git, code, web, permissions) with summaries
  - Return **Capabilities Overview** (`/capabilities`): Full system status including:
    - Built-in tools
    - Configured MCP servers and tools (from `config.mcp_tools`)
    - MDH data resources (from `config.mdh_pointers`)
    - LLM configuration (provider, model)
    - Voice configuration (provider, device, local model)
    - Device/Simulator settings (developer mode, simulators)
    - System settings (hotkey, grace period)
  - Support tool-specific detail commands (`/tools file`, `/tools shell`, etc.)
  - Bypass LLM entirely for introspection queries (deterministic responses)
  - Emit responses via streaming event system for UI consistency
- **Priority**: High
- **Business Impact**: User discoverability of system capabilities; reduces confusion when LLM lacks tool knowledge
- **CLI Equivalent**: `gestura tools list`, `gestura config show`
- **Implementation**: `gestura_core::tools::registry` module with `render_tools_overview()`, `render_capabilities()`, `looks_like_tools_question()`, `looks_like_capabilities_question()`

---

## 4. CLI Functional Requirements

### 4.1 CLI Command Structure

The CLI follows a subcommand architecture inspired by OpenAI Codex, with the following top-level structure:

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
  completion  Generate shell completions
  init        First-time setup wizard
  version     Display version information
  help        Display help for commands

Global Options:
  -c, --config <FILE>     Path to config file
  -v, --verbose           Enable verbose output
  -q, --quiet             Suppress non-essential output
  --no-color              Disable colored output
  --json                  Output in JSON format
```

### 4.2 Core CLI Commands

#### FR-CLI-001: Interactive Chat (`gestura chat`)
**Requirement**: Interactive TUI-based chat interface
- **Input**: User text input, optional voice input
- **Output**: AI responses with streaming, conversation history
- **Behavior**:
  ```
  gestura chat [OPTIONS]

  Options:
    --model <MODEL>         LLM model to use (e.g., gpt-4, claude-3, llama3)
    --provider <PROVIDER>   AI provider (openai, anthropic, grok, ollama)
    --voice                 Enable voice input mode
    --session <ID>          Resume existing session
    --system <PROMPT>       System prompt for conversation
    --no-stream             Disable response streaming
  ```
- **Priority**: Critical
- **Business Impact**: Primary CLI interaction mode
- **GUI Equivalent**: Chat window

#### FR-CLI-002: Single Execution (`gestura exec`)
**Requirement**: Execute a single prompt and exit (non-interactive)
- **Input**: Prompt text via argument or stdin
- **Output**: AI response to stdout
- **Behavior**:
  ```
  gestura exec [OPTIONS] [PROMPT]

  Options:
    --model <MODEL>         LLM model to use
    --provider <PROVIDER>   AI provider
    --stdin                 Read prompt from stdin
    --file <FILE>           Read prompt from file
    --output <FILE>         Write response to file
    --timeout <SECONDS>     Maximum wait time (default: 120)

  Examples:
    gestura exec "Explain this error: $(cat error.log)"
    echo "Summarize this" | gestura exec --stdin
    gestura exec --file prompt.txt --output response.md
  ```
- **Priority**: Critical
- **Business Impact**: Enables scripting and automation
- **GUI Equivalent**: N/A (CLI-specific)

#### FR-CLI-003: Voice Input (`gestura listen`)
**Requirement**: Voice-to-text input with optional AI processing
- **Input**: Microphone audio
- **Output**: Transcribed text, optional AI response
- **Behavior**:
  ```
  gestura listen [OPTIONS]

  Options:
    --provider <PROVIDER>   STT provider (openai, local)
    --model <MODEL>         Whisper model for local STT
    --device <DEVICE>       Audio input device
    --timeout <SECONDS>     Recording timeout
    --vad                   Enable voice activity detection
    --transcribe-only       Output transcription without AI processing
    --continuous            Continuous listening mode

  Examples:
    gestura listen                          # Record and process with AI
    gestura listen --transcribe-only        # Just transcribe
    gestura listen --continuous --vad       # Continuous with auto-stop
  ```
- **Priority**: Critical
- **Business Impact**: Core voice functionality in CLI
- **GUI Equivalent**: System tray "Start Listening", chat voice button

#### FR-CLI-004: Configuration Management (`gestura config`)
**Requirement**: View and modify application configuration
- **Input**: Configuration keys and values
- **Output**: Current configuration, confirmation of changes
- **Behavior**:
  ```
  gestura config <SUBCOMMAND>

  Subcommands:
    show                    Display current configuration
    get <KEY>               Get specific config value
    set <KEY> <VALUE>       Set config value
    edit                    Open config in $EDITOR
    path                    Show config file path
    reset                   Reset to defaults
    import <FILE>           Import configuration from file
    export <FILE>           Export configuration to file

  Examples:
    gestura config show
    gestura config get llm.provider
    gestura config set llm.provider anthropic
    gestura config set llm.openai.api_key $OPENAI_API_KEY
  ```
- **Priority**: High
- **Business Impact**: Essential for CLI usability
- **GUI Equivalent**: Settings panel

### 4.3 Model Management Commands

#### FR-CLI-005: Whisper Model Management (`gestura model whisper`)
**Requirement**: Download and manage local Whisper models
- **Input**: Model selection, download commands
- **Output**: Model status, download progress
- **Behavior**:
  ```
  gestura model whisper <SUBCOMMAND>

  Subcommands:
    list                    List available models with status
    download <MODEL>        Download a model (tiny, base, small, medium, large)
    status <MODEL>          Check model download status
    remove <MODEL>          Remove downloaded model
    validate <MODEL>        Validate model integrity

  Examples:
    gestura model whisper list
    gestura model whisper download base
    gestura model whisper status base
  ```
- **Priority**: High
- **Business Impact**: Enables local STT setup
- **GUI Equivalent**: Whisper Models section in settings

#### FR-CLI-006: LLM Provider Testing (`gestura model test`)
**Requirement**: Test LLM provider connectivity and configuration
- **Input**: Provider selection, test parameters
- **Output**: Connection status, response time, errors
- **Behavior**:
  ```
  gestura model test [OPTIONS]

  Options:
    --provider <PROVIDER>   Provider to test (openai, anthropic, grok, ollama)
    --model <MODEL>         Specific model to test
    --all                   Test all configured providers

  Examples:
    gestura model test --provider openai
    gestura model test --all
  ```
- **Priority**: Medium
- **Business Impact**: Troubleshooting and validation
- **GUI Equivalent**: "Test Connection" buttons in settings

### 4.4 Device Management Commands

#### FR-CLI-007: Haptic Device Management (`gestura device`)
**Requirement**: Manage haptic devices from CLI
- **Input**: Device commands, pairing requests
- **Output**: Device list, status, feedback confirmation
- **Behavior**:
  ```
  gestura device <SUBCOMMAND>

  Subcommands:
    scan                    Scan for available devices
    list                    List paired devices
    pair <DEVICE_ID>        Pair with a device
    unpair <DEVICE_ID>      Unpair a device
    status [DEVICE_ID]      Show device status
    haptic <PATTERN>        Send haptic feedback
    gesture start           Start gesture monitoring
    gesture stop            Stop gesture monitoring

  Examples:
    gestura device scan
    gestura device pair ring-001
    gestura device haptic pulse
  ```
- **Priority**: Medium
- **Business Impact**: Full device control from CLI
- **GUI Equivalent**: Device Management section in settings

### 4.5 MCP Integration Commands

#### FR-CLI-008: MCP Server Management (`gestura mcp`)
**Requirement**: Manage MCP server integrations
- **Input**: MCP tool configurations
- **Output**: Tool list, connection status
- **Behavior**:
  ```
  gestura mcp <SUBCOMMAND>

  Subcommands:
    list                    List configured MCP tools
    add <NAME> <CONFIG>     Add MCP tool
    remove <NAME>           Remove MCP tool
    test <NAME>             Test MCP tool connectivity

  Examples:
    gestura mcp list
    gestura mcp add filesystem '{"path": "/home/user"}'
    gestura mcp test filesystem
  ```
- **Priority**: Medium
- **Business Impact**: Extensibility from CLI
- **GUI Equivalent**: MCP Integration section in settings

### 4.6 Session Management Commands

#### FR-CLI-009: Session Management (`gestura session`)
**Requirement**: Manage conversation sessions (inspired by Codex resume/fork)
- **Input**: Session IDs, filter criteria
- **Output**: Session list, session content
- **Behavior**:
  ```
  gestura session <SUBCOMMAND>

  Subcommands:
    list                    List all sessions
    show <ID>               Display session content
    resume <ID>             Resume a session (alias: gestura chat --session <ID>)
    fork <ID>               Fork a session into new conversation
    delete <ID>             Delete a session
    export <ID> <FILE>      Export session to file
    counts                  Show session statistics

  Options for list:
    --limit <N>             Number of sessions to show
    --last                  Show most recent session
    --all                   Show all sessions (not just current directory)

  Examples:
    gestura session list --limit 10
    gestura session resume --last
    gestura session fork abc123
  ```
- **Priority**: High
- **Business Impact**: Conversation continuity
- **GUI Equivalent**: Session restoration in chat

### 4.7 Agent Commands

#### FR-CLI-010: Agent Interaction (`gestura agent`)
**Requirement**: Interact with Gestura agent system
- **Input**: Agent messages, status queries
- **Output**: Agent responses, status information
- **Behavior**:
  ```
  gestura agent <SUBCOMMAND>

  Subcommands:
    send <MESSAGE>          Send message to agent
    status                  Get agent status

  Examples:
    gestura agent status
    gestura agent send "Schedule a meeting for tomorrow"
  ```
- **Priority**: Medium
- **Business Impact**: Agent functionality access
- **GUI Equivalent**: Agent integration in chat

### 4.8 Utility Commands

#### FR-CLI-011: Shell Completions (`gestura completion`)
**Requirement**: Generate shell completion scripts
- **Input**: Shell type
- **Output**: Completion script to stdout
- **Behavior**:
  ```
  gestura completion <SHELL>

  Shells:
    bash                    Bash completions
    zsh                     Zsh completions
    fish                    Fish completions
    powershell              PowerShell completions

  Examples:
    gestura completion bash > ~/.local/share/bash-completion/completions/gestura
    gestura completion zsh > ~/.zfunc/_gestura
  ```
- **Priority**: Medium
- **Business Impact**: CLI usability
- **GUI Equivalent**: N/A (CLI-specific)

#### FR-CLI-012: First-Time Setup (`gestura init`)
**Requirement**: Interactive first-time setup wizard
- **Input**: User responses to prompts
- **Output**: Configured application
- **Behavior**:
  ```
  gestura init [OPTIONS]

  Options:
    --non-interactive       Use defaults without prompting
    --provider <PROVIDER>   Pre-select AI provider

  Steps:
    1. Check/request microphone permission
    2. Select AI provider
    3. Configure API key
    4. Select/download Whisper model (optional)
    5. Test configuration
  ```
- **Priority**: Medium
- **Business Impact**: User onboarding
- **GUI Equivalent**: Onboarding window

### 4.9 GDPR Compliance Commands

#### FR-CLI-013: Data Privacy (`gestura privacy`)
**Requirement**: GDPR compliance commands
- **Input**: Privacy action requests
- **Output**: Data exports, deletion confirmations
- **Behavior**:
  ```
  gestura privacy <SUBCOMMAND>

  Subcommands:
    export <FILE>           Export all user data
    delete                  Delete all user data (requires confirmation)
    consents                List registered consents
    consent <TYPE> <VALUE>  Register consent (true/false)

  Examples:
    gestura privacy export ~/my-data.json
    gestura privacy delete --confirm
    gestura privacy consents
  ```
- **Priority**: High
- **Business Impact**: Regulatory compliance
- **GUI Equivalent**: Security & Privacy section in settings

### 4.10 Telemetry Commands

#### FR-CLI-014: System Health (`gestura health`)
**Requirement**: System health and metrics access
- **Input**: Health/metrics queries
- **Output**: Health status, metrics data
- **Behavior**:
  ```
  gestura health [SUBCOMMAND]

  Subcommands:
    status                  Overall system health
    metrics                 Recent metrics summary
    clear                   Clear metrics data

  Examples:
    gestura health status
    gestura health metrics --json
  ```
- **Priority**: Low
- **Business Impact**: Troubleshooting and monitoring
- **GUI Equivalent**: Telemetry section in settings

### 4.11 Advanced Agent Features

#### FR-CLI-015: Agent-to-Agent (A2A) Protocol (`gestura a2a`)
**Requirement**: Manage inter-agent communication and task delegation.
- **Input**: Agent discovery requests, task messages, token generation.
- **Output**: Agent profiles, task status, authentication tokens.
- **Behavior**:
  ```
  gestura a2a <SUBCOMMAND>

  Subcommands:
    status                  Show A2A protocol status
    profiles                List registered agent profiles
    discover <URL>          Discover a remote agent
    register <ID> <NAME>    Register a new agent profile
    token <ID> <HOURS>      Generate an auth token
    validate <TOKEN>        Validate an auth token
    agents                  List known remote agents
    send <URL> <MSG>        Send a task to a remote agent
  ```
- **Priority**: High
- **Business Impact**: Enables multi-agent collaboration and distributed workflows.
- **GUI Equivalent**: N/A (Backend feature)

#### FR-CLI-016: Knowledge System (`gestura knowledge`)
**Requirement**: Access and manage the agent's knowledge base.
- **Input**: Knowledge queries, category filters.
- **Output**: Knowledge items, search results.
- **Behavior**:
  ```
  gestura knowledge <SUBCOMMAND>

  Subcommands:
    list                    List all knowledge items
    show <ID>               Show details of a knowledge item
    search <QUERY>          Search for knowledge items
    categories              List all categories
    status                  Show knowledge system status
  ```
- **Priority**: Medium
- **Business Impact**: Improves agent expertise and domain-specific assistance.
- **GUI Equivalent**: N/A (Backend feature)

#### FR-CLI-017: Context Management (`gestura context`)
**Requirement**: Smart context analysis and management for optimal LLM performance.
- **Input**: Requests for analysis, cache management commands.
- **Output**: Context analysis results, cache status.
- **Behavior**:
  ```
  gestura context <SUBCOMMAND>

  Subcommands:
    analyze <REQUEST>       Analyze a request to determine needed context
    status                  Show context system status
    categories              List available context categories
    clear                   Clear all context caches
  ```
- **Priority**: Medium
- **Business Impact**: Optimizes token usage and improves response relevance.
- **GUI Equivalent**: N/A (Backend feature)

### 4.12 System Tools Requirements

The following requirements define built-in system tools that enable Gestura to perform tasks on the local system. These tools are inspired by industry-leading AI coding assistants (Aider, Claude Code, OpenAI Codex) and provide the foundation for agentic workflows.

#### FR-TOOLS-001: File Operations (`gestura tools file`)
**Requirement**: Read, write, edit, and search files on the local filesystem
- **Input**: File paths, content, search patterns
- **Output**: File contents, operation confirmations, search results
- **Behavior**:
  ```
  gestura tools file <SUBCOMMAND>

  Subcommands:
    read <PATH>             Read file contents
    write <PATH> <CONTENT>  Write content to file (creates if not exists)
    edit <PATH>             Open file in $EDITOR or apply patch
    search <PATTERN> [DIR]  Search for pattern in files (ripgrep-style)
    list [DIR]              List files in directory
    tree [DIR]              Show directory tree structure
    add <PATH>              Add file to current chat context
    drop <PATH>             Remove file from chat context
    context                 Show files currently in context

  Options:
    --recursive, -r         Recursive operation
    --hidden                Include hidden files
    --gitignore             Respect .gitignore patterns (default: true)
    --max-depth <N>         Maximum directory depth

  Examples:
    gestura tools file read src/main.rs
    gestura tools file search "fn main" --recursive
    gestura tools file add src/lib.rs src/config.rs
    gestura tools file tree --max-depth 3
  ```
- **Priority**: Critical
- **Business Impact**: Core agentic capability - enables AI to understand and modify codebases
- **GUI Equivalent**: File browser integration in chat
- **Security**: Requires explicit user permission; sandboxed to allowed directories

#### FR-TOOLS-002: Shell Command Execution (`gestura tools shell`)
**Requirement**: Execute shell commands with output capture and optional chat integration
- **Input**: Shell commands, execution options
- **Output**: Command output (stdout/stderr), exit codes
- **Behavior**:
  ```
  gestura tools shell <COMMAND> [ARGS...]

  Aliases:
    gestura run <COMMAND>   Shorthand for shell execution
    gestura !<COMMAND>      Inline shell execution (interactive mode)

  Options:
    --capture               Capture output for chat context
    --timeout <SECS>        Command timeout (default: 300)
    --cwd <DIR>             Working directory
    --env <KEY=VALUE>       Set environment variable
    --no-confirm            Skip confirmation for dangerous commands
    --on-fail <ACTION>      Action on non-zero exit: ignore, warn, error, add-to-chat

  Subcommands:
    run <COMMAND>           Execute command
    test <COMMAND>          Run command, add output to chat only on failure
    history                 Show command execution history
    last                    Show last command output

  Examples:
    gestura tools shell cargo build
    gestura run "npm test" --on-fail add-to-chat
    gestura tools shell test "cargo clippy" --capture
  ```
- **Priority**: Critical
- **Business Impact**: Enables build, test, and automation workflows
- **GUI Equivalent**: Terminal integration in chat
- **Security**: Dangerous commands require confirmation; configurable allowlist/blocklist

#### FR-TOOLS-003: Git Integration (`gestura tools git`)
**Requirement**: Perform git operations with AI-assisted commit messages and conflict resolution
- **Input**: Git commands, commit options
- **Output**: Git status, diffs, operation results
- **Behavior**:
  ```
  gestura tools git <SUBCOMMAND>

  Subcommands:
    status                  Show repository status
    diff [PATH]             Show changes (staged and unstaged)
    log [OPTIONS]           Show commit history
    commit [OPTIONS]        Commit changes with AI-generated message
    undo                    Undo last AI-made commit
    branch [NAME]           List or create branches
    checkout <REF>          Switch branches or restore files
    stash [push|pop|list]   Stash management
    blame <PATH>            Show file blame information
    conflicts               List merge conflicts
    resolve <PATH>          AI-assisted conflict resolution

  Options for commit:
    --message, -m <MSG>     Override AI-generated message
    --all, -a               Stage all changes before commit
    --amend                 Amend previous commit
    --no-verify             Skip pre-commit hooks

  Options for log:
    --oneline               Compact format
    --limit, -n <N>         Number of commits to show
    --author <NAME>         Filter by author

  Examples:
    gestura tools git status
    gestura tools git commit -a
    gestura tools git diff src/
    gestura tools git undo
  ```
- **Priority**: High
- **Business Impact**: Seamless version control integration for development workflows
- **GUI Equivalent**: Git status panel in chat
- **Security**: Destructive operations (force push, reset) require explicit confirmation

#### FR-TOOLS-004: Code Analysis & Search (`gestura tools code`)
**Requirement**: Analyze codebases, find symbols, and generate repository maps
- **Input**: Search queries, analysis options
- **Output**: Code locations, symbol information, repository structure
- **Behavior**:
  ```
  gestura tools code <SUBCOMMAND>

  Subcommands:
    map [DIR]               Generate repository map (file structure + key symbols)
    symbols [PATH]          List symbols (functions, classes, etc.) in file/directory
    references <SYMBOL>     Find all references to a symbol
    definition <SYMBOL>     Find symbol definition
    lint [PATH]             Run linter and show issues
    test [PATH]             Run tests and capture results
    deps                    Show project dependencies
    stats                   Show codebase statistics

  Options:
    --language <LANG>       Filter by language
    --format <FMT>          Output format: text, json, markdown
    --include <PATTERN>     Include files matching pattern
    --exclude <PATTERN>     Exclude files matching pattern

  Examples:
    gestura tools code map
    gestura tools code symbols src/lib.rs
    gestura tools code references "AppConfig"
    gestura tools code lint --format json
  ```
- **Priority**: High
- **Business Impact**: Enables intelligent code navigation and understanding
- **GUI Equivalent**: Code explorer integration
- **Dependencies**: tree-sitter for parsing, ripgrep for search

#### FR-TOOLS-005: Web Content Integration (`gestura tools web`)
**Requirement**: Fetch web content and convert to markdown for chat context
- **Input**: URLs, fetch options
- **Output**: Markdown-formatted content
- **Behavior**:
  ```
  gestura tools web <SUBCOMMAND>

  Subcommands:
    fetch <URL>             Fetch URL and convert to markdown
    search <QUERY>          Web search and return results
    screenshot <URL>        Capture webpage screenshot

  Options:
    --selector <CSS>        Extract specific element
    --timeout <SECS>        Request timeout (default: 30)
    --user-agent <UA>       Custom user agent
    --no-images             Strip images from output
    --add-to-context        Add result to chat context

  Examples:
    gestura tools web fetch https://docs.rs/tokio
    gestura tools web search "rust async patterns"
    gestura tools web fetch https://example.com --selector "main"
  ```
- **Priority**: Medium
- **Business Impact**: Enables documentation lookup and research workflows
- **GUI Equivalent**: Link preview in chat
- **Dependencies**: reqwest for HTTP, scraper for HTML parsing

#### FR-TOOLS-006: Tool Permission Management (`gestura tools permissions`)
**Requirement**: Manage permissions and safety controls for system tools
- **Input**: Permission configurations
- **Output**: Permission status, audit logs
- **Behavior**:
  ```
  gestura tools permissions <SUBCOMMAND>

  Subcommands:
    show                    Show current permission settings
    set <TOOL> <LEVEL>      Set permission level for tool
    allow <PATTERN>         Add to allowlist (paths, commands)
    deny <PATTERN>          Add to denylist
    audit                   Show tool usage audit log
    reset                   Reset to default permissions

  Permission Levels:
    ask                     Always ask before execution (default)
    allow                   Allow without confirmation
    deny                    Block execution

  Tool Categories:
    file.read               File read operations
    file.write              File write operations
    shell.safe              Safe shell commands (ls, cat, etc.)
    shell.dangerous         Dangerous commands (rm, sudo, etc.)
    git.read                Git read operations
    git.write               Git write operations (commit, push)
    web.fetch               Web content fetching

  Examples:
    gestura tools permissions show
    gestura tools permissions set shell.safe allow
    gestura tools permissions allow "/home/user/projects/*"
    gestura tools permissions deny "rm -rf"
  ```
- **Priority**: Critical
- **Business Impact**: Security and user trust - essential for safe agentic operation
- **GUI Equivalent**: Permissions panel in settings
- **Security**: Audit logging enabled by default; permissions persist across sessions

### 4.12 System Tools Configuration

#### FR-TOOLS-007: Tools Configuration File
**Requirement**: Configure system tools via TOML configuration
- **Configuration Location**: `~/.config/gestura/tools.toml`
- **Structure**:
  ```toml
  # Gestura System Tools Configuration

  [tools]
  enabled = true                    # Master switch for all tools
  confirm_dangerous = true          # Require confirmation for dangerous ops
  audit_log = true                  # Enable audit logging
  sandbox_mode = false              # Restrict to project directory only

  [tools.file]
  enabled = true
  allowed_paths = ["~", "/tmp"]     # Allowed base paths
  denied_paths = ["/etc", "/usr"]   # Denied paths
  max_file_size_mb = 10             # Max file size to read
  respect_gitignore = true

  [tools.shell]
  enabled = true
  default_timeout_secs = 300
  allowed_commands = ["cargo", "npm", "git", "make"]
  denied_commands = ["sudo", "rm -rf /"]
  capture_output = true

  [tools.git]
  enabled = true
  auto_commit_message = true        # AI-generated commit messages
  confirm_push = true               # Confirm before push
  confirm_destructive = true        # Confirm reset, force-push, etc.

  [tools.code]
  enabled = true
  languages = ["rust", "typescript", "python"]
  lint_on_save = false
  test_on_commit = false

  [tools.web]
  enabled = true
  timeout_secs = 30
  allowed_domains = []              # Empty = all allowed
  denied_domains = ["localhost", "127.0.0.1"]
  ```
- **Priority**: High
- **Business Impact**: Customization and security policy enforcement

### 4.13 Tool Registry & Introspection Requirements

#### FR-TOOLS-008: Tool Registry (`gestura tools list`)
**Requirement**: Query and display available system tools with descriptions
- **Input**: Optional tool name for detailed view
- **Output**: Formatted list of tools or detailed tool information
- **Behavior**:
  ```
  gestura tools list [TOOL_NAME]
  gestura tools                     # Alias for 'gestura tools list'

  Examples:
    gestura tools                   # List all built-in tools
    gestura tools list              # Same as above
    gestura tools list file         # Detailed info for file tool
    gestura tools file              # Shorthand for detail view
  ```
- **Priority**: High
- **Business Impact**: User discoverability of available capabilities
- **GUI Equivalent**: `/tools` command in chat, natural language questions

#### FR-TOOLS-009: Capabilities Introspection (`gestura capabilities`)
**Requirement**: Display comprehensive system status including dynamic configuration
- **Input**: None (reads from current configuration)
- **Output**: Formatted overview of all system capabilities and settings
- **Behavior**:
  ```
  gestura capabilities

  Output includes:
    ## Built-in Tools
    • file - Read/write/list files and directories
    • shell - Run shell commands
    • git - Git operations with AI-assisted commits
    • code - Code analysis and search
    • web - Fetch web content and convert to markdown
    • permissions - Manage tool permissions

    ## MCP Servers & Tools
    • <name> → <endpoint>
    (Lists configured MCP tools from config.mcp_tools)

    ## MDH Data Resources
    • <alias> → <pointer>
    (Lists configured MDH pointers)

    ## LLM Configuration
    • Primary Provider: <provider>
    • Model: <model>

    ## Voice Configuration
    • Provider: <local|openai>
    • Device: <audio_device>
    • Local Model: <whisper_model_path>

    ## Device & Simulator Settings
    • Developer Mode: <enabled|disabled>
    • Simulators: <enabled|disabled>

    ## System
    • Hotkey: <hotkey>
    • Grace Period: <seconds>s
  ```
- **Priority**: High
- **Business Impact**: Full system transparency for troubleshooting and configuration verification
- **GUI Equivalent**: `/capabilities` command in chat, "what can you do?" natural language questions
- **Implementation**: `gestura_core::tools::render_capabilities()` function

#### FR-TOOLS-010: Introspection Heuristics
**Requirement**: Detect user intent for tool/capabilities queries via natural language
- **Input**: User message text
- **Output**: Boolean indicating whether message is a tool/capabilities question
- **Behavior**:
  - **Tool Questions** (triggers tool overview response):
    - `/tools`, `tools`, `list tools`, `what tools`, `available tools`
  - **Capabilities Questions** (triggers full capabilities response):
    - `/capabilities`, `capabilities`, `what can you do`, `have access to`
    - `mcp servers`, `device settings`, `system status`, `current config`
  - Bypass LLM for detected introspection queries
  - Return deterministic, accurate responses
- **Priority**: High
- **Business Impact**: Eliminates LLM hallucination about tool availability; ensures users get accurate information
- **Implementation**: `looks_like_tools_question()`, `looks_like_capabilities_question()` in `gestura_core::tools::registry`

### 4.14 Streaming LLM Response Requirements

#### FR-STREAM-001: Real-Time Response Streaming
**Requirement**: Stream LLM responses token-by-token to the frontend for real-time display
- **Input**: User prompt, LLM configuration
- **Output**: Stream of `StreamChunk` events (Text, Done, Cancelled, Error)
- **Behavior**:
  - Support streaming for all LLM providers: OpenAI, Anthropic Claude, Grok, Ollama
  - Emit `chat-stream-chunk` events containing partial text as tokens arrive
  - Emit `chat-stream-done` event when response is complete
  - Emit `chat-stream-cancelled` event on user cancellation
  - Emit error events with descriptive messages on failure
  - Default 5-minute timeout (`STREAMING_TIMEOUT_SECS = 300`)
- **Priority**: Critical
- **Business Impact**: Responsive user experience; users see responses as they are generated
- **Implementation**: `gestura_core::streaming` module with `StreamChunk` enum and provider-specific functions

#### FR-STREAM-002: Stream Cancellation
**Requirement**: Allow users to cancel an in-progress streaming response
- **Input**: Cancellation request from UI or keyboard interrupt
- **Output**: Stream terminated, `StreamChunk::Cancelled` sent
- **Behavior**:
  - `CancellationToken` passed to streaming functions
  - Check `cancel_token.is_cancelled()` between chunks
  - Graceful termination with proper cleanup
  - Frontend can request cancellation via Tauri command
- **Priority**: High
- **Business Impact**: User control over long-running requests
- **Implementation**: `streaming::CancellationToken` (wraps `Arc<AtomicBool>`)

#### FR-STREAM-003: Provider-Specific Streaming
**Requirement**: Implement streaming for each supported LLM provider
- **Providers**:
  - `stream_openai()` - OpenAI-compatible APIs (GPT-4, etc.)
  - `stream_anthropic()` - Anthropic Claude API with SSE
  - `stream_ollama()` - Local Ollama instance
  - `stream_grok()` - xAI Grok API (OpenAI-compatible)
- **Behavior**:
  - Parse provider-specific SSE/streaming formats
  - Extract content deltas from JSON responses
  - Handle `[DONE]` markers and completion signals
  - Route through `start_streaming()` dispatcher based on `config.llm.primary`
- **Priority**: High
- **Business Impact**: Consistent streaming experience across all providers
- **Implementation**: Individual `stream_*` functions in `gestura_core::streaming`

### 4.15 Subagent Orchestration Requirements

#### FR-AGENT-001: Agent Manager
**Requirement**: Manage lifecycle of spawned subagents
- **Input**: Agent spawn/shutdown requests, agent IDs
- **Output**: Agent status, command channels
- **Behavior**:
  - `spawn_agent(id, name)` - Create and register a new agent
  - `send_event(id, payload)` - Send command/event to running agent
  - `load_state(id)` - Restore agent state from persistence (NATS KV)
  - `shutdown_all(grace_secs)` - Graceful shutdown with timeout
  - Persist agent status to NATS JetStream KV (if enabled)
  - Support both NATS-backed and in-memory message buses
- **Priority**: Medium
- **Business Impact**: Foundation for multi-agent workflows
- **Implementation**: `gestura_gui::agents::AgentManager`, `AgentSpawner` trait

#### FR-AGENT-002: Agent Orchestrator
**Requirement**: Coordinate task delegation to subagents
- **Input**: `DelegatedTask` with prompt, context, required tools, priority
- **Output**: `TaskResult` with success status, output, tool calls, duration
- **Behavior**:
  - `spawn_subagent(id, name)` - Create specialized subagent
  - `delegate_task(task)` - Route task to appropriate agent
  - `cancel_task(task_id)` - Cancel running task
  - `list_subagents()` - Enumerate active agents with status
  - Auto-spawn agents if not already running
  - Execute tasks asynchronously with result channel
- **Structs**:
  ```rust
  pub struct DelegatedTask {
      pub id: String,
      pub agent_id: String,
      pub prompt: String,
      pub context: Option<serde_json::Value>,
      pub required_tools: Vec<String>,
      pub priority: u8,
  }

  pub struct TaskResult {
      pub task_id: String,
      pub agent_id: String,
      pub success: bool,
      pub output: String,
      pub tool_calls: Vec<ToolCallRecord>,
      pub duration_ms: u64,
  }
  ```
- **Priority**: Medium
- **Business Impact**: Enables complex multi-step workflows and parallel task execution
- **Implementation**: `gestura_gui::orchestrator::AgentOrchestrator`

#### FR-AGENT-003: Process Spawner
**Requirement**: Spawn subagents as isolated subprocess with IPC
- **Input**: Agent ID, name
- **Output**: `ProcessAgent` with stdin/stdout handles
- **Behavior**:
  - Spawn subprocess with piped stdin/stdout
  - Send prompts via stdin, receive responses via stdout
  - Load/persist agent state via NATS KV store
  - Support for agent binary or shell wrapper
- **Priority**: Low (future enhancement)
- **Business Impact**: Process isolation for security and resource management
- **Implementation**: `gestura_gui::process_spawner::ProcessSpawner`

#### FR-AGENT-004: Message Bus
**Requirement**: Unified message bus for agent communication
- **Input**: Subject/topic, payload
- **Output**: Published messages, subscriptions
- **Behavior**:
  - `UnifiedBus` enum with NATS or Memory backends
  - Prefer NATS JetStream if available, fallback to in-memory
  - `publish(subject, payload)` - Send message to topic
  - `subscribe(subject, handler)` - Receive messages with callback
  - Support wildcard subscriptions (`events.*`)
- **Priority**: Medium
- **Business Impact**: Decoupled communication between agents and system components
- **Implementation**: `gestura_gui::memory_bus::UnifiedBus`, `gestura_gui::nats_mq`

---

## 5. Non-Functional Requirements

### 5.1 Performance Requirements

#### NFR-PERF-001: Response Time
- **Target**: Voice command processing within 3 seconds end-to-end
- **Measurement**: Time from voice input start to AI response display
- **Acceptance Criteria**: 95% of voice commands processed within target time
- **CLI Specific**: Non-interactive mode (`gestura exec`) should complete within timeout

#### NFR-PERF-002: System Resource Usage
- **GUI Target**: Maximum 200MB RAM usage during idle, 500MB during active processing
- **CLI Target**: Maximum 50MB RAM for non-interactive, 150MB for interactive mode
- **Peak Load**: Handle continuous voice processing for 8+ hours
- **Scalability**: Graceful degradation under resource constraints

#### NFR-PERF-003: Audio Processing Latency
- **Target**: Audio capture latency under 100ms
- **Measurement**: Time from speech to audio buffer availability
- **Acceptance Criteria**: Real-time audio processing without noticeable delay

#### NFR-PERF-004: CLI Startup Time
- **Target**: CLI cold start under 500ms
- **Measurement**: Time from command invocation to first output
- **Acceptance Criteria**: Responsive feel for interactive use

### 5.2 Reliability Requirements

#### NFR-REL-001: Application Availability
- **Target Uptime**: 99.9% availability during user sessions
- **Recovery Time**: Automatic recovery from crashes within 5 seconds
- **Fault Tolerance**: Graceful handling of network failures and API errors

#### NFR-REL-002: Data Persistence
- **Configuration Backup**: Automatic backup of user settings
- **Session Recovery**: Restore application state after unexpected shutdown
- **Error Recovery**: Maintain functionality during partial system failures

#### NFR-REL-003: CLI Exit Codes
- **0**: Success
- **1**: General error
- **2**: Configuration error
- **3**: Network/API error
- **4**: Permission denied
- **130**: Interrupted (Ctrl+C)

### 5.3 Security Requirements

#### NFR-SEC-001: Data Protection
- **Encryption**: All API keys encrypted at rest using system keychain
- **Access Control**: System permission validation before accessing microphone/files
- **Audit Trail**: Comprehensive logging of security-relevant events
- **CLI Specific**: API keys via environment variables or secure config file

#### NFR-SEC-002: Privacy Protection
- **Local Processing**: Option for completely local speech processing
- **Data Minimization**: No unnecessary data collection or transmission
- **User Consent**: Clear permission requests with detailed explanations

### 5.4 Usability Requirements

#### NFR-USE-001: GUI User Interface
- **Design Consistency**: Follow platform-specific design guidelines
- **Accessibility**: Support for screen readers and keyboard navigation
- **Internationalization**: Support for multiple languages and locales

#### NFR-USE-002: CLI User Interface
- **Unix Philosophy**: Composable commands, stdin/stdout support
- **Discoverability**: Comprehensive `--help` for all commands
- **Consistency**: Predictable option naming (`--verbose`, `--quiet`, `--json`)
- **Error Messages**: Clear, actionable error messages with suggestions

#### NFR-USE-003: Installation and Setup
- **Installation Time**: Complete installation within 2 minutes
- **First-Time Setup**: Guided setup process for new users (GUI onboarding, CLI `init`)
- **Configuration Import**: Easy migration from other voice control applications

---

## 6. Technical Architecture

### 6.1 System Architecture

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           Gestura Application                                │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  ┌─────────────────────────────┐    ┌─────────────────────────────┐         │
│  │      GUI (Tauri v2)         │    │      CLI (clap + ratatui)   │         │
│  ├─────────────────────────────┤    ├─────────────────────────────┤         │
│  │  • System Tray              │    │  • Subcommand Parser        │         │
│  │  • Chat Windows             │    │  • Interactive TUI          │         │
│  │  • Settings Panel           │    │  • Non-Interactive Mode     │         │
│  │  • Onboarding Flow          │    │  • Shell Completions        │         │
│  │  • Device Management UI     │    │  • Progress Indicators      │         │
│  └──────────────┬──────────────┘    └──────────────┬──────────────┘         │
│                 │                                   │                        │
│                 └───────────────┬───────────────────┘                        │
│                                 │                                            │
│  ┌──────────────────────────────▼──────────────────────────────────┐        │
│  │                    gestura-core (Rust Library)                   │        │
│  ├──────────────────────────────────────────────────────────────────┤        │
│  │  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐              │        │
│  │  │   Config    │  │   Speech    │  │     LLM     │              │        │
│  │  │  Manager    │  │  Processor  │  │  Provider   │              │        │
│  │  └─────────────┘  └─────────────┘  └─────────────┘              │        │
│  │  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐              │        │
│  │  │   Audio     │  │   Device    │  │    MCP      │              │        │
│  │  │  Capture    │  │  Manager    │  │  Client     │              │        │
│  │  └─────────────┘  └─────────────┘  └─────────────┘              │        │
│  │  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐              │        │
│  │  │  Session    │  │  Telemetry  │  │    GDPR     │              │        │
│  │  │  Manager    │  │  Collector  │  │  Compliance │              │        │
│  │  └─────────────┘  └─────────────┘  └─────────────┘              │        │
│  └──────────────────────────────────────────────────────────────────┘        │
│                                                                              │
├──────────────────────────────────────────────────────────────────────────────┤
│  External Integrations                                                       │
│  ├── OpenAI API (Whisper, GPT)                                              │
│  ├── Anthropic API (Claude)                                                 │
│  ├── Grok API (xAI)                                                         │
│  ├── Ollama (Local LLMs)                                                    │
│  ├── Local Whisper (whisper-rs)                                             │
│  ├── MCP Servers                                                            │
│  └── Haptic Devices (NATS)                                                  │
└──────────────────────────────────────────────────────────────────────────────┘
```

### 6.2 Crate Structure

```
gestura-app/
├── Cargo.toml                    # Workspace definition
├── crates/
│   ├── gestura-core/             # Shared library crate
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── config.rs         # Configuration management
│   │       ├── speech.rs         # Speech processing
│   │       ├── llm_provider.rs   # LLM integrations
│   │       ├── audio_capture.rs  # Audio capture
│   │       ├── device.rs         # Device management
│   │       ├── mcp.rs            # MCP client
│   │       ├── session.rs        # Session management
│   │       ├── telemetry.rs      # Metrics collection
│   │       └── gdpr.rs           # GDPR compliance
│   │
│   ├── gestura-cli/              # CLI binary crate
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── main.rs           # Entry point with clap
│   │       ├── commands/         # Subcommand implementations
│   │       │   ├── mod.rs
│   │       │   ├── chat.rs
│   │       │   ├── exec.rs
│   │       │   ├── listen.rs
│   │       │   ├── config.rs
│   │       │   ├── model.rs
│   │       │   ├── device.rs
│   │       │   ├── mcp.rs
│   │       │   ├── session.rs
│   │       │   └── ...
│   │       └── tui/              # Terminal UI components
│   │           ├── mod.rs
│   │           ├── chat.rs
│   │           └── progress.rs
│   │
│   └── gestura-gui/              # GUI binary crate (current src-tauri)
│       ├── Cargo.toml
│       ├── tauri.conf.json
│       └── src/
│           ├── main.rs           # Tauri entry point
│           ├── api.rs            # Tauri commands
│           ├── tray.rs           # System tray
│           └── ...
│
├── public/                       # Web frontend assets
└── docs/                         # Documentation
### 6.3 Data Model
```rust
// Core Configuration Structure (shared between GUI and CLI)
pub struct AppConfig {
    pub voice_settings: VoiceSettings,
    pub llm_settings: LlmSettings,
    pub mcp_tools: Vec<McpTool>,
    pub mdh_pointers: HashMap<String, String>,
    pub telemetry_enabled: bool,
    pub first_run_completed: bool,
}

pub struct VoiceSettings {
    pub provider: VoiceProvider,  // "openai" | "local"
    pub openai_api_key: Option<String>,
    pub whisper_model: String,    // "tiny" | "base" | "small" | "medium" | "large"
    pub input_device: Option<String>,
    pub vad_enabled: bool,
    pub recording_timeout_secs: u32,
}

pub struct LlmSettings {
    pub provider: LlmProvider,    // "openai" | "anthropic" | "grok" | "ollama"
    pub openai: OpenAiSettings,
    pub anthropic: AnthropicSettings,
    pub grok: GrokSettings,
    pub ollama: OllamaSettings,
}

// Session Management (for resume/fork functionality)
pub struct ChatSession {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub messages: Vec<ChatMessage>,
    pub model: String,
    pub provider: String,
    pub working_directory: Option<PathBuf>,
}
```

### 6.4 CLI Configuration File

The CLI supports configuration via TOML file at `~/.config/gestura/config.toml`:

```toml
# Gestura CLI Configuration

[voice]
provider = "local"           # "openai" | "local"
whisper_model = "base"       # "tiny" | "base" | "small" | "medium" | "large"
vad_enabled = true
recording_timeout_secs = 30

[llm]
provider = "anthropic"       # "openai" | "anthropic" | "grok" | "ollama"
default_model = "claude-3-5-sonnet-20241022"

[llm.openai]
api_key = "${OPENAI_API_KEY}"  # Environment variable expansion
model = "gpt-4o"

[llm.anthropic]
api_key = "${ANTHROPIC_API_KEY}"
model = "claude-3-5-sonnet-20241022"

[llm.grok]
api_key = "${GROK_API_KEY}"
model = "grok-2"

[llm.ollama]
base_url = "http://localhost:11434"
model = "llama3.2"

[output]
color = true
json = false
verbose = false

[telemetry]
enabled = false
```

Environment variables override config file values with prefix `GESTURA_`:
- `GESTURA_LLM_PROVIDER=openai`
- `GESTURA_VOICE_PROVIDER=local`
- `GESTURA_VERBOSE=true`

---

## 7. Integration Requirements

### 7.1 Upstream Dependencies
- **OpenAI API**: Speech-to-text (Whisper) and language model (GPT) services
- **Anthropic API**: Claude language model services
- **Grok API**: xAI language model services
- **Ollama**: Local LLM inference
- **whisper-rs**: Local Whisper STT bindings
- **System APIs**: macOS/Windows/Linux audio, notification, and permission systems

### 7.2 Downstream Consumers
- **MCP Servers**: Extended functionality through Model Context Protocol
- **Haptic Devices**: Haptic Harmony Ring and other gesture devices
- **Shell Scripts**: CLI integration for automation
- **CI/CD Pipelines**: Automated workflows using CLI

### 7.3 Data Flow

**GUI Mode:**
```
User Voice Input → Audio Capture → Speech-to-Text Provider →
Text Processing → AI Provider → Response Generation →
Chat Window Display → Haptic Feedback (optional)
```

**CLI Interactive Mode:**
```
User Voice/Text Input → gestura-core → AI Provider →
Response Streaming → Terminal Display
```

**CLI Non-Interactive Mode:**
```
stdin/args → gestura exec → AI Provider → stdout
```

---

## 8. Business Alignment

### 8.1 Business Objectives Supported
- **Voice AI Leadership**: Demonstrates Gestura's capabilities in voice-controlled computing
- **Platform Ecosystem**: Serves as reference implementation for Gestura's technology stack
- **User Experience Innovation**: Showcases natural human-computer interaction paradigms
- **Market Differentiation**: Unique combination of voice, AI, and haptic feedback
- **Developer Ecosystem**: MCP integration and CLI enable third-party extensions and automation
- **Developer Adoption**: CLI interface attracts power users and enables CI/CD integration

### 8.2 Success Criteria
- **User Adoption**: 10,000+ active users within 6 months of release
- **Feature Utilization**: 80%+ of users actively using voice commands
- **CLI Adoption**: 20%+ of users utilizing CLI for automation
- **Provider Diversity**: Support for 4+ AI providers with seamless switching
- **System Integration**: Native integration with all major desktop platforms
- **Developer Engagement**: 50+ MCP servers integrated within first year

### 8.3 Timeline Alignment
```yaml
Milestones:
  - Name: Core Voice Processing (GUI)
    Target Date: Q3 2025 ✅ COMPLETE
    Completion Criteria: End-to-end voice-to-AI pipeline functional
    Business Impact: Demonstrates core value proposition

  - Name: Multi-Provider Support
    Target Date: Q4 2025 ✅ COMPLETE
    Completion Criteria: 4+ AI providers integrated (OpenAI, Anthropic, Grok, Ollama)
    Business Impact: Reliability and user choice

  - Name: Local Whisper STT
    Target Date: Q1 2026 ✅ COMPLETE
    Completion Criteria: Local speech-to-text without API dependency
    Business Impact: Privacy and offline capability

  - Name: CLI v1.0 Release
    Target Date: Q2 2026
    Completion Criteria: Feature parity with GUI, all core commands implemented
    Business Impact: Developer adoption and automation

  - Name: Haptic Integration
    Target Date: Q2 2026
    Completion Criteria: Haptic Harmony Ring fully integrated
    Business Impact: Unique market differentiation

  - Name: CLI Session Management
    Target Date: Q3 2026
    Completion Criteria: Resume/fork sessions, history management
    Business Impact: Power user workflows

  - Name: MCP Ecosystem
    Target Date: Q3 2026
    Completion Criteria: 20+ MCP servers available
    Business Impact: Platform ecosystem growth
```

---

## 9. Quality Assurance

### 9.1 Testing Strategy
- **Unit Testing**: 90%+ code coverage for gestura-core library
- **Integration Testing**: End-to-end testing of voice processing pipeline
- **CLI Testing**: Automated CLI command testing with expected outputs
- **Performance Testing**: Load testing with continuous voice processing
- **Security Testing**: Penetration testing of API integrations and data handling
- **Usability Testing**: User experience validation across target platforms
- **Compatibility Testing**: Validation across macOS, Windows, and Linux

### 9.2 Quality Gates
```yaml
Code Quality:
  - Test Coverage: 90% minimum for gestura-core
  - Test Coverage: 80% minimum for CLI commands
  - Code Complexity: Maximum cyclomatic complexity of 10
  - Documentation: All public APIs documented with examples
  - Static Analysis: Zero critical security vulnerabilities
  - Clippy: Zero warnings with -D warnings

Security:
  - Vulnerability Scanning: Weekly automated scans (cargo-audit)
  - Dependency Auditing: Monthly security audit of dependencies
  - Penetration Testing: Quarterly security assessment

Performance:
  - Load Testing: Handle 100 concurrent voice sessions
  - Stress Testing: 24-hour continuous operation
  - Memory Testing: No memory leaks during extended use
  - Response Time: 95% of operations within SLA targets
  - CLI Startup: <500ms cold start
```

---

## 10. Risk Assessment

### 10.1 Technical Risks
| Risk | Probability | Impact | Mitigation Strategy |
|------|-------------|--------|-------------------|
| AI Provider API Changes | Medium | High | Multi-provider support with fallback mechanisms |
| Audio Processing Latency | Low | Medium | Optimize audio pipeline and local processing options |
| Cross-Platform Compatibility | Medium | Medium | Comprehensive testing on all target platforms |
| Memory Leaks in Long Sessions | Low | High | Extensive memory testing and monitoring |
| Security Vulnerabilities | Medium | High | Regular security audits and dependency updates |
| CLI/GUI Code Divergence | Medium | Medium | Shared gestura-core library with comprehensive tests |

### 10.2 Business Risks
| Risk | Probability | Impact | Mitigation Strategy |
|------|-------------|--------|-------------------|
| Slow User Adoption | Medium | High | Comprehensive onboarding and user education |
| Competitor Feature Parity | High | Medium | Continuous innovation and unique value propositions |
| AI Provider Cost Increases | Medium | Medium | Local processing options and cost optimization |
| Platform Policy Changes | Low | High | Maintain compliance and alternative distribution |
| CLI Complexity Barrier | Medium | Low | Excellent documentation, shell completions, examples |

### 10.3 Dependencies and Assumptions
- **Assumption 1**: AI provider APIs remain stable and accessible
- **Assumption 2**: Users have reliable internet connectivity for cloud services
- **Assumption 3**: System permissions can be obtained for microphone access
- **Assumption 4**: CLI users have basic terminal proficiency
- **Dependency 1**: Tauri framework continued development and support
- **Dependency 2**: Rust ecosystem stability and security updates
- **Dependency 3**: Platform-specific audio APIs remain accessible
- **Dependency 4**: clap and ratatui crates remain maintained

---

## 11. Success Metrics

### 11.1 Technical Metrics
```yaml
Development Metrics:
  - Code Quality Score: 8.5/10 minimum
  - Test Coverage: 90% for gestura-core, 80% for CLI
  - Bug Density: <1 critical bug per 1000 lines of code
  - Performance Benchmarks: <3s voice processing, <100ms audio latency, <500ms CLI startup

Operational Metrics:
  - Uptime: 99.9% availability during user sessions
  - Response Time: 95% of voice commands within 3 seconds
  - Error Rate: <1% failed voice processing attempts
  - Memory Usage: <500MB GUI peak, <150MB CLI peak
```

### 11.2 Business Metrics
```yaml
Business Impact:
  - Feature Adoption: 80% of users actively using voice commands
  - CLI Adoption: 20% of users utilizing CLI for automation
  - User Satisfaction: 4.5/5 average rating in app stores
  - Business Value: Demonstrate Gestura's technology capabilities
  - ROI Contribution: Platform for future commercial products

User Engagement:
  - Daily Active Users: Track daily usage patterns (GUI and CLI)
  - Session Duration: Average session length and frequency
  - Feature Utilization: Usage statistics for each major feature
  - User Retention: 30-day and 90-day retention rates
  - CLI Script Usage: Track automation and scripting patterns
```

### 11.3 Monitoring and Alerting
```yaml
Monitoring Requirements:
  - Health Checks: Application responsiveness and core functionality
  - Performance Metrics: Response times, memory usage, CPU utilization
  - Business Metrics: Feature usage, user engagement, error rates
  - Security Metrics: Failed authentication attempts, permission denials

Alert Conditions:
  - Critical: Application crashes, security breaches, data loss
  - Warning: Performance degradation, high error rates, resource limits
  - Information: Feature usage patterns, system updates, configuration changes
```

---

## Appendices

### Appendix A: Glossary
- **CLI**: Command-Line Interface - Terminal-based interaction mode
- **GUI**: Graphical User Interface - Visual desktop application mode
- **MCP**: Model Context Protocol - Standard for AI model integration
- **MDH**: Multi-Device Harmony - Gestura's device coordination system
- **STT**: Speech-to-Text - Audio to text conversion technology
- **LLM**: Large Language Model - AI systems for text generation
- **TUI**: Terminal User Interface - Interactive terminal-based UI
- **VAD**: Voice Activity Detection - Automatic speech endpoint detection
- **Haptic Feedback**: Touch-based sensory feedback from devices
- **System Tray**: Desktop notification area for background applications

### Appendix B: References
- [Tauri v2 Documentation](https://v2.tauri.app/)
- [clap Documentation](https://docs.rs/clap/)
- [ratatui Documentation](https://ratatui.rs/)
- [OpenAI API Documentation](https://platform.openai.com/docs)
- [Anthropic Claude API](https://docs.anthropic.com/)
- [Model Context Protocol Specification](https://modelcontextprotocol.io/)
- [OpenAI Codex CLI](https://github.com/openai/codex) - Reference implementation
- [Aider](https://github.com/Aider-AI/aider) - Reference implementation
- [Claude Code](https://github.com/anthropics/claude-code) - Reference implementation

### Appendix C: CLI Command Reference

```
gestura - Voice-first AI assistant

USAGE:
    gestura [OPTIONS] <COMMAND>

COMMANDS:
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
    version     Display version information
    help        Display help for commands

GLOBAL OPTIONS:
    -c, --config <FILE>     Path to config file
    -v, --verbose           Enable verbose output
    -q, --quiet             Suppress non-essential output
    --no-color              Disable colored output
    --json                  Output in JSON format
    -h, --help              Print help
    -V, --version           Print version

ENVIRONMENT VARIABLES:
    GESTURA_LLM_PROVIDER      Override LLM provider
    GESTURA_VOICE_PROVIDER    Override voice provider
    OPENAI_API_KEY            OpenAI API key
    ANTHROPIC_API_KEY         Anthropic API key
    GROK_API_KEY              Grok API key

EXAMPLES:
    gestura chat                          # Start interactive chat
    gestura listen --transcribe-only      # Voice to text only
    gestura exec "Explain this code"      # Single prompt
    echo "Summarize" | gestura exec       # Pipe input
    gestura session resume --last         # Resume last session
    gestura config set llm.provider anthropic
```

### Appendix D: GUI/CLI Feature Parity Matrix

| Feature | GUI | CLI | Notes |
|---------|-----|-----|-------|
| Voice Input | ✅ | ✅ | `gestura listen` |
| AI Chat | ✅ | ✅ | `gestura chat` |
| Single Prompt | ❌ | ✅ | `gestura exec` (CLI-only) |
| Configuration | ✅ | ✅ | Settings panel / `gestura config` |
| Whisper Models | ✅ | ✅ | `gestura model whisper` |
| LLM Testing | ✅ | ✅ | `gestura model test` |
| Device Management | ✅ | ✅ | `gestura device` |
| MCP Tools | ✅ | ✅ | `gestura mcp` |
| Tool Registry | ✅ | ✅ | `/tools` in chat / `gestura tools list` |
| Capabilities Introspection | ✅ | ✅ | `/capabilities` in chat / `gestura capabilities` |
| Streaming Responses | ✅ | ✅ | Real-time token-by-token LLM responses |
| Stream Cancellation | ✅ | ✅ | Cancel in-progress streaming requests |
| Subagent Orchestration | ✅ | ❌ | GUI-only (agent spawning & task delegation) |
| Message Bus (NATS/Memory) | ✅ | ❌ | GUI-only (inter-agent communication) |
| Session Resume | ✅ | ✅ | `gestura session resume` |
| Session Fork | ❌ | ✅ | `gestura session fork` (CLI-only) |
| GDPR Export | ✅ | ✅ | `gestura privacy export` |
| System Tray | ✅ | ❌ | GUI-only |
| Onboarding | ✅ | ✅ | Onboarding window / `gestura init` |
| Shell Completions | ❌ | ✅ | `gestura completion` (CLI-only) |
| Pipe/Redirect | ❌ | ✅ | CLI-only |
| JSON Output | ❌ | ✅ | `--json` flag (CLI-only) |

### Appendix E: Change Log
| Version | Date | Author | Changes |
|---------|------|--------|---------|
| 2.3 | January 17, 2026 | Development Team | Added Streaming LLM Response Requirements (FR-STREAM-001/002/003, Section 4.14) and Subagent Orchestration Requirements (FR-AGENT-001/002/003/004, Section 4.15) |
| 2.2 | January 17, 2026 | Development Team | Added Tool Registry & Capabilities Introspection (FR-GUI-INT-003, FR-TOOLS-008/009/010, Section 4.13); enables `/tools` and `/capabilities` commands with deterministic responses bypassing LLM |
| 2.1 | January 14, 2026 | Development Team | Minor updates and clarifications |
| 2.0 | January 13, 2026 | Development Team | Major revision: Added CLI requirements, feature parity matrix, updated architecture for shared core library |
| 1.1 | August 17, 2025 | Development Team | Added comprehensive GitHub release workflow and package manager publishing |
| 1.0 | August 17, 2025 | Development Team | Initial comprehensive SRS based on implemented functionality |

---

**Document Approval:**
- **Business Owner**: Gestura LLC Executive Team - January 17, 2026
- **Technical Lead**: Development Team Lead - January 17, 2026
- **Quality Assurance**: QA Team Lead - January 17, 2026
- **Product Manager**: Product Management - January 17, 2026

**End of Document**
