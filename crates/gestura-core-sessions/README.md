# gestura-core-sessions

Chat session persistence, session management, and workspace tracking for Gestura.

## What belongs here

- Chat session CRUD (create, read, update, delete, list)
- Session message store and legacy migration
- Session manager (active session tracking)
- Session workspace detection and management
- Session type definitions

Keep GUI/CLI session display concerns out of this crate; those belong in presentation layers.

## Modules

- `chat_sessions`      Session CRUD, store, types, and legacy migration
- `session_manager`    Active session tracking and lifecycle
- `session_workspace`  Workspace detection for session context

## Stable import paths

Most code should import through the facade:

- `gestura_core::chat_sessions::*`
- `gestura_core::session_manager::*`
- `gestura_core::session_workspace::*`

The facades in `crates/gestura-core/src/` re-export this crate.

## Development

```bash
cargo test -p gestura-core-sessions
cargo clippy -p gestura-core-sessions --all-targets --all-features -- -D warnings
```

