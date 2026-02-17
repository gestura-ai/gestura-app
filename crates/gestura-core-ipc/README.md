# gestura-core-ipc

IPC (inter-process communication) primitives for Gestura hotkey forwarding.

## What belongs here

- TCP-based hotkey trigger channel between GUI and CLI processes
- Discovery file management (ephemeral port file in OS temp dir)
- Hotkey server (`start_cli_hotkey_server`) and client (`try_send_hotkey_trigger_to_cli`)
- Server guard with automatic cleanup on drop

Keep platform-specific hotkey registration in presentation layers.

## Key types

| Type / Function | Description |
|-----------------|-------------|
| `CliHotkeyEndpoint` | On-disk discovery record (port, PID, version) |
| `CliHotkeyServerGuard` | RAII guard that cleans up the port file on drop |
| `start_cli_hotkey_server` | Bind an ephemeral TCP server for hotkey triggers |
| `try_send_hotkey_trigger_to_cli` | Send a trigger to a running CLI server |
| `default_cli_hotkey_port_file` | Default discovery file path |

## Stable import paths

Most code should import through the facade:

- `gestura_core::hotkey_ipc::*`

The facade in `crates/gestura-core/src/lib.rs` re-exports this crate.

## Development

```bash
cargo test -p gestura-core-ipc
cargo clippy -p gestura-core-ipc --all-targets --all-features -- -D warnings
```

