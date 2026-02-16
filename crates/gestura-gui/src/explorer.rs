//! Workspace file explorer — thin re-export from core.
//!
//! All business logic lives in [`gestura_core::explorer`].  This module
//! re-exports everything so that existing `crate::explorer::*` imports in the
//! GUI crate continue to resolve.

pub use gestura_core::explorer::*;
