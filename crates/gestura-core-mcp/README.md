# gestura-core-mcp

MCP (Model Context Protocol) 2025-11-25 implementation for Gestura.

## What belongs here

- MCP client and server implementations
- Service discovery and lifecycle management
- Prompt registry and notification handling
- MCP-specific types, error handling, and configuration
- Tool inspection for MCP tool calls

Keep higher-level pipeline orchestration in `gestura-core`; this crate implements the MCP protocol layer.

## Modules

- `client`          MCP client and client registry
- `server`          MCP server implementation
- `discovery`       Service discovery (filesystem, network)
- `integrator`      Local MCP integration and MDH translation
- `lifecycle`       Session management for MCP connections
- `notifications`   Notification handler and dispatch
- `prompts`         Prompt registry for MCP prompt resources
- `types`           MCP protocol types and message definitions
- `config`          MCP-specific configuration types
- `error`           MCP error types
- `execution_mode`  Execution mode helpers for MCP context
- `tool_inspection` Tool call inspection and validation

## Stable import paths

Most code should import through the facade:

- `gestura_core::mcp::*`
- `gestura_core::tool_inspection::*`

The facade in `crates/gestura-core/src/mcp/mod.rs` re-exports this crate.

## Development

```bash
cargo test -p gestura-core-mcp
cargo clippy -p gestura-core-mcp --all-targets --all-features -- -D warnings
```

