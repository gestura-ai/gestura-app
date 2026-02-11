# gestura-core-streaming

Streaming LLM provider support, cancellation, and shell output streaming for Gestura.

## What belongs here

- LLM streaming response processing (`StreamChunk`, `start_streaming`)
- Token usage tracking and pricing (`TokenUsageStatus`)
- Stream cancellation registry
- Shell output streaming (`ShellOutputStream`, `ShellProcessState`)
- Think-block splitting for reasoning models
- Streaming configuration types

Keep pipeline orchestration in `gestura-core`; this crate provides the streaming primitives.

## Modules

- `streaming`      Core streaming types and processing (`StreamChunk`, `start_streaming`, `TokenUsageStatus`)
- `cancellation`   `StreamCancellationRegistry` for cooperative stream cancellation
- `config`         `StreamingConfig` (decoupled from `AppConfig` for portability)

## Stable import paths

Most code should import through the facade:

- `gestura_core::streaming::*`
- `gestura_core::stream_cancellation::*`

The facades in `crates/gestura-core/src/` re-export this crate.

## Development

```bash
cargo test -p gestura-core-streaming
cargo clippy -p gestura-core-streaming --all-targets --all-features -- -D warnings
```

