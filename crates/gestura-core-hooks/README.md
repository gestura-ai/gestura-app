# gestura-core-hooks

Safe-by-default hooks engine for Gestura, inspired by Claude Code's hooks model.

## What belongs here

- Hook data model: events, definitions, command templates
- Hook engine: registration, matching, execution records
- Hook executor: process-based command execution with allow-listing
- Template rendering with variable substitution

Hooks are **disabled by default** and require explicit allow-listing of
programs before anything is executed.

## Modules

- `engine`    — `HookEngine`, `HookExecutionRecord`
- `executor`  — `HookExecutor` trait, `ProcessHookExecutor`
- `template`  — `TemplateVars`, `render_template`
- `types`     — `HookEvent`, `HookDefinition`, `HookCommandTemplate`, `HookContext`, `HooksSettings`

## Key types

| Type | Description |
|------|-------------|
| `HookEngine` | Central engine for registering and firing hooks |
| `HookEvent` | Events that can trigger hooks (e.g., pre/post tool execution) |
| `HookDefinition` | A hook binding an event to a command template |
| `HookExecutor` / `ProcessHookExecutor` | Trait + impl for executing hook commands |
| `HooksSettings` | User-facing configuration for allowed programs |

## Stable import paths

Most code should import through the facade:

- `gestura_core::hooks::*`

The facade in `crates/gestura-core/src/lib.rs` re-exports this crate.

## Development

```bash
cargo test -p gestura-core-hooks
cargo clippy -p gestura-core-hooks --all-targets --all-features -- -D warnings
```

