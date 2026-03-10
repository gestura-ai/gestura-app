# Gestura.app API Guide

## Status

This document is now a **routing guide** rather than the canonical API reference.

The Rust library API is being moved into crate-level and module-level Rustdoc so that `cargo doc` becomes the primary source of truth.

## Canonical Rust API Reference

Generate the workspace docs with:

```bash
cargo doc --workspace --no-deps
```

Primary entry points:

- `gestura-core` — stable public facade
- `gestura-core-tools` — tool implementations and policies
- `gestura-core-mcp` — MCP protocol implementation
- `gestura-core-llm` — provider abstractions and implementations
- `gestura-core-config` — typed configuration API
- `gestura-core-security` — secure storage and sandbox helpers

## Which APIs Live Where

### Rust library API

Canonical location: generated Rustdoc from `cargo doc`.

Use `gestura_core::*` as the default public import surface.

### CLI surface

Canonical location:

- `gestura --help`
- `gestura <subcommand> --help`
- generated man pages / completion output

CLI behavior remains user-facing documentation rather than Rustdoc-only API.

### MCP protocol surface

Canonical implementation docs live in Rustdoc for `gestura-core-mcp`, while protocol examples and interoperability notes may remain in manual docs where helpful.

### GUI / Tauri IPC surface

Tauri command and IPC payload details may continue to live in focused manual documents where they are primarily frontend/integration contracts rather than public Rust library APIs.

## What This File No Longer Tries to Do

This file no longer duplicates:

- Rust type-by-type API walkthroughs
- hand-maintained module reference tables
- code examples that are better kept next to the owning crate/module

Those belong in the codebase so they version with the implementation.

## Related Docs

- `README.md` — project overview and documentation strategy
- `docs/ARCHITECTURE.md` — architecture transition guide
- `docs/IPC_CONTRACTS_GESTURA_GUI.md` — GUI-facing IPC contract material
- `docs/command-templates/` — reusable operational command workflows