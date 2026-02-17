# gestura-core-knowledge

Progressive-disclosure knowledge system for agent expertise and context,
inspired by the claude-skills pattern.

## What belongs here

- `KnowledgeStore` — register, persist, and query knowledge items
- `KnowledgeSettingsManager` — per-session knowledge preferences
- Built-in expertise files (Rust, Tauri, CLI, MCP, voice, A2A, Anthropic sales skills)
- Knowledge types, scoring, and trigger matching

Keep pipeline-level knowledge injection in `gestura-core`.

## Key types

| Type | Description |
|------|-------------|
| `KnowledgeStore` | Central store for registering and querying knowledge |
| `KnowledgeItem` | A single knowledge item with triggers and references |
| `KnowledgeQuery` | Query parameters for searching knowledge |
| `KnowledgeMatch` | A scored result from a knowledge search |
| `KnowledgeSettingsManager` | Per-session knowledge preferences |
| `register_builtin_knowledge` | Registers all built-in expertise items |

## Stable import paths

Most code should import through the facade:

- `gestura_core::knowledge::*`

The facade in `crates/gestura-core/src/lib.rs` re-exports this crate.

## Development

```bash
cargo test -p gestura-core-knowledge
cargo clippy -p gestura-core-knowledge --all-targets --all-features -- -D warnings
```

