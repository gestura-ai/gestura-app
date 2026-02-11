# gestura-core-tools

The **tools domain** for Gestura: built-in tool implementations plus tool policy/permissions.

This crate is intended to be independently workable (fast builds, low coupling).

## What belongs here

- Built-in tools (file/shell/git/web/screen/etc.)
- Tool registry and schemas
- Tool permission model and policy helpers

Avoid putting pipeline orchestration or MCP transport concerns here; those stay in `gestura-core`.

## Modules (high signal)

- `file` 		File read/write/edit/search/tree
- `shell` 	Shell execution + history
- `git` 		Git status/log/branches
- `web` 		Web fetch + content extraction
- `screen` 	Screenshot / screen recording helpers
- `registry` 	Built-in tool list (`all_tools()`)
- `schemas` 	Provider tool schemas (OpenAI/Anthropic)
- `permissions` 	Permission manager + auditing
- `policy` 		Policy evaluation helpers

## Stable import paths

Most code should import tools through the facade:

- `gestura_core::tools::*`

The facade lives in `crates/gestura-core/src/tools/` and re-exports this crate.

## Development

```bash
cargo test -p gestura-core-tools
cargo clippy -p gestura-core-tools --all-targets --all-features -- -D warnings
```
