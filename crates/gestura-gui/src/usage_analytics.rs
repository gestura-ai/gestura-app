//! Usage analytics and insights - thin wrapper over gestura_core::analytics
//!
//! All analytics logic lives in gestura-core. This module re-exports the
//! core types for GUI usage.

pub use gestura_core::analytics::{
    AnalyticsConfig, AnalyticsInsights, ErrorAnalysis, EventType, PerformanceMetrics, PrivacyMode,
    TimePeriod, UsageAnalytics, UsageEvent, UsagePatterns,
};
