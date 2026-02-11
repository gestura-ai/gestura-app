# gestura-core-foundation

Shared, dependency-light primitives used across Gestura core domain crates.

## What belongs here

- Cross-cutting **models** and **policy primitives** used by multiple domains
- Shared error/types used by public facades

Keep this crate small: no GUI/CLI concerns, no tool implementations, and no protocol adapters.

## Modules

- `error` 			Shared `AppError` and `Result`
- `permissions` 	Shared permission primitives
- `execution_mode` 	Execution mode model + tool-category permission defaults

## Stable import paths

Most downstream code should continue to import through the facade:

- `gestura_core::execution_mode::*`

Domain crates may depend on foundation directly when appropriate.

## Development

```bash
cargo test -p gestura-core-foundation
cargo clippy -p gestura-core-foundation --all-targets --all-features -- -D warnings
```
