//! Usage analytics, insights, and personalized recommendations for Gestura.
//!
//! This crate merges the `analytics` and `recommendations` domains into a
//! single analytics crate covering usage tracking, pattern analysis, and
//! personalised recommendation generation.
//!
//! ## Stable import paths
//!
//! - Analytics: `gestura_core::analytics::*`
//! - Recommendations: `gestura_core::recommendations::*`

pub mod analytics;
pub mod recommendations;

pub use analytics::*;
pub use recommendations::*;
