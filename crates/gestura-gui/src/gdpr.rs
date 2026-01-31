//! GDPR module shim for `gestura-gui`.
//!
//! Core-first architecture rule: GDPR business logic lives in `gestura-core`.
//! The GUI crate only re-exports the core API to preserve stable import paths
//! (e.g. `crate::gdpr::get_gdpr_manager()` from GUI code).

pub use gestura_core::gdpr::*;
