# Code Organization

## Status

This document is now a **high-level contributor map**.

Detailed module and crate reference material is being moved into crate-level and module-level Rustdoc so code organization can be discovered through generated documentation instead of manually synchronized tables.

## Workspace Shape

```text
gestura-app/
├── Cargo.toml
├── crates/
│   ├── gestura-core/            # public facade + cross-domain integration
│   ├── gestura-core-*/          # focused domain crates
│   ├── gestura-cli/             # thin CLI presentation layer
│   └── gestura-gui/             # thin Tauri presentation layer
├── docs/                        # operational docs, RFCs, guides
└── AGENTS.md                    # repository guidance for coding agents
```

## Crate Ownership Model

### Public facade

- `gestura-core`
  - stable public API surface
  - re-exports domain crates under durable import paths
  - owns cross-domain orchestration that does not fit a single domain crate

### Shared primitives

- `gestura-core-foundation`
  - dependency-light shared types such as errors, events, telemetry, platform, execution mode, and related primitives

### Domain crates

Common examples:

- `gestura-core-tools`
- `gestura-core-mcp`
- `gestura-core-llm`
- `gestura-core-pipeline`
- `gestura-core-sessions`
- `gestura-core-config`
- `gestura-core-context`
- `gestura-core-security`
- `gestura-core-streaming`
- `gestura-core-knowledge`
- `gestura-core-memory-bank`
- `gestura-core-agents`
- `gestura-core-tasks`
- `gestura-core-hooks`
- `gestura-core-scripting`
- `gestura-core-plugins`

### Presentation layers

- `gestura-cli`
  - command-line entry points and TUI
- `gestura-gui`
  - Tauri host, platform integrations, frontend bridge, thin re-export wrappers

## How to Navigate the Codebase Now

1. Start with `gestura-core` Rustdoc for stable public entry points.
2. Jump to the owning `gestura-core-*` crate for domain details.
3. Use `gestura-cli` or `gestura-gui` only when the concern is presentation, platform integration, or transport wiring.

## Canonical Generated Docs

```bash
cargo doc --workspace --no-deps
```

This is the intended source of truth for:

- crate responsibilities
- module boundaries
- stable import paths
- public type/function reference

## What Still Belongs in Manual Docs

- build and release workflow documents
- packaging and installer details
- troubleshooting
- RFCs / research notes
- frontend-specific contracts where Rustdoc is not the best fit