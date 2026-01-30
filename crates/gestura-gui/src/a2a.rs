//! A2A protocol re-exports for the GUI crate.
//!
//! Gestura uses a *core-first* architecture: protocol types and business logic live in
//! `gestura-core`. The GUI crate keeps this module as a thin adapter to preserve internal
//! module paths (`crate::a2a::*`) while delegating implementation to core.

pub use gestura_core::a2a::*;
