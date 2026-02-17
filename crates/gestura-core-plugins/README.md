# gestura-core-plugins

Plugin system for Gestura: discovery, lifecycle management, and sandboxed execution.

## What belongs here

- Plugin metadata, dependencies, and permission model
- Plugin lifecycle: load → start → stop → unload
- `PluginManager` for centralized plugin management
- `PluginApi` trait for plugin implementations
- Event and command handler routing

## Key types

| Type | Description |
|------|-------------|
| `PluginManager` | Central manager for plugin discovery and lifecycle |
| `Plugin` | Plugin instance (metadata, state, config) |
| `PluginMetadata` | Name, version, author, dependencies, permissions |
| `PluginApi` | Trait that plugin implementations must satisfy |
| `PluginPermission` | Sandboxed permission model (FileSystem, Network, etc.) |
| `PluginState` | `Loaded`, `Running`, `Stopped`, `Error`, `Disabled` |
| `PluginDependency` | Inter-plugin dependency declaration |
| `get_plugin_manager` | Global singleton accessor |

## Stable import paths

Most code should import through the facade:

- `gestura_core::plugin_system::*`

The facade in `crates/gestura-core/src/lib.rs` re-exports this crate.

## Development

```bash
cargo test -p gestura-core-plugins
cargo clippy -p gestura-core-plugins --all-targets --all-features -- -D warnings
```

