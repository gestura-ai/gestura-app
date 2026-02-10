//! Application error types for Gestura Core.
//!
//! This module is a compatibility wrapper around `gestura-core-foundation`.
//!
//! Keeping `gestura_core::error::{AppError, Result}` stable allows CLI/GUI and
//! internal modules to migrate incrementally while domain crates are extracted.

pub use gestura_core_foundation::error::{AppError, Result};
