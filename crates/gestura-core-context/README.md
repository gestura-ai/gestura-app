# gestura-core-context

Smart context management for efficient LLM interactions in Gestura.

## What belongs here

- Request analysis (intent detection, entity extraction, category classification)
- Context resolution (load only the context needed for a request)
- Context caching with TTL
- Tool provider callback for decoupled tool registry integration

## Architecture

The context system uses a three-tier approach:

1. **Request Analysis** — parse user requests to determine intent without an LLM
2. **Context Resolution** — load only the context needed for the request
3. **Smart Caching** — cache frequently accessed context with TTL

Tool availability is injected via `ToolProviderFn` callback rather than a direct
dependency on the tool registry, keeping this crate decoupled from `gestura-core-tools`.

## Modules

- `analyzer`  — `RequestAnalyzer` (regex-based intent and entity extraction)
- `cache`     — `ContextCache<T>` (generic TTL cache)
- `manager`   — `ContextManager` (orchestrates analysis, resolution, and caching)

## Key types

| Type | Description |
|------|-------------|
| `ContextManager` | Central manager: analyse, resolve, cache |
| `RequestAnalyzer` | Regex-based request analysis |
| `ContextCache<T>` | Generic TTL-based cache |
| `ToolProviderFn` | `Box<dyn Fn() -> Vec<(String, String)> + Send + Sync>` |
| `ContextManagerStats` / `CacheStats` | Observability stats |

Types re-exported from `gestura-core-foundation::context`:

| Type | Description |
|------|-------------|
| `ResolvedContext` | Fully resolved context for an LLM call |
| `RequestAnalysis` | Result of request analysis |
| `ContextCategory` | FileSystem, Git, Shell, Web, General, etc. |
| `EntityType` | FilePath, Url, GitRef, etc. |
| `FileContext` / `ToolContext` | Context payloads |

## Stable import paths

Most code should import through the facade:

- `gestura_core::context::*`

The facade in `crates/gestura-core/src/lib.rs` re-exports this crate.

## Development

```bash
cargo test -p gestura-core-context
cargo clippy -p gestura-core-context --all-targets --all-features -- -D warnings
```

