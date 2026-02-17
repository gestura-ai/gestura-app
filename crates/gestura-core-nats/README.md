# gestura-core-nats

NATS messaging queue utilities for Gestura.

## What belongs here

- NATS connection management (connect, retry)
- Embedded NATS server spawning with JetStream
- Publish/subscribe helpers (JSON payloads, wildcard subjects)
- JetStream KV bucket initialization
- Connection health monitoring
- Standard subject constants (`subjects::*`)
- Dispatch event routing types

## Feature gates

The `nats` Cargo feature enables the `async-nats` implementation.
When disabled, stub types and no-op functions are provided so the
rest of the workspace compiles without the NATS dependency.

## Key types

| Type / Function | Description |
|-----------------|-------------|
| `Connection` | `async_nats::Client` (or `()` stub) |
| `connect_nats` | Connect to a NATS server |
| `connect_with_retry` | Connect with automatic retries |
| `spawn_nats_server` | Launch embedded NATS with JetStream |
| `publish_json` | Publish a JSON payload to a subject |
| `subscribe` / `subscribe_wildcard` | Subscribe to subjects |
| `init_jetstream` | Create JetStream KV bucket |
| `NatsHealthMonitor` | Connection health watcher |
| `DispatchEvent` | Event routing hint enum |
| `subjects` | Standard NATS subject constants |

## Stable import paths

Most code should import through the facade:

- `gestura_core::nats_mq::*`

The facade in `crates/gestura-core/src/lib.rs` re-exports this crate.

## Development

```bash
cargo test -p gestura-core-nats
cargo clippy -p gestura-core-nats --all-targets --all-features -- -D warnings
```

