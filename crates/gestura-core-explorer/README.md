# gestura-core-explorer

File system explorer utility for the Gestura agent pipeline.

## What belongs here

- Safe relative path validation and resolution
- Directory listing (sorted, truncated, symlink-aware)
- Git status parsing (`git status --porcelain`)
- Workspace root canonicalization

Keep GUI/CLI rendering of explorer results in presentation layers.

## Key types

| Type / Function | Description |
|-----------------|-------------|
| `ExplorerEntry` | A single directory entry (name, path, kind) |
| `ExplorerEntryKind` | File or directory |
| `ExplorerListDirResponse` | Directory listing payload |
| `ExplorerRootResponse` | Workspace root info |
| `ExplorerGitStatusResponse` | Per-path git status |
| `ExplorerGitChangeKind` | Added, Modified, Deleted, Renamed, etc. |
| `ExplorerError` | Domain error enum |
| `list_dir` | List directory contents safely |
| `ensure_safe_rel_path` | Validate a relative path |
| `resolve_under_root` | Resolve and sandbox a path within root |

## Stable import paths

Most code should import through the facade:

- `gestura_core::explorer::*`

The facade in `crates/gestura-core/src/lib.rs` re-exports this crate.

## Development

```bash
cargo test -p gestura-core-explorer
cargo clippy -p gestura-core-explorer --all-targets --all-features -- -D warnings
```

