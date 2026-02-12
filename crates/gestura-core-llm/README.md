# gestura-core-llm

LLM provider implementations and model definitions for Gestura.

Feature-gated to keep compile times fast when only a subset of providers is needed.

## What belongs here

- LLM provider trait and implementations (Anthropic, OpenAI, Google, etc.)
- Token usage types and cost tracking
- Default model definitions and aliases
- Provider-specific request/response types

Keep pipeline orchestration and tool dispatch in `gestura-core`; this crate provides the raw LLM interface.

## Modules

- `default_models`   Built-in model aliases and default configurations
- Provider implementations (feature-gated)

## Stable import paths

Most code should import through the facade:

- `gestura_core::llm_provider::*`
- `gestura_core::default_models::*`

The facade in `crates/gestura-core/src/llm_provider.rs` re-exports this crate.

## Development

```bash
cargo test -p gestura-core-llm
cargo clippy -p gestura-core-llm --all-targets --all-features -- -D warnings
```

