# Gestura.app API Documentation

## Overview

Gestura.app provides multiple API interfaces:

1. **gestura-core Library API** - Rust library for direct integration
2. **MCP Protocol** - Model Context Protocol for AI tool execution
3. **CLI Commands** - Command-line interface
4. **Tauri Commands** - GUI IPC commands

## gestura-core Library API

The core library exports types and functions for all Gestura functionality.

### Pipeline API

Execute agent requests through the pipeline:

```rust
use gestura_core::pipeline::{Pipeline, AgentRequest, AgentResponse};

// Create a pipeline
let pipeline = Pipeline::new(config).await?;

// Execute a request
let request = AgentRequest::new("Analyze this code");
let response = pipeline.process(request).await?;
```

### Agent Sessions API

Manage persistent agent sessions:

```rust
use gestura_core::agent_sessions::{AgentSession, AgentSessionStore, FileAgentSessionStore};

// Create a session store
let store = FileAgentSessionStore::new(Path::new("~/.gestura/sessions"))?;

// Create a new session
let session = AgentSession::new("project-agent")?;
store.save(&session).await?;

// List sessions
let sessions = store.list().await?;

// Load a session
let session = store.load("session-id").await?;
```

### Tool Registry API

Register and execute tools:

```rust
use gestura_core::tools::{ToolRegistry, ToolDefinition, PermissionManager};

// Create a registry
let registry = ToolRegistry::new();

// Register a tool
registry.register(ToolDefinition {
    name: "my_tool".to_string(),
    description: "Custom tool".to_string(),
    input_schema: json!({"type": "object"}),
    handler: Box::new(my_handler),
})?;

// Check permissions
let perms = PermissionManager::new(config);
if perms.is_action_allowed(&request) {
    // Execute tool
}
```

### MCP Server API

Create an MCP server:

```rust
use gestura_core::mcp::{McpServer, McpServerConfig};

let config = McpServerConfig {
    name: "my-server".to_string(),
    version: "1.0.0".to_string(),
    ..Default::default()
};

let server = McpServer::new(config)?;
server.start().await?;
```

### Security API

Encrypt and store sensitive data:

```rust
use gestura_core::security::{SecureStorage, Encryptor, create_secure_storage};

// Create secure storage
let storage = create_secure_storage("my-app")?;

// Store a secret
storage.set("api_key", "secret_value").await?;

// Retrieve a secret
let value = storage.get("api_key").await?;

// Encrypt data
let encryptor = Encryptor::new(key)?;
let encrypted = encryptor.encrypt(data)?;
```

### Analytics API

Track usage with privacy controls:

```rust
use gestura_core::analytics::{UsageAnalytics, PrivacyMode};

let analytics = UsageAnalytics::new(PrivacyMode::Limited);

// Track an event
analytics.track_event("tool_used", json!({"tool": "file_read"})).await?;

// Get insights
let insights = analytics.get_insights().await?;
```

## MCP Protocol API

Gestura implements MCP (Model Context Protocol) version 2025-11-25 for AI tool execution.

### Protocol Overview

MCP uses JSON-RPC 2.0 over STDIO for communication:

```json
{"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {...}}
```

### Initialization

```json
// Request
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "initialize",
  "params": {
    "protocolVersion": "2025-11-25",
    "capabilities": {
      "tools": {},
      "resources": {},
      "prompts": {}
    },
    "clientInfo": {
      "name": "my-client",
      "version": "1.0.0"
    }
  }
}

// Response
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "protocolVersion": "2025-11-25",
    "serverInfo": {
      "name": "gestura-mcp",
      "version": "0.1.0"
    },
    "capabilities": {
      "tools": {"listChanged": true},
      "resources": {"subscribe": true}
    }
  }
}
```

### List Tools

```json
// Request
{"jsonrpc": "2.0", "id": 2, "method": "tools/list"}

// Response
{
  "jsonrpc": "2.0",
  "id": 2,
  "result": {
    "tools": [
      {
        "name": "file_read",
        "description": "Read file contents",
        "inputSchema": {
          "type": "object",
          "properties": {
            "path": {"type": "string"}
          },
          "required": ["path"]
        }
      }
    ]
  }
}
```

### Call Tool

```json
// Request
{
  "jsonrpc": "2.0",
  "id": 3,
  "method": "tools/call",
  "params": {
    "name": "file_read",
    "arguments": {"path": "/path/to/file.txt"}
  }
}

// Response
{
  "jsonrpc": "2.0",
  "id": 3,
  "result": {
    "content": [
      {"type": "text", "text": "File contents here..."}
    ]
  }
}
```

### Built-in Tools

| Tool | Description | Required Args |
|------|-------------|---------------|
| `file_read` | Read file contents | `path` |
| `file_write` | Write to file | `path`, `content` |
| `file_edit` | Edit file with search/replace | `path`, `edits` |
| `shell_exec` | Execute shell command | `command` |
| `git_status` | Get git repository status | - |
| `git_diff` | Get git diff | `ref` |
| `web_fetch` | Fetch URL content | `url` |

## CLI Commands

The `gestura-cli` provides command-line access to all features.

### Agent Commands

```bash
# Start interactive agent
gestura agent

# One-shot execution
gestura exec "Explain this code"

# Continue a session
gestura agent --session <session-id>
```

### Session Management

```bash
# List sessions
gestura session list

# Show session details
gestura session show <session-id>

# Delete a session
gestura session delete <session-id>
```

### MCP Commands

```bash
# Start MCP server
gestura mcp serve

# List available tools
gestura mcp tools

# Call a tool
gestura mcp call file_read --path /path/to/file
```

### A2A Commands

```bash
# Start A2A server
gestura a2a serve --port 8080

# Discover agents
gestura a2a discover

# Send message to agent
gestura a2a send --agent <agent-id> --message "Hello"
```

### Configuration

```bash
# Show config
gestura config show

# Set config value
gestura config set llm.provider openai

# Initialize project
gestura init
```

### Other Commands

```bash
# Listen for voice input
gestura listen

# Health check
gestura health

# Generate shell completions
gestura completion bash > ~/.bash_completion.d/gestura
```

## Tauri Commands (GUI IPC)

The GUI exposes Tauri commands for frontend-backend communication:

### Session Commands

```typescript
// List agent sessions
await invoke('list_agent_sessions');

// Create new session
await invoke('create_agent_session', { name: 'My Agent' });

// Load session
await invoke('load_agent_session', { sessionId: 'uuid' });

// Delete session
await invoke('delete_agent_session', { sessionId: 'uuid' });
```

### Agent Commands

```typescript
// Execute agent request
await invoke('execute_agent_request', {
  prompt: 'Analyze this code',
  sessionId: 'uuid'
});

// Cancel execution
await invoke('cancel_execution', { requestId: 'uuid' });
```

### Tool Commands

```typescript
// List available tools
await invoke('list_tools');

// Execute tool
await invoke('execute_tool', {
  name: 'file_read',
  arguments: { path: '/path/to/file' }
});

// Confirm tool execution (restricted mode)
await invoke('confirm_tool_execution', {
  requestId: 'uuid',
  confirmed: true
});
```

### Configuration Commands

```typescript
// Get configuration
await invoke('get_config');

// Update configuration
await invoke('update_config', {
  key: 'llm.provider',
  value: 'openai'
});
```

## Error Handling

### Rust Error Types

```rust
use gestura_core::error::GesturaError;

// Error variants
pub enum GesturaError {
    Config(String),
    Io(std::io::Error),
    Pipeline(String),
    Tool(String),
    Permission(String),
    Session(String),
    Mcp(String),
}
```

### MCP Error Codes

| Code | Description |
|------|-------------|
| `-32700` | Parse error |
| `-32600` | Invalid request |
| `-32601` | Method not found |
| `-32602` | Invalid params |
| `-32603` | Internal error |

## Key Types Reference

### Pipeline Types

```rust
pub struct AgentRequest {
    pub id: String,
    pub prompt: String,
    pub context: Option<Context>,
    pub tools: Vec<ToolDefinition>,
}

pub struct AgentResponse {
    pub id: String,
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
    pub usage: TokenUsage,
}
```

### Session Types

```rust
pub struct AgentSession {
    pub id: String,
    pub name: String,
    pub messages: Vec<AgentMessage>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub struct AgentMessage {
    pub role: Role,
    pub content: String,
    pub tool_calls: Option<Vec<ToolCall>>,
}
```

### Tool Types

```rust
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

pub struct ToolResult {
    pub call_id: String,
    pub content: Vec<Content>,
    pub is_error: bool,
}
```

## Support

- **Repository**: https://github.com/gestura-ai/gestura-app
- **Documentation**: https://docs.gestura.app
- **Issues**: https://github.com/gestura-ai/gestura-app/issues
