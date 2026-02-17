# Agent Loop Architecture Research

**Date:** 2026-01-20  
**Status:** Complete  
**Task:** Task 32 - Research Agent Loop Architectures

## Executive Summary

This document analyzes agent execution patterns from three leading open-source coding agent projects to inform Gestura's agent architecture. Key findings include common patterns around MCP integration, tool orchestration, permission management, and context handling.

---

## Projects Analyzed

| Project | Language | Focus | License |
|---------|----------|-------|---------|
| [OpenAI Codex](https://github.com/openai/codex) | Rust (97.5%) | Lightweight CLI coding agent | Apache-2.0 |
| [Block Goose](https://github.com/block/goose) | Rust (59.6%) + TypeScript | Local extensible AI agent | Apache-2.0 |
| [Kilo Code](https://github.com/Kilo-Org/kilocode) | TypeScript (91.1%) | VS Code AI coding extension | Apache-2.0 |

---

## 1. OpenAI Codex Architecture

### Structure
```
codex-rs/
├── core/           # Core agent logic
├── cli/            # Command-line interface
├── tui/            # Terminal UI
├── protocol/       # Communication protocol
├── mcp-server/     # MCP server implementation
└── exec/           # Execution environment
```

### Key Patterns

**Modular Crate Design:**
- Separate crates for core, CLI, TUI, protocol, and MCP
- Clean separation between agent logic and presentation layers
- `CodexThread` and `ThreadManager` for conversation management

**Sandbox Environment:**
- `CODEX_SANDBOX_NETWORK_DISABLED_ENV_VAR` for network isolation
- Dedicated sandboxing module for safe code execution
- Environment variable-based configuration

**Response Streaming:**
- `ResponseStream` type for streaming LLM responses
- Async-first design with tokio runtime

**Testing:**
- Snapshot testing with `insta` crate
- Comprehensive test coverage for core functionality

---

## 2. Block Goose Architecture

### Structure
```
crates/
├── goose/          # Core agent logic
├── goose-cli/      # CLI entry point
├── goose-server/   # Backend server (goosed)
├── goose-mcp/      # MCP extensions
├── goose-bench/    # Benchmarking
├── mcp-client/     # MCP client
├── mcp-core/       # MCP shared types
└── mcp-server/     # MCP server
```

### Core Agent Implementation

The `Agent` struct is the heart of Goose's architecture:

```rust
pub struct Agent {
    pub provider: SharedProvider,
    pub config: AgentConfig,
    pub extension_manager: Arc<ExtensionManager>,
    pub prompt_manager: Mutex<PromptManager>,
    pub retry_manager: RetryManager,
    pub tool_inspection_manager: ToolInspectionManager,
    // Channels for async communication
}
```

### Key Patterns

**Tool Categorization:**
- Frontend tools (UI-driven)
- Platform tools (system-level)
- Subagent tools (delegated tasks)
- Extension tools (MCP-based)

**Permission System:**
```rust
pub enum PermissionCheckResult {
    Approved,
    NeedsApproval { reason: String },
    Denied { reason: String },
}
```

**Retry Logic:**
- `RetryManager` handles transient failures
- Configurable retry policies per operation type
- Exponential backoff with jitter

**Context Compaction:**
- Automatic compaction on `ContextLengthExceeded` error
- Preserves system prompts and recent exchanges
- Removes redundant middle content

**Event Streaming:**
```rust
pub enum AgentEvent {
    Message(Message),
    McpNotification((String, ServerNotification)),
    ModelChange { model: String, mode: String },
    HistoryReplaced(Conversation),
}
```

**Execution Modes:**
- `GooseMode::Auto` - Autonomous tool execution
- `GooseMode::Agent` - Interactive conversation mode

---

## 3. Kilo Code Architecture

### Structure
```
├── src/                # VS Code extension core
│   ├── api/providers/  # 50+ AI provider implementations
│   ├── core/tools/     # Tool implementations
│   └── services/       # MCP, browser, checkpoints
├── webview-ui/         # React frontend
├── cli/                # Standalone CLI
├── packages/           # Shared packages
└── jetbrains/          # JetBrains plugin
```

### Key Patterns

**Multi-Mode Architecture:**
- Architect mode (planning)
- Coder mode (implementation)
- Debugger mode (troubleshooting)

**Provider Abstraction:**
- 500+ AI model support
- Unified provider interface
- Dynamic model discovery

**MCP Marketplace:**
- Extensible tool ecosystem
- Server discovery and installation
- Permission-based tool access

---

## Common Patterns Across All Projects

### 1. MCP (Model Context Protocol) Integration
All three projects use MCP as the primary extension mechanism:
- Standardized tool interface
- Server-based tool hosting
- Dynamic capability discovery

### 2. Separation of Concerns
```
┌─────────────────────────────────────────────────┐
│                   Presentation                   │
│         (CLI / TUI / GUI / VS Code)             │
├─────────────────────────────────────────────────┤
│                   Agent Core                     │
│    (Conversation, Tool Dispatch, Retry)         │
├─────────────────────────────────────────────────┤
│                   Providers                      │
│      (OpenAI, Anthropic, Ollama, etc.)          │
├─────────────────────────────────────────────────┤
│                   Tools/MCP                      │
│    (File, Shell, Web, Code Analysis)            │
└─────────────────────────────────────────────────┘
```

### 3. Permission and Safety Layers
- Tool inspection before execution
- User confirmation for dangerous operations
- Sandbox environments for code execution
- Allowlist/denylist for tool access

### 4. Session and State Management
- Persistent session storage
- Conversation history management
- Extension state preservation
- Workspace context tracking

### 5. Streaming Response Handling
- Async streaming from LLM providers
- Event-based UI updates
- Partial response rendering
- Error recovery during streams

---

## Recommendations for Gestura

### Patterns to Adopt

| Pattern | Priority | Rationale |
|---------|----------|-----------|
| **MCP Integration** | HIGH | Industry standard for tool extensibility |
| **Tool Inspection Manager** | HIGH | Security and permission control |
| **Retry Manager** | HIGH | Resilience for transient failures |
| **Context Compaction** | HIGH | Token efficiency for long conversations |
| **Event Streaming** | MEDIUM | Real-time UI updates |
| **Execution Modes** | MEDIUM | Auto vs interactive control |
| **Extension Manager** | MEDIUM | Plugin architecture |

### Patterns to Avoid

| Anti-Pattern | Reason |
|--------------|--------|
| Monolithic agent class | Hard to test and extend |
| Synchronous tool execution | Blocks UI and limits parallelism |
| Hardcoded tool lists | Prevents extensibility |
| Global mutable state | Race conditions and testing issues |

### Implementation Roadmap

**Phase 1: Core Agent Refactor**
1. Extract `AgentCore` struct with clear responsibilities
2. Implement `ToolDispatcher` for routing tool calls
3. Add `RetryManager` with configurable policies
4. Implement `ContextManager` for token-aware history

**Phase 2: Permission System**
1. Define `PermissionLevel` enum (Sandbox/Restricted/Full)
2. Implement `ToolInspector` trait for pre-execution checks
3. Add user confirmation flow for dangerous operations
4. Create permission persistence per session

**Phase 3: MCP Integration**
1. Implement MCP client for external tool servers
2. Add MCP server for exposing Gestura tools
3. Create tool discovery and registration system
4. Build extension marketplace UI

**Phase 4: Advanced Features**
1. Multi-mode support (Architect/Coder/Debugger)
2. Subagent delegation for complex tasks
3. Checkpoint and rollback system
4. Parallel tool execution

---

## Technical Debt to Address

Based on this research, Gestura should address:

1. **Session LLM Config** - Currently session overrides may not persist correctly
2. **Tool Permission UI** - Settings panel exists but backend not wired
3. **Context Management** - Need automatic compaction on context overflow
4. **Error Recovery** - Add retry logic for transient LLM failures

---

## References

- [OpenAI Codex Repository](https://github.com/openai/codex)
- [Block Goose Repository](https://github.com/block/goose)
- [Kilo Code Repository](https://github.com/Kilo-Org/kilocode)
- [Model Context Protocol Specification](https://modelcontextprotocol.io/)

---

*Document generated as part of Task 32 research.*

