# gestura-core-a2a

A2A (Agent-to-Agent) protocol implementation for Gestura.

## What belongs here

- Transport-agnostic JSON-RPC protocol server (`A2AServer`) for hosting over HTTP/SSE
- HTTP client (`A2AClient`) for calling remote A2A agents
- A2A message types, task lifecycle, and agent card definitions

Keep HTTP listener setup in presentation layers (GUI/CLI); this crate provides
the protocol logic only.

## Key types

| Type | Description |
|------|-------------|
| `A2AServer` | JSON-RPC protocol server (transport-agnostic) |
| `A2AClient` | HTTP client for remote A2A agents |
| `A2ATask` | Task lifecycle model |
| `AgentCard` | Agent capability advertisement |
| `A2AMessage` / `A2AResponse` | Protocol message types |

## Stable import paths

Most code should import through the facade:

- `gestura_core::a2a::*`

The facade in `crates/gestura-core/src/lib.rs` re-exports this crate.

## Development

```bash
cargo test -p gestura-core-a2a
cargo clippy -p gestura-core-a2a --all-targets --all-features -- -D warnings
```

