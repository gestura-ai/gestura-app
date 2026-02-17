# gestura-core-scripting

Multi-language scripting engine for Gestura automation.

## What belongs here

- Script loading, validation, and sandboxed execution
- Language runtimes: Lua, Python, JavaScript
- Script metadata, permissions, and triggers
- Execution context and result tracking

## Key types

| Type | Description |
|------|-------------|
| `ScriptingEngine` | Central engine for loading and executing scripts |
| `Script` | Script metadata (id, language, source, permissions, triggers) |
| `ScriptLanguage` | `Lua`, `Python`, `JavaScript` |
| `ScriptPermission` | Sandboxed permission model (FileSystem, Network, etc.) |
| `ScriptTrigger` | Event bindings (voice command, gesture, schedule, etc.) |
| `ScriptContext` | Execution context with variables and timeout |
| `ScriptExecutionResult` | Outcome of a script run |
| `LuaRuntime` / `PythonRuntime` / `JavaScriptRuntime` | Per-language runtimes |

## Stable import paths

Most code should import through the facade:

- `gestura_core::scripting::*`

The facade in `crates/gestura-core/src/lib.rs` re-exports this crate.

## Development

```bash
cargo test -p gestura-core-scripting
cargo clippy -p gestura-core-scripting --all-targets --all-features -- -D warnings
```

