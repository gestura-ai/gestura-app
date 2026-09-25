# Shared Semantic Protocol — moved

The protocol reference now lives with the protocol crate in the standalone
SDK repository: **<https://github.com/gestura-ai/gestura-sdk/blob/main/docs/PROTOCOL.md>**
(`gestura-sdk/docs/PROTOCOL.md`), alongside the ratified v0.4 intent-layer
design (`docs/ssp-v0.4-intent-layer.md`), the JSON Schemas (`schemas/`), the
golden conformance vectors (`conformance/`) and the manifest registry
(`registry/`). Moved 2026-09-25 with the `gestura-protocol` crate and the
TypeScript SDK (decision 2026-09-24: SDK home = standalone `gestura-sdk`).

This crate consumes `gestura-protocol` from that repository (see the
workspace `Cargo.toml`) and re-exports it as `gestura_core_ring::protocol`,
so every existing import path keeps resolving. Protocol changes are proposed
there, cross-checked with the firmware lane and confirmed by the user before
landing; nothing about the wire is decided in this repository.
