//! Execution mode model and policy helpers.
//!
//! This module is a **compatibility facade** that re-exports the shared
//! implementation from `gestura-core-foundation` so downstream code can keep
//! importing `gestura_core::execution_mode::*`.

pub use gestura_core_foundation::execution_mode::*;
