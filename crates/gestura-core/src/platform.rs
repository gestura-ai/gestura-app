//! Platform detection utilities.
//!
//! This module is a **compatibility facade** that re-exports the shared
//! implementation from `gestura-core-foundation` so downstream code can keep
//! importing `gestura_core::platform::*`.

pub use gestura_core_foundation::platform::*;
