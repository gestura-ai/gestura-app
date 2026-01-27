//! Personalized recommendations - thin wrapper over gestura_core::recommendations
//!
//! All recommendation logic lives in gestura-core. This module re-exports the
//! core types for GUI usage.

pub use gestura_core::recommendations::{
    PersonalizedRecommendationEngine, Recommendation, RecommendationConfig, RecommendationFeedback,
    RecommendationType, SessionPatterns, UserBehaviorPattern,
};
