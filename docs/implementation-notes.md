# Gestura Implementation Notes

This document captures key architectural decisions, design patterns, and implementation context for the Gestura workspace refactoring and CLI implementation project.

---

## Document History

| Date | Version | Author | Changes |
|------|---------|--------|---------|
| 2026-01-14 | 1.0 | AI Assistant | Initial creation with system tools research |

---

## 1. System Tools Design Decisions

### 1.1 Research Sources

The system tools requirements (FR-TOOLS-001 through FR-TOOLS-007) were informed by analysis of three industry-leading AI coding assistants:

#### Aider (Python)
- **Key Features**: In-agent commands (`/add`, `/drop`, `/run`, `/git`, `/undo`, `/voice`)
- **Design Pattern**: Slash commands for tool invocation within agent context
- **Notable**: Voice-to-code integration, automatic git commits, repository mapping
- **Reference**: https://aider.agent/docs/usage/commands.html

#### OpenAI Codex (Rust)
- **Key Features**: IDE extension, CLI, web interface; MCP support; skills system
- **Design Pattern**: Subcommand architecture with session management (resume/fork)
- **Notable**: Non-interactive mode for automation, AGENTS.md for custom prompts
- **Reference**: https://developers.openai.com/codex

#### Claude Code (Node.js)
- **Key Features**: `/permissions` command, bash tool, GitHub integration
- **Design Pattern**: Plugin architecture with hooks for extensibility
- **Notable**: Complex git operations, shell integration with persistence
- **Reference**: https://code.claude.com/docs

### 1.2 Tool Categories

Based on research, system tools are organized into six categories:

1. **File Operations** (FR-TOOLS-001)
   - Read, write, edit files
   - Search with ripgrep-style patterns
   - Context management (add/drop files from agent)

2. **Shell Execution** (FR-TOOLS-002)
   - Run arbitrary commands with output capture
   - Test commands (add output to agent on failure)
   - Command history and replay

3. **Git Integration** (FR-TOOLS-003)
   - AI-generated commit messages
   - Undo last AI commit
   - Conflict resolution assistance

4. **Code Analysis** (FR-TOOLS-004)
   - Repository mapping (inspired by Aider)
   - Symbol search and navigation
   - Lint and test integration

5. **Web Content** (FR-TOOLS-005)
   - URL fetching with markdown conversion
   - Web search integration
   - Documentation lookup

6. **Permission Management** (FR-TOOLS-006)
   - Granular permission levels (ask/allow/deny)
   - Allowlist/denylist patterns
   - Audit logging

### 1.3 Security Model

The permission system follows a **defense-in-depth** approach:

1. **Default Deny**: All dangerous operations require explicit confirmation
2. **Sandboxing**: Optional restriction to project directory only
3. **Audit Trail**: All tool invocations logged for review
4. **Configurable Policies**: Per-tool permission levels in `tools.toml`

---

## 2. Workspace Architecture

### 2.1 Crate Structure

```
gestura-app/
├── Cargo.toml                    # Workspace root
├── crates/
│   ├── gestura-core/             # Shared business logic (library)
│   ├── gestura-cli/              # CLI binary
│   └── gestura-gui/              # Tauri GUI binary
```

### 2.2 Module Classification

| Module | Destination | Refactoring Needed |
|--------|-------------|-------------------|
| `error.rs` | gestura-core | None |
| `config.rs` | gestura-core | None |
| `llm_provider.rs` | gestura-core | None |
| `mcp.rs` | gestura-core | None |
| `gdpr.rs` | gestura-core | None |
| `session_manager.rs` | gestura-core | None |
| `telemetry.rs` | gestura-core | None |
| `audio_capture.rs` | gestura-core | Remove config coupling |
| `speech.rs` | Split | Extract core logic from Tauri bindings |
| `tray.rs` | gestura-gui | None (GUI-only) |
| `api.rs` | gestura-gui | None (GUI-only) |

### 2.3 Dependency Strategy

- **Workspace Dependencies**: Shared versions in root `Cargo.toml`
- **Feature Flags**: `voice-local`, `voice-openai`, `nats`, `ble`, `security`
- **Optional Dependencies**: Cannot use `optional = true` in workspace deps

---

## 3. CLI Design Patterns

### 3.1 Command Structure

Using `clap` derive macros with subcommand enums:

```rust
#[derive(Parser)]
#[command(name = "gestura")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Agent(AgentArgs),
    Exec(ExecArgs),
    Tools(ToolsArgs),
    // ...
}
```

### 3.2 Interactive Mode

Using `ratatui` for terminal UI in agent mode:
- Split pane layout (input/output)
- Syntax highlighting for code blocks
- Progress indicators for long operations

---

## 4. Open Questions

1. **MCP vs Native Tools**: Should system tools be implemented as MCP servers or native Rust modules?
   - **Decision**: Native Rust for core tools, MCP for extensibility

2. **Sandbox Implementation**: Use OS-level sandboxing (seccomp, sandbox-exec) or application-level?
   - **Decision**: Application-level initially, OS-level as enhancement

3. **Tool Output Format**: Structured (JSON) or human-readable by default?
   - **Decision**: Human-readable default, `--json` flag for structured output

---

## 5. System Tools Implementation (2026-01-14)

### 5.1 Completed Implementation

All six system tool categories have been implemented in `crates/gestura-cli/src/commands/tools/`:

| Module | File | Subcommands | Status |
|--------|------|-------------|--------|
| File | `file.rs` | read, write, edit, search, list, tree, add, drop, context | ✅ Complete |
| Shell | `shell.rs` | run, test, history, last | ✅ Complete |
| Git | `git.rs` | status, diff, log, commit, undo, branch, checkout, stash, blame, conflicts, resolve | ✅ Complete |
| Code | `code.rs` | map, symbols, references, definition, lint, test, deps, stats | ✅ Complete |
| Web | `web.rs` | fetch, search, screenshot | ✅ Complete |
| Permissions | `permissions.rs` | list, grant, revoke, reset, check | ✅ Complete |

### 5.2 Key Implementation Details

1. **File Search**: Uses `regex` crate for pattern matching. Respects common ignore patterns (.git, node_modules, target).

2. **Shell Execution**: Uses `std::process::Command` with `sh -c` for command execution. Includes safety analysis for dangerous patterns.

3. **Git Operations**: Wraps native `git` commands. Provides colorized output for status, diff, and log.

4. **Code Analysis**: Uses regex-based symbol extraction (future: tree-sitter for proper parsing). Includes repository mapping and statistics.

5. **Web Fetching**: Uses `reqwest` for HTTP requests. Basic HTML-to-text conversion (future: use `scraper` crate).

6. **Permissions**: JSON-based permission storage in `~/.config/gestura/permissions.json`. Default permissions for safe operations.

### 5.3 Dependencies Added

```toml
# crates/gestura-cli/Cargo.toml
regex = "1.12"
reqwest = { version = "0.12", features = ["json"] }
```

### 5.4 Usage Examples

```bash
# File operations
gestura tools file read Cargo.toml --lines 1-20
gestura tools file list --all
gestura tools file tree --depth 3
gestura tools file search "fn main" --recursive

# Shell operations
gestura tools shell run "ls -la"
gestura tools shell test "rm -rf /"  # Safety check

# Git operations
gestura tools git status
gestura tools git diff --staged
gestura tools git log --count 5 --oneline

# Code analysis
gestura tools code stats
gestura tools code symbols src/main.rs
gestura tools code map --depth 2

# Web operations
gestura tools web fetch https://example.com

# Permissions
gestura tools permissions list
gestura tools permissions grant shell.run
gestura tools permissions check write ./file.txt
```

---

## 6. CLI UX Conventions: Claude-Code/Codex Style Mapping

### 6.1 Design Principles

The Gestura CLI follows modern AI assistant conventions inspired by Claude Code and Codex:

1. **Minimal Chrome**: Reduce visual noise; let content dominate
2. **Consistent Prefixes**: Clear role identification in transcripts
3. **Compact Tables**: Aligned columns for tool/command listings
4. **Status Feedback**: Always-visible state (Ready, Thinking, Error)
5. **Keyboard-First**: Discoverable shortcuts in status/footer

### 6.2 Visual Layout

#### Header (Non-TUI)
```
╭──────────────────────────────────────────────────────────────╮
│ gestura — voice-first AI assistant                           │
│ session abc12345 · provider openai · model gpt-4             │
├──────────────────────────────────────────────────────────────┤
│ /help commands · /tools list · Ctrl+C quit                   │
╰──────────────────────────────────────────────────────────────╯
```

#### Header (TUI)
```
┌─ gestura — openai ───────────────────────────────────────────┐
│ session abc12345 │ model gpt-4 │ 5 messages                  │
```

#### Message Prefixes
- User: `>` (green)
- Assistant: `◆` (blue)
- System/Error: `!` (red)
- Loading: `◇` (dimmed)

#### Status Bar
```
 Ready                                 Enter send │ /help │ Ctrl+C quit
```

### 6.3 Slash Commands

| Command | Alias | Description |
|---------|-------|-------------|
| `/help` | `/h`, `/?` | Show commands table |
| `/tools` | `/t` | List tool registry |
| `/tools <name>` | | Show tool details |
| `/quit` | `/q`, `/exit` | Save and exit |
| `/clear` | `/c` | Clear transcript |
| `/history` | | Show session stats |
| `/new` | | Start new session |
| `/save` | | Force-save session |

### 6.4 Tool Display Format

Compact table with fixed-width columns:
```
┌─ Tools ───────────────────────────────────────────────────────┐
│                                                               │
│  file:read       Read file contents with optional line range  │
│  file:write      Write or create file with given content      │
│  shell:run       Execute shell command and capture output     │
│  ...                                                          │
│                                                               │
└───────────────────────────────────────────────────────────────┘
```

### 6.5 Keybindings

| Key | Action |
|-----|--------|
| `Enter` | Send message |
| `Ctrl+C` | Quit (save session) |
| `Ctrl+Q` | Quit (TUI only) |
| `Esc` | Clear input (TUI) |
| `↑/↓` | Scroll history (TUI) |
| `PageUp/PageDown` | Fast scroll (TUI) |

### 6.6 Guardrails

**Allowed** (default):
- Read files within project directory
- Run safe shell commands (ls, cat, grep, etc.)
- Git status, diff, log operations

**Requires Confirmation**:
- Write/delete files
- Execute arbitrary shell commands
- Git commits, pushes, merges
- Network requests outside docs

**Not Allowed** (blocked):
- System-level destructive commands (rm -rf /, etc.)
- Credential/secret access without explicit grant
- Operations outside sandboxed scope

### 6.7 Tool Call Display

When the assistant invokes a tool:
```
◆ Let me check the file contents...

  ┌─ tool:file:read ─────────────────────────────────────────┐
  │ path: src/main.rs                                         │
  │ lines: 1-50                                               │
  └───────────────────────────────────────────────────────────┘

  ─ Result ───────────────────────────────────────────────────
  [file contents here]
  ──────────────────────────────────────────────────────────────
```

---

## 7. References

- [SRS Document](./SRS-gestura-app.md) - Full requirements specification
- [Aider Commands](https://aider.agent/docs/usage/commands.html)
- [Codex Documentation](https://developers.openai.com/codex)
- [Claude Code Docs](https://code.claude.com/docs)

