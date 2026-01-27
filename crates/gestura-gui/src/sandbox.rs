//! Agent sandboxing and isolation utilities - thin wrapper over gestura_core::sandbox
//!
//! This module provides re-exports from gestura-core's sandbox module.
//! All sandbox configuration and validation logic lives in core.

// Re-export core sandbox types
pub use gestura_core::sandbox::{SandboxConfig, SandboxManager, create_default_sandbox};
