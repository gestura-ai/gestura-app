# gestura-core-tasks

Task management and workflow primitives for Gestura.

## What belongs here

- Task CRUD operations and persistence
- Task state machine (pending → in-progress → complete/failed)
- Workflow definitions and execution tracking
- Task template management

Keep pipeline orchestration and agent loop concerns in `gestura-core`.

## Modules

- `tasks`       Task types, CRUD, persistence, and state management
- `workflows`   Workflow definitions, templates, and execution tracking

## Stable import paths

Most code should import through the facade:

- `gestura_core::tasks::*`
- `gestura_core::workflows::*`

The facades in `crates/gestura-core/src/` re-export this crate.

## Development

```bash
cargo test -p gestura-core-tasks
cargo clippy -p gestura-core-tasks --all-targets --all-features -- -D warnings
```

