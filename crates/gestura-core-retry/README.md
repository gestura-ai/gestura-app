# gestura-core-retry

Retry strategies and execution for Gestura (exponential backoff, jitter).

## What belongs here

- Error classification (transient vs. permanent)
- Retry policy configuration (max attempts, delays, backoff, jitter)
- `RetryManager` for executing async operations with automatic retry
- Retry event callbacks for observability

## Key types

| Type | Description |
|------|-------------|
| `RetryManager` | Executes async operations with configurable retry policy |
| `RetryPolicy` | Max attempts, initial/max delay, backoff multiplier, jitter |
| `ErrorClass` | `Transient`, `Permanent`, `Unknown` |
| `RetryEvent` | Notification payload for each retry attempt |
| `RetryCallback` | `Box<dyn Fn(RetryEvent) + Send + Sync>` |

## Stable import paths

Most code should import through the facade:

- `gestura_core::retry::*`

The facade in `crates/gestura-core/src/lib.rs` re-exports this crate.

## Development

```bash
cargo test -p gestura-core-retry
cargo clippy -p gestura-core-retry --all-targets --all-features -- -D warnings
```

