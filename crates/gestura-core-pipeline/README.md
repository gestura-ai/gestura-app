# gestura-core-pipeline

Pipeline types and persona for the Gestura agent pipeline.

## What belongs here

- Pipeline shared types (`AgentRequest`, `AgentResponse`, `PipelineStage`, etc.)
- Default system prompt / persona definition
- Types used across pipeline sub-modules

The pipeline **execution logic** (agent loop, tool dispatch, compaction, prompt building) remains in `gestura-core/src/pipeline/` because it orchestrates across multiple domain crates.

## Modules

- `types`    Shared pipeline types (`AgentRequest`, `AgentResponse`, `PipelineStage`, etc.)
- `persona`  Default system prompt and persona definition

## Stable import paths

Most code should import through the facade:

- `gestura_core::pipeline::*`

Pipeline types are re-exported from `crates/gestura-core/src/pipeline/types.rs`.

## Development

```bash
cargo test -p gestura-core-pipeline
cargo clippy -p gestura-core-pipeline --all-targets --all-features -- -D warnings
```

