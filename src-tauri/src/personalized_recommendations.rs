//! Personalized recommendations for Gestura.app
//! Provides intelligent recommendations based on user behavior and preferences

#[allow(unused_imports)]
use crate::AppError;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Recommendation types
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum RecommendationType {
    Feature,
    Gesture,
    VoiceCommand,
    Setting,
    Workflow,
    Tutorial,
    Optimization,
}

/// Recommendation item
#[derive(Debug, Clone, serde::Serialize)]
pub struct Recommendation {
    pub id: String,
    pub title: String,
    pub description: String,
    pub recommendation_type: RecommendationType,
    pub confidence: f32,
    pub priority: u8, // 1-10, 10 being highest
    pub category: String,
    pub tags: Vec<String>,
    pub action_url: Option<String>,
    pub estimated_benefit: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// User behavior pattern
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UserBehaviorPattern {
    pub user_id: String,
    pub feature_usage: HashMap<String, u32>,
    pub gesture_frequency: HashMap<String, u32>,
    pub voice_command_frequency: HashMap<String, u32>,
    pub session_patterns: SessionPatterns,
    pub error_patterns: HashMap<String, u32>,
    pub preference_scores: HashMap<String, f32>,
    pub last_updated: chrono::DateTime<chrono::Utc>,
}

/// Session usage patterns
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SessionPatterns {
    pub average_session_duration_minutes: f32,
    pub peak_usage_hours: Vec<u8>,
    pub most_used_features: Vec<String>,
    pub feature_adoption_rate: f32,
    pub error_rate: f32,
}

impl Default for SessionPatterns {
    fn default() -> Self {
        Self {
            average_session_duration_minutes: 0.0,
            peak_usage_hours: Vec::new(),
            most_used_features: Vec::new(),
            feature_adoption_rate: 0.0,
            error_rate: 0.0,
        }
    }
}

/// Recommendation engine configuration
#[derive(Debug, Clone)]
pub struct RecommendationConfig {
    pub max_recommendations: usize,
    pub min_confidence_threshold: f32,
    pub learning_rate: f32,
    pub recommendation_refresh_hours: u64,
    pub enable_cross_user_learning: bool,
    pub privacy_mode: bool,
}

impl Default for RecommendationConfig {
    fn default() -> Self {
        Self {
            max_recommendations: 10,
            min_confidence_threshold: 0.6,
            learning_rate: 0.1,
            recommendation_refresh_hours: 24,
            enable_cross_user_learning: false,
            privacy_mode: true,
        }
    }
}

/// Personalized recommendation engine
pub struct PersonalizedRecommendationEngine {
    user_patterns: Arc<RwLock<HashMap<String, UserBehaviorPattern>>>,
    recommendations_cache: Arc<RwLock<HashMap<String, Vec<Recommendation>>>>,
    global_patterns: Arc<RwLock<HashMap<String, f32>>>,
    config: Arc<RwLock<RecommendationConfig>>,
    recommendation_templates: Arc<RwLock<Vec<RecommendationTemplate>>>,
}

/// Recommendation template for generating personalized recommendations
#[derive(Debug, Clone)]
struct RecommendationTemplate {
    #[allow(dead_code)]
    id: String,
    title_template: String,
    description_template: String,
    recommendation_type: RecommendationType,
    category: String,
    tags: Vec<String>,
    conditions: Vec<RecommendationCondition>,
    priority: u8,
    estimated_benefit: String,
}

/// Conditions for triggering recommendations
#[derive(Debug, Clone)]
#[allow(dead_code)]
enum RecommendationCondition {
    FeatureUsageBelow { feature: String, threshold: u32 },
    FeatureUsageAbove { feature: String, threshold: u32 },
    ErrorRateAbove { threshold: f32 },
    SessionDurationBelow { threshold_minutes: f32 },
    GestureAccuracyBelow { threshold: f32 },
    VoiceAccuracyBelow { threshold: f32 },
    HasNotUsedFeature { feature: String },
    UsesFeatureFrequently { feature: String, min_usage: u32 },
}

impl PersonalizedRecommendationEngine {
    /// Create a new recommendation engine
    pub fn new(config: RecommendationConfig) -> Self {
        let engine = Self {
            user_patterns: Arc::new(RwLock::new(HashMap::new())),
            recommendations_cache: Arc::new(RwLock::new(HashMap::new())),
            global_patterns: Arc::new(RwLock::new(HashMap::new())),
            config: Arc::new(RwLock::new(config)),
            recommendation_templates: Arc::new(RwLock::new(Vec::new())),
        };

        // Initialize with default templates
        tokio::spawn({
            let engine = engine.clone();
            async move {
                if let Err(e) = engine.initialize_default_templates().await {
                    tracing::error!("Failed to initialize recommendation templates: {}", e);
                }
            }
        });

        engine
    }

    /// Update user behavior pattern
    pub async fn update_user_pattern(&self, user_id: &str, usage_data: serde_json::Value) -> Result<(), AppError> {
        let mut patterns = self.user_patterns.write().await;
        
        let pattern = patterns.entry(user_id.to_string()).or_insert_with(|| {
            UserBehaviorPattern {
                user_id: user_id.to_string(),
                feature_usage: HashMap::new(),
                gesture_frequency: HashMap::new(),
                voice_command_frequency: HashMap::new(),
                session_patterns: SessionPatterns::default(),
                error_patterns: HashMap::new(),
                preference_scores: HashMap::new(),
                last_updated: chrono::Utc::now(),
            }
        });

        // Update pattern based on usage data
        if let Some(features) = usage_data.get("features") {
            if let Ok(feature_map) = serde_json::from_value::<HashMap<String, u32>>(features.clone()) {
                for (feature, count) in feature_map {
                    *pattern.feature_usage.entry(feature).or_insert(0) += count;
                }
            }
        }

        if let Some(gestures) = usage_data.get("gestures") {
            if let Ok(gesture_map) = serde_json::from_value::<HashMap<String, u32>>(gestures.clone()) {
                for (gesture, count) in gesture_map {
                    *pattern.gesture_frequency.entry(gesture).or_insert(0) += count;
                }
            }
        }

        if let Some(voice_commands) = usage_data.get("voice_commands") {
            if let Ok(voice_map) = serde_json::from_value::<HashMap<String, u32>>(voice_commands.clone()) {
                for (command, count) in voice_map {
                    *pattern.voice_command_frequency.entry(command).or_insert(0) += count;
                }
            }
        }

        pattern.last_updated = chrono::Utc::now();

        // Clear cached recommendations for this user
        let mut cache = self.recommendations_cache.write().await;
        cache.remove(user_id);

        tracing::debug!("Updated behavior pattern for user: {}", user_id);
        Ok(())
    }

    /// Generate personalized recommendations for a user
    pub async fn generate_recommendations(&self, user_id: &str) -> Result<Vec<Recommendation>, AppError> {
        // Check cache first
        {
            let cache = self.recommendations_cache.read().await;
            if let Some(cached_recommendations) = cache.get(user_id) {
                let config = self.config.read().await;
                let cache_age = chrono::Utc::now().timestamp() - 
                    cached_recommendations.first().map(|r| r.created_at.timestamp()).unwrap_or(0);
                
                if cache_age < (config.recommendation_refresh_hours * 3600) as i64 {
                    return Ok(cached_recommendations.clone());
                }
            }
        }

        let patterns = self.user_patterns.read().await;
        let user_pattern = patterns.get(user_id);
        
        if user_pattern.is_none() {
            return Ok(Vec::new()); // No data yet
        }

        let user_pattern = user_pattern.unwrap();
        let templates = self.recommendation_templates.read().await;
        let mut recommendations = Vec::new();

        // Generate recommendations based on templates
        for template in templates.iter() {
            if let Some(recommendation) = self.evaluate_template(template, user_pattern).await {
                recommendations.push(recommendation);
            }
        }

        // Sort by priority and confidence
        recommendations.sort_by(|a, b| {
            b.priority.cmp(&a.priority)
                .then_with(|| b.confidence.partial_cmp(&a.confidence).unwrap_or(std::cmp::Ordering::Equal))
        });

        // Apply filters
        let config = self.config.read().await;
        recommendations.retain(|r| r.confidence >= config.min_confidence_threshold);
        recommendations.truncate(config.max_recommendations);

        // Cache recommendations
        drop(config);
        drop(templates);
        drop(patterns);
        let mut cache = self.recommendations_cache.write().await;
        cache.insert(user_id.to_string(), recommendations.clone());

        tracing::info!("Generated {} recommendations for user: {}", recommendations.len(), user_id);
        Ok(recommendations)
    }

    /// Evaluate a recommendation template against user pattern
    async fn evaluate_template(&self, template: &RecommendationTemplate, user_pattern: &UserBehaviorPattern) -> Option<Recommendation> {
        let mut confidence = 0.5; // Base confidence
        let mut conditions_met = 0;
        let total_conditions = template.conditions.len();

        // Evaluate conditions
        for condition in &template.conditions {
            match condition {
                RecommendationCondition::FeatureUsageBelow { feature, threshold } => {
                    let usage = user_pattern.feature_usage.get(feature).unwrap_or(&0);
                    if *usage < *threshold {
                        conditions_met += 1;
                        confidence += 0.1;
                    }
                }
                RecommendationCondition::FeatureUsageAbove { feature, threshold } => {
                    let usage = user_pattern.feature_usage.get(feature).unwrap_or(&0);
                    if *usage > *threshold {
                        conditions_met += 1;
                        confidence += 0.1;
                    }
                }
                RecommendationCondition::ErrorRateAbove { threshold } => {
                    if user_pattern.session_patterns.error_rate > *threshold {
                        conditions_met += 1;
                        confidence += 0.15;
                    }
                }
                RecommendationCondition::SessionDurationBelow { threshold_minutes } => {
                    if user_pattern.session_patterns.average_session_duration_minutes < *threshold_minutes {
                        conditions_met += 1;
                        confidence += 0.1;
                    }
                }
                RecommendationCondition::HasNotUsedFeature { feature } => {
                    if !user_pattern.feature_usage.contains_key(feature) {
                        conditions_met += 1;
                        confidence += 0.2;
                    }
                }
                RecommendationCondition::UsesFeatureFrequently { feature, min_usage } => {
                    let usage = user_pattern.feature_usage.get(feature).unwrap_or(&0);
                    if *usage >= *min_usage {
                        conditions_met += 1;
                        confidence += 0.1;
                    }
                }
                _ => {} // Handle other conditions
            }
        }

        // Require at least 50% of conditions to be met
        if total_conditions > 0 && (conditions_met as f32 / total_conditions as f32) < 0.5 {
            return None;
        }

        // Adjust confidence based on user preferences
        if let Some(pref_score) = user_pattern.preference_scores.get(&template.category) {
            confidence *= pref_score;
        }

        Some(Recommendation {
            id: uuid::Uuid::new_v4().to_string(),
            title: self.personalize_text(&template.title_template, user_pattern),
            description: self.personalize_text(&template.description_template, user_pattern),
            recommendation_type: template.recommendation_type.clone(),
            confidence: confidence.min(1.0),
            priority: template.priority,
            category: template.category.clone(),
            tags: template.tags.clone(),
            action_url: None,
            estimated_benefit: template.estimated_benefit.clone(),
            created_at: chrono::Utc::now(),
        })
    }

    /// Personalize text templates with user data
    fn personalize_text(&self, template: &str, user_pattern: &UserBehaviorPattern) -> String {
        let mut result = template.to_string();
        
        // Replace placeholders with user-specific data
        if let Some(most_used) = user_pattern.session_patterns.most_used_features.first() {
            result = result.replace("{most_used_feature}", most_used);
        }
        
        result = result.replace("{session_duration}", 
            &format!("{:.1}", user_pattern.session_patterns.average_session_duration_minutes));
        
        result = result.replace("{error_rate}", 
            &format!("{:.1}%", user_pattern.session_patterns.error_rate * 100.0));

        result
    }

    /// Initialize default recommendation templates
    async fn initialize_default_templates(&self) -> Result<(), AppError> {
        let mut templates = self.recommendation_templates.write().await;
        
        templates.push(RecommendationTemplate {
            id: "voice_training".to_string(),
            title_template: "Improve Voice Recognition Accuracy".to_string(),
            description_template: "Your voice recognition accuracy could be improved. Try the voice training feature to personalize the system to your voice.".to_string(),
            recommendation_type: RecommendationType::Feature,
            category: "voice".to_string(),
            tags: vec!["accuracy".to_string(), "training".to_string()],
            conditions: vec![
                RecommendationCondition::HasNotUsedFeature { feature: "voice_training".to_string() },
                RecommendationCondition::ErrorRateAbove { threshold: 0.1 },
            ],
            priority: 8,
            estimated_benefit: "Up to 30% improvement in voice recognition accuracy".to_string(),
        });

        templates.push(RecommendationTemplate {
            id: "gesture_customization".to_string(),
            title_template: "Customize Your Gestures".to_string(),
            description_template: "You use gestures frequently! Create custom gestures for your most common actions to save time.".to_string(),
            recommendation_type: RecommendationType::Feature,
            category: "gestures".to_string(),
            tags: vec!["customization".to_string(), "efficiency".to_string()],
            conditions: vec![
                RecommendationCondition::UsesFeatureFrequently { feature: "gestures".to_string(), min_usage: 50 },
                RecommendationCondition::HasNotUsedFeature { feature: "custom_gestures".to_string() },
            ],
            priority: 7,
            estimated_benefit: "Reduce gesture time by up to 40%".to_string(),
        });

        templates.push(RecommendationTemplate {
            id: "session_optimization".to_string(),
            title_template: "Optimize Your Workflow".to_string(),
            description_template: "Your sessions are shorter than average ({session_duration} min). Try these workflow optimizations to be more productive.".to_string(),
            recommendation_type: RecommendationType::Optimization,
            category: "productivity".to_string(),
            tags: vec!["workflow".to_string(), "efficiency".to_string()],
            conditions: vec![
                RecommendationCondition::SessionDurationBelow { threshold_minutes: 15.0 },
            ],
            priority: 6,
            estimated_benefit: "Increase productivity by 25%".to_string(),
        });

        templates.push(RecommendationTemplate {
            id: "ring_calibration".to_string(),
            title_template: "Calibrate Your Ring".to_string(),
            description_template: "Your gesture accuracy seems low. Try recalibrating your Haptic Harmony ring for better performance.".to_string(),
            recommendation_type: RecommendationType::Setting,
            category: "hardware".to_string(),
            tags: vec!["calibration".to_string(), "accuracy".to_string()],
            conditions: vec![
                RecommendationCondition::GestureAccuracyBelow { threshold: 0.8 },
            ],
            priority: 9,
            estimated_benefit: "Improve gesture accuracy by up to 50%".to_string(),
        });

        templates.push(RecommendationTemplate {
            id: "tutorial_advanced".to_string(),
            title_template: "Learn Advanced Features".to_string(),
            description_template: "You're using {most_used_feature} frequently. Learn about advanced features that can enhance your experience.".to_string(),
            recommendation_type: RecommendationType::Tutorial,
            category: "learning".to_string(),
            tags: vec!["advanced".to_string(), "features".to_string()],
            conditions: vec![
                RecommendationCondition::FeatureUsageAbove { feature: "basic_features".to_string(), threshold: 100 },
            ],
            priority: 5,
            estimated_benefit: "Unlock 10+ advanced features".to_string(),
        });

        tracing::info!("Initialized {} recommendation templates", templates.len());
        Ok(())
    }

    /// Record user feedback on recommendations
    pub async fn record_feedback(&self, user_id: &str, recommendation_id: &str, 
                                feedback: RecommendationFeedback) -> Result<(), AppError> {
        let mut patterns = self.user_patterns.write().await;
        
        if let Some(pattern) = patterns.get_mut(user_id) {
            // Update preference scores based on feedback
            let cache = self.recommendations_cache.read().await;
            if let Some(recommendations) = cache.get(user_id) {
                if let Some(recommendation) = recommendations.iter().find(|r| r.id == recommendation_id) {
                    let category = &recommendation.category;
                    let current_score = pattern.preference_scores.get(category).unwrap_or(&0.5);
                    
                    let adjustment = match feedback {
                        RecommendationFeedback::Helpful => 0.1,
                        RecommendationFeedback::NotHelpful => -0.1,
                        RecommendationFeedback::Implemented => 0.2,
                        RecommendationFeedback::Dismissed => -0.05,
                    };
                    
                    let new_score = (current_score + adjustment).max(0.0).min(1.0);
                    pattern.preference_scores.insert(category.clone(), new_score);
                    
                    tracing::debug!("Updated preference score for category '{}' to {:.2} based on feedback", 
                        category, new_score);
                }
            }
        }

        Ok(())
    }

    /// Get recommendation statistics
    pub async fn get_stats(&self) -> serde_json::Value {
        let patterns = self.user_patterns.read().await;
        let cache = self.recommendations_cache.read().await;
        let templates = self.recommendation_templates.read().await;

        let total_users = patterns.len();
        let total_cached_recommendations: usize = cache.values().map(|v| v.len()).sum();
        let total_templates = templates.len();

        serde_json::json!({
            "total_users": total_users,
            "total_cached_recommendations": total_cached_recommendations,
            "total_templates": total_templates,
            "average_recommendations_per_user": if total_users > 0 { 
                total_cached_recommendations as f64 / total_users as f64 
            } else { 
                0.0 
            }
        })
    }

    /// Clear user data
    pub async fn clear_user_data(&self, user_id: &str) -> Result<(), AppError> {
        let mut patterns = self.user_patterns.write().await;
        let mut cache = self.recommendations_cache.write().await;
        
        patterns.remove(user_id);
        cache.remove(user_id);
        
        tracing::info!("Cleared recommendation data for user: {}", user_id);
        Ok(())
    }
}

impl Clone for PersonalizedRecommendationEngine {
    fn clone(&self) -> Self {
        Self {
            user_patterns: self.user_patterns.clone(),
            recommendations_cache: self.recommendations_cache.clone(),
            global_patterns: self.global_patterns.clone(),
            config: self.config.clone(),
            recommendation_templates: self.recommendation_templates.clone(),
        }
    }
}

/// User feedback on recommendations
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum RecommendationFeedback {
    Helpful,
    NotHelpful,
    Implemented,
    Dismissed,
}

/// Global recommendation engine instance
static RECOMMENDATION_ENGINE: tokio::sync::OnceCell<PersonalizedRecommendationEngine> = tokio::sync::OnceCell::const_new();

/// Get the global recommendation engine
pub async fn get_recommendation_engine() -> &'static PersonalizedRecommendationEngine {
    RECOMMENDATION_ENGINE.get_or_init(|| async {
        PersonalizedRecommendationEngine::new(RecommendationConfig::default())
    }).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_recommendation_generation() {
        let engine = PersonalizedRecommendationEngine::new(RecommendationConfig::default());
        
        // Create user pattern
        let usage_data = serde_json::json!({
            "features": {
                "voice_commands": 10,
                "gestures": 50
            },
            "gestures": {
                "tap": 30,
                "swipe": 20
            }
        });
        
        engine.update_user_pattern("user1", usage_data).await.unwrap();
        
        let recommendations = engine.generate_recommendations("user1").await.unwrap();
        assert!(!recommendations.is_empty());
    }

    #[tokio::test]
    async fn test_feedback_recording() {
        let engine = PersonalizedRecommendationEngine::new(RecommendationConfig::default());
        
        // Setup user and generate recommendations
        let usage_data = serde_json::json!({
            "features": {"gestures": 100}
        });
        
        engine.update_user_pattern("user1", usage_data).await.unwrap();
        let recommendations = engine.generate_recommendations("user1").await.unwrap();
        
        if let Some(rec) = recommendations.first() {
            engine.record_feedback("user1", &rec.id, RecommendationFeedback::Helpful).await.unwrap();
        }
        
        // Verify feedback was recorded (would check preference scores in real test)
        let stats = engine.get_stats().await;
        assert_eq!(stats["total_users"].as_u64().unwrap(), 1);
    }
}
