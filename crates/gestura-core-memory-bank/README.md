# gestura-core-memory-bank

Persistent context storage for Gestura conversation history, inspired by
Kilo Code's Memory Bank concept.

## What belongs here

- `MemoryBankEntry` data model (markdown-based, human-readable format)
- Async CRUD operations: save, load, list, search, update, delete, clear
- Workspace-scoped storage under `.gestura/memory/`

Keep GUI/CLI display concerns out of this crate; those belong in presentation layers.

## Key types

| Type / Function | Description |
|-----------------|-------------|
| `MemoryBankEntry` | Timestamped, categorised conversation memory |
| `MemoryBankError` | Domain error enum (I/O, parse) |
| `save_to_memory_bank` | Persist an entry to disk |
| `load_from_memory_bank` | Load an entry from a markdown file |
| `list_memory_bank` | List all entries in a workspace |
| `search_memory_bank` | Full-text search across entries |
| `delete_memory_bank_entry` | Remove a single entry |
| `clear_memory_bank` | Remove all entries |

## Stable import paths

Most code should import through the facade:

- `gestura_core::memory_bank::*`

The facade in `crates/gestura-core/src/lib.rs` re-exports this crate.

## Development

```bash
cargo test -p gestura-core-memory-bank
cargo clippy -p gestura-core-memory-bank --all-targets --all-features -- -D warnings
```

