//! Usage analytics, privacy-aware insights, and personalized recommendations.
//!
//! `gestura-core-analytics` combines two closely related domains:
//!
//! - usage analytics and insight generation
//! - recommendation generation based on observed behavior patterns
//!
//! ## Main entry points
//!
//! - `UsageAnalytics`: event ingestion plus aggregated insights
//! - `UsageEvent`, `EventType`: tracked activity model
//! - `AnalyticsInsights`, `UsagePatterns`, `PerformanceMetrics`, `ErrorAnalysis`:
//!   derived analytics summaries
//! - `AnalyticsConfig`, `PrivacyMode`: privacy-aware analytics behavior
//! - `PersonalizedRecommendationEngine`: recommendation generation and feedback loop
//! - `Recommendation`, `RecommendationType`, `RecommendationFeedback`:
//!   recommendation model and user feedback
//!
//! ## Architecture role
//!
//! This crate provides the analytics and recommendation domain model itself.
//! Product policy around whether analytics are enabled, how consent is gathered,
//! and how recommendations are displayed belongs in higher-level configuration,
//! privacy, and UI layers.
//!
//! ## Stable import paths
//!
//! - Analytics: `gestura_core::analytics::*`
//! - Recommendations: `gestura_core::recommendations::*`

pub mod analytics;
pub mod recommendations;

pub use analytics::*;
pub use recommendations::*;
