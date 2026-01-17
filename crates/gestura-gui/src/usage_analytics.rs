//! Usage analytics and insights for Gestura.app
//! Collects and analyzes user behavior patterns while respecting privacy

#[allow(unused_imports)]
use crate::AppError;
use chrono::Timelike;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Usage event types
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum EventType {
    AppLaunch,
    AppClose,
    VoiceCommand,
    GesturePerformed,
    RingConnected,
    RingDisconnected,
    SettingsChanged,
    ErrorOccurred,
    FeatureUsed(String),
    Custom(String),
}

/// Usage event
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UsageEvent {
    pub event_id: String,
    pub event_type: EventType,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub user_id: Option<String>,
    pub session_id: String,
    pub properties: HashMap<String, serde_json::Value>,
    pub duration_ms: Option<u64>,
}

/// Analytics insights
#[derive(Debug, Clone, serde::Serialize)]
pub struct AnalyticsInsights {
    pub total_events: usize,
    pub unique_users: usize,
    pub active_sessions: usize,
    pub most_used_features: Vec<(String, usize)>,
    pub usage_patterns: UsagePatterns,
    pub performance_metrics: PerformanceMetrics,
    pub error_analysis: ErrorAnalysis,
    pub time_period: TimePeriod,
}

/// Usage patterns analysis
#[derive(Debug, Clone, serde::Serialize)]
pub struct UsagePatterns {
    pub peak_usage_hours: Vec<u8>, // Hours of day (0-23)
    pub average_session_duration_minutes: f64,
    pub most_common_gestures: Vec<(String, usize)>,
    pub voice_command_frequency: f64, // Commands per session
    pub feature_adoption_rate: HashMap<String, f64>, // Feature -> adoption rate
}

/// Performance metrics
#[derive(Debug, Clone, serde::Serialize)]
pub struct PerformanceMetrics {
    pub average_response_time_ms: f64,
    pub gesture_recognition_accuracy: f64,
    pub voice_recognition_accuracy: f64,
    pub system_stability_score: f64, // 0-1 scale
}

/// Error analysis
#[derive(Debug, Clone, serde::Serialize)]
pub struct ErrorAnalysis {
    pub total_errors: usize,
    pub error_rate: f64, // Errors per session
    pub most_common_errors: Vec<(String, usize)>,
    pub error_trends: Vec<(chrono::DateTime<chrono::Utc>, usize)>,
}

/// Time period for analysis
#[derive(Debug, Clone, serde::Serialize)]
pub struct TimePeriod {
    pub start: chrono::DateTime<chrono::Utc>,
    pub end: chrono::DateTime<chrono::Utc>,
    pub duration_days: i64,
}

/// Analytics configuration
#[derive(Debug, Clone)]
pub struct AnalyticsConfig {
    pub enable_collection: bool,
    pub anonymize_data: bool,
    pub retention_days: i64,
    pub batch_size: usize,
    pub flush_interval_seconds: u64,
    pub privacy_mode: PrivacyMode,
}

/// Privacy modes for analytics
#[derive(Debug, Clone, PartialEq)]
pub enum PrivacyMode {
    Full,      // Collect all data
    Limited,   // Collect only essential metrics
    Anonymous, // Collect data without user identification
    Disabled,  // No data collection
}

impl Default for AnalyticsConfig {
    fn default() -> Self {
        Self {
            enable_collection: true,
            anonymize_data: true,
            retention_days: 30,
            batch_size: 100,
            flush_interval_seconds: 300, // 5 minutes
            privacy_mode: PrivacyMode::Anonymous,
        }
    }
}

/// Usage analytics system
pub struct UsageAnalytics {
    events: Arc<RwLock<Vec<UsageEvent>>>,
    config: Arc<RwLock<AnalyticsConfig>>,
    session_cache: Arc<RwLock<HashMap<String, SessionInfo>>>,
    insights_cache: Arc<RwLock<Option<AnalyticsInsights>>>,
    last_flush: Arc<RwLock<chrono::DateTime<chrono::Utc>>>,
}

/// Session information
#[derive(Debug, Clone)]
struct SessionInfo {
    #[allow(dead_code)]
    session_id: String,
    #[allow(dead_code)]
    user_id: Option<String>,
    start_time: chrono::DateTime<chrono::Utc>,
    last_activity: chrono::DateTime<chrono::Utc>,
    event_count: usize,
}

impl UsageAnalytics {
    /// Create a new usage analytics system
    pub fn new(config: AnalyticsConfig) -> Self {
        Self {
            events: Arc::new(RwLock::new(Vec::new())),
            config: Arc::new(RwLock::new(config)),
            session_cache: Arc::new(RwLock::new(HashMap::new())),
            insights_cache: Arc::new(RwLock::new(None)),
            last_flush: Arc::new(RwLock::new(chrono::Utc::now())),
        }
    }

    /// Track a usage event
    pub async fn track_event(&self, mut event: UsageEvent) -> Result<(), AppError> {
        let config = self.config.read().await;

        if !config.enable_collection || config.privacy_mode == PrivacyMode::Disabled {
            return Ok(());
        }

        // Apply privacy settings
        if config.anonymize_data || config.privacy_mode == PrivacyMode::Anonymous {
            event.user_id = None;
            // Remove potentially identifying properties
            event.properties.remove("device_id");
            event.properties.remove("ip_address");
            event.properties.remove("user_agent");
        }

        // Update session info
        self.update_session_info(&event).await;

        // Store event
        let mut events = self.events.write().await;
        events.push(event);

        // Check if we need to flush
        if events.len() >= config.batch_size {
            drop(events);
            drop(config);
            self.flush_events().await?;
        }

        Ok(())
    }

    /// Track app launch
    pub async fn track_app_launch(
        &self,
        session_id: String,
        user_id: Option<String>,
    ) -> Result<(), AppError> {
        let event = UsageEvent {
            event_id: uuid::Uuid::new_v4().to_string(),
            event_type: EventType::AppLaunch,
            timestamp: chrono::Utc::now(),
            user_id,
            session_id,
            properties: HashMap::from([
                (
                    "version".to_string(),
                    serde_json::Value::String("1.0.0".to_string()),
                ),
                (
                    "platform".to_string(),
                    serde_json::Value::String(std::env::consts::OS.to_string()),
                ),
            ]),
            duration_ms: None,
        };

        self.track_event(event).await
    }

    /// Track voice command
    pub async fn track_voice_command(
        &self,
        session_id: String,
        user_id: Option<String>,
        command: &str,
        confidence: f32,
        processing_time_ms: u64,
    ) -> Result<(), AppError> {
        let event = UsageEvent {
            event_id: uuid::Uuid::new_v4().to_string(),
            event_type: EventType::VoiceCommand,
            timestamp: chrono::Utc::now(),
            user_id,
            session_id,
            properties: HashMap::from([
                (
                    "command_length".to_string(),
                    serde_json::Value::Number(serde_json::Number::from(command.len())),
                ),
                (
                    "confidence".to_string(),
                    serde_json::Value::Number(
                        serde_json::Number::from_f64(confidence as f64).unwrap(),
                    ),
                ),
                (
                    "processing_time_ms".to_string(),
                    serde_json::Value::Number(serde_json::Number::from(processing_time_ms)),
                ),
            ]),
            duration_ms: Some(processing_time_ms),
        };

        self.track_event(event).await
    }

    /// Track gesture
    pub async fn track_gesture(
        &self,
        session_id: String,
        user_id: Option<String>,
        gesture_type: &str,
        confidence: f32,
    ) -> Result<(), AppError> {
        let event = UsageEvent {
            event_id: uuid::Uuid::new_v4().to_string(),
            event_type: EventType::GesturePerformed,
            timestamp: chrono::Utc::now(),
            user_id,
            session_id,
            properties: HashMap::from([
                (
                    "gesture_type".to_string(),
                    serde_json::Value::String(gesture_type.to_string()),
                ),
                (
                    "confidence".to_string(),
                    serde_json::Value::Number(
                        serde_json::Number::from_f64(confidence as f64).unwrap(),
                    ),
                ),
            ]),
            duration_ms: None,
        };

        self.track_event(event).await
    }

    /// Track error
    pub async fn track_error(
        &self,
        session_id: String,
        user_id: Option<String>,
        error_type: &str,
        error_message: &str,
    ) -> Result<(), AppError> {
        let event = UsageEvent {
            event_id: uuid::Uuid::new_v4().to_string(),
            event_type: EventType::ErrorOccurred,
            timestamp: chrono::Utc::now(),
            user_id,
            session_id,
            properties: HashMap::from([
                (
                    "error_type".to_string(),
                    serde_json::Value::String(error_type.to_string()),
                ),
                (
                    "error_message".to_string(),
                    serde_json::Value::String(error_message.to_string()),
                ),
            ]),
            duration_ms: None,
        };

        self.track_event(event).await
    }

    /// Generate analytics insights
    pub async fn generate_insights(
        &self,
        days_back: Option<i64>,
    ) -> Result<AnalyticsInsights, AppError> {
        let days = days_back.unwrap_or(7);
        let start_time = chrono::Utc::now() - chrono::Duration::days(days);
        let end_time = chrono::Utc::now();

        let events = self.events.read().await;
        let filtered_events: Vec<&UsageEvent> = events
            .iter()
            .filter(|e| e.timestamp >= start_time && e.timestamp <= end_time)
            .collect();

        if filtered_events.is_empty() {
            return Ok(AnalyticsInsights {
                total_events: 0,
                unique_users: 0,
                active_sessions: 0,
                most_used_features: Vec::new(),
                usage_patterns: UsagePatterns {
                    peak_usage_hours: Vec::new(),
                    average_session_duration_minutes: 0.0,
                    most_common_gestures: Vec::new(),
                    voice_command_frequency: 0.0,
                    feature_adoption_rate: HashMap::new(),
                },
                performance_metrics: PerformanceMetrics {
                    average_response_time_ms: 0.0,
                    gesture_recognition_accuracy: 0.0,
                    voice_recognition_accuracy: 0.0,
                    system_stability_score: 1.0,
                },
                error_analysis: ErrorAnalysis {
                    total_errors: 0,
                    error_rate: 0.0,
                    most_common_errors: Vec::new(),
                    error_trends: Vec::new(),
                },
                time_period: TimePeriod {
                    start: start_time,
                    end: end_time,
                    duration_days: days,
                },
            });
        }

        // Basic metrics
        let total_events = filtered_events.len();
        let unique_users = filtered_events
            .iter()
            .filter_map(|e| e.user_id.as_ref())
            .collect::<std::collections::HashSet<_>>()
            .len();
        let unique_sessions = filtered_events
            .iter()
            .map(|e| &e.session_id)
            .collect::<std::collections::HashSet<_>>()
            .len();

        // Feature usage analysis
        let mut feature_counts = HashMap::new();
        for event in &filtered_events {
            let feature_name = match &event.event_type {
                EventType::VoiceCommand => "voice_commands",
                EventType::GesturePerformed => "gestures",
                EventType::RingConnected => "ring_connection",
                EventType::SettingsChanged => "settings",
                EventType::FeatureUsed(name) => name,
                _ => "other",
            };
            *feature_counts.entry(feature_name.to_string()).or_insert(0) += 1;
        }

        let mut most_used_features: Vec<(String, usize)> = feature_counts.into_iter().collect();
        most_used_features.sort_by(|a, b| b.1.cmp(&a.1));
        most_used_features.truncate(10);

        // Usage patterns
        let usage_patterns = self.analyze_usage_patterns(&filtered_events).await;

        // Performance metrics
        let performance_metrics = self.analyze_performance(&filtered_events).await;

        // Error analysis
        let error_analysis = self.analyze_errors(&filtered_events).await;

        let insights = AnalyticsInsights {
            total_events,
            unique_users,
            active_sessions: unique_sessions,
            most_used_features,
            usage_patterns,
            performance_metrics,
            error_analysis,
            time_period: TimePeriod {
                start: start_time,
                end: end_time,
                duration_days: days,
            },
        };

        // Cache insights
        let mut cache = self.insights_cache.write().await;
        *cache = Some(insights.clone());

        Ok(insights)
    }

    /// Analyze usage patterns
    async fn analyze_usage_patterns(&self, events: &[&UsageEvent]) -> UsagePatterns {
        // Peak usage hours
        let mut hour_counts = BTreeMap::new();
        for event in events {
            let hour = event.timestamp.hour() as u8;
            *hour_counts.entry(hour).or_insert(0) += 1;
        }

        let mut peak_hours: Vec<(u8, usize)> = hour_counts.into_iter().collect();
        peak_hours.sort_by(|a, b| b.1.cmp(&a.1));
        let peak_usage_hours = peak_hours.into_iter().take(3).map(|(h, _)| h).collect();

        // Session duration analysis
        let sessions = self.session_cache.read().await;
        let avg_duration = if !sessions.is_empty() {
            let total_duration: i64 = sessions
                .values()
                .map(|s| (s.last_activity - s.start_time).num_minutes())
                .sum();
            total_duration as f64 / sessions.len() as f64
        } else {
            0.0
        };

        // Gesture analysis
        let mut gesture_counts = HashMap::new();
        for event in events {
            if let EventType::GesturePerformed = event.event_type
                && let Some(gesture_type) = event.properties.get("gesture_type")
                && let Some(gesture_str) = gesture_type.as_str()
            {
                *gesture_counts.entry(gesture_str.to_string()).or_insert(0) += 1;
            }
        }

        let mut most_common_gestures: Vec<(String, usize)> = gesture_counts.into_iter().collect();
        most_common_gestures.sort_by(|a, b| b.1.cmp(&a.1));
        most_common_gestures.truncate(5);

        // Voice command frequency
        let voice_commands = events
            .iter()
            .filter(|e| matches!(e.event_type, EventType::VoiceCommand))
            .count();
        let unique_sessions = events
            .iter()
            .map(|e| &e.session_id)
            .collect::<std::collections::HashSet<_>>()
            .len();
        let voice_command_frequency = if unique_sessions > 0 {
            voice_commands as f64 / unique_sessions as f64
        } else {
            0.0
        };

        UsagePatterns {
            peak_usage_hours,
            average_session_duration_minutes: avg_duration,
            most_common_gestures,
            voice_command_frequency,
            feature_adoption_rate: HashMap::new(), // Would be calculated based on user cohorts
        }
    }

    /// Analyze performance metrics
    async fn analyze_performance(&self, events: &[&UsageEvent]) -> PerformanceMetrics {
        let mut response_times = Vec::new();
        let mut gesture_confidences = Vec::new();
        let mut voice_confidences = Vec::new();

        for event in events {
            if let Some(duration) = event.duration_ms {
                response_times.push(duration as f64);
            }

            if let Some(confidence) = event.properties.get("confidence")
                && let Some(conf_val) = confidence.as_f64()
            {
                match event.event_type {
                    EventType::GesturePerformed => gesture_confidences.push(conf_val),
                    EventType::VoiceCommand => voice_confidences.push(conf_val),
                    _ => {}
                }
            }
        }

        let avg_response_time = if !response_times.is_empty() {
            response_times.iter().sum::<f64>() / response_times.len() as f64
        } else {
            0.0
        };

        let gesture_accuracy = if !gesture_confidences.is_empty() {
            gesture_confidences.iter().sum::<f64>() / gesture_confidences.len() as f64
        } else {
            0.0
        };

        let voice_accuracy = if !voice_confidences.is_empty() {
            voice_confidences.iter().sum::<f64>() / voice_confidences.len() as f64
        } else {
            0.0
        };

        // System stability (1 - error_rate)
        let error_count = events
            .iter()
            .filter(|e| matches!(e.event_type, EventType::ErrorOccurred))
            .count();
        let stability_score = if !events.is_empty() {
            1.0 - (error_count as f64 / events.len() as f64)
        } else {
            1.0
        };

        PerformanceMetrics {
            average_response_time_ms: avg_response_time,
            gesture_recognition_accuracy: gesture_accuracy,
            voice_recognition_accuracy: voice_accuracy,
            system_stability_score: stability_score,
        }
    }

    /// Analyze errors
    async fn analyze_errors(&self, events: &[&UsageEvent]) -> ErrorAnalysis {
        let error_events: Vec<&UsageEvent> = events
            .iter()
            .filter(|e| matches!(e.event_type, EventType::ErrorOccurred))
            .cloned()
            .collect();

        let total_errors = error_events.len();
        let unique_sessions = events
            .iter()
            .map(|e| &e.session_id)
            .collect::<std::collections::HashSet<_>>()
            .len();
        let error_rate = if unique_sessions > 0 {
            total_errors as f64 / unique_sessions as f64
        } else {
            0.0
        };

        // Most common errors
        let mut error_counts = HashMap::new();
        for event in &error_events {
            if let Some(error_type) = event.properties.get("error_type")
                && let Some(error_str) = error_type.as_str()
            {
                *error_counts.entry(error_str.to_string()).or_insert(0) += 1;
            }
        }

        let mut most_common_errors: Vec<(String, usize)> = error_counts.into_iter().collect();
        most_common_errors.sort_by(|a, b| b.1.cmp(&a.1));
        most_common_errors.truncate(5);

        // Error trends (daily)
        let mut daily_errors = BTreeMap::new();
        for event in &error_events {
            let date = event.timestamp.date_naive();
            *daily_errors.entry(date).or_insert(0) += 1;
        }

        let error_trends: Vec<(chrono::DateTime<chrono::Utc>, usize)> = daily_errors
            .into_iter()
            .map(|(date, count)| (date.and_hms_opt(0, 0, 0).unwrap().and_utc(), count))
            .collect();

        ErrorAnalysis {
            total_errors,
            error_rate,
            most_common_errors,
            error_trends,
        }
    }

    /// Update session information
    async fn update_session_info(&self, event: &UsageEvent) {
        let mut sessions = self.session_cache.write().await;

        let session_info =
            sessions
                .entry(event.session_id.clone())
                .or_insert_with(|| SessionInfo {
                    session_id: event.session_id.clone(),
                    user_id: event.user_id.clone(),
                    start_time: event.timestamp,
                    last_activity: event.timestamp,
                    event_count: 0,
                });

        session_info.last_activity = event.timestamp;
        session_info.event_count += 1;
    }

    /// Flush events to persistent storage
    async fn flush_events(&self) -> Result<(), AppError> {
        let mut events = self.events.write().await;
        let _config = self.config.read().await;

        if events.is_empty() {
            return Ok(());
        }

        // In a real implementation, this would write to a database or file
        tracing::info!("Flushing {} analytics events", events.len());

        // Clear events after flushing
        events.clear();

        let mut last_flush = self.last_flush.write().await;
        *last_flush = chrono::Utc::now();

        Ok(())
    }

    /// Clean up old data
    pub async fn cleanup_old_data(&self) -> Result<usize, AppError> {
        let config = self.config.read().await;
        let cutoff_date = chrono::Utc::now() - chrono::Duration::days(config.retention_days);

        let mut events = self.events.write().await;
        let initial_count = events.len();

        events.retain(|event| event.timestamp > cutoff_date);

        let removed_count = initial_count - events.len();
        if removed_count > 0 {
            tracing::info!("Cleaned up {} old analytics events", removed_count);
        }

        Ok(removed_count)
    }

    /// Update configuration
    pub async fn update_config(&self, new_config: AnalyticsConfig) {
        let mut config = self.config.write().await;
        *config = new_config;
    }

    /// Get current configuration
    pub async fn get_config(&self) -> AnalyticsConfig {
        let config = self.config.read().await;
        config.clone()
    }

    /// Get cached insights
    pub async fn get_cached_insights(&self) -> Option<AnalyticsInsights> {
        let cache = self.insights_cache.read().await;
        cache.clone()
    }
}

/// Global usage analytics instance
static USAGE_ANALYTICS: tokio::sync::OnceCell<UsageAnalytics> = tokio::sync::OnceCell::const_new();

/// Get the global usage analytics
pub async fn get_usage_analytics() -> &'static UsageAnalytics {
    USAGE_ANALYTICS
        .get_or_init(|| async { UsageAnalytics::new(AnalyticsConfig::default()) })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_event_tracking() {
        // Use Full privacy mode to preserve user_id for testing
        let config = AnalyticsConfig {
            enable_collection: true,
            anonymize_data: false,
            privacy_mode: PrivacyMode::Full,
            ..Default::default()
        };
        let analytics = UsageAnalytics::new(config);

        analytics
            .track_app_launch("session1".to_string(), Some("user1".to_string()))
            .await
            .unwrap();
        analytics
            .track_voice_command(
                "session1".to_string(),
                Some("user1".to_string()),
                "test command",
                0.9,
                100,
            )
            .await
            .unwrap();

        let insights = analytics.generate_insights(Some(1)).await.unwrap();
        assert_eq!(insights.total_events, 2);
        assert_eq!(insights.unique_users, 1);
    }

    #[tokio::test]
    async fn test_privacy_mode() {
        let config = AnalyticsConfig {
            privacy_mode: PrivacyMode::Anonymous,
            ..Default::default()
        };

        let analytics = UsageAnalytics::new(config);

        let event = UsageEvent {
            event_id: "test".to_string(),
            event_type: EventType::VoiceCommand,
            timestamp: chrono::Utc::now(),
            user_id: Some("user123".to_string()),
            session_id: "session1".to_string(),
            properties: HashMap::new(),
            duration_ms: None,
        };

        analytics.track_event(event.clone()).await.unwrap();

        // User ID should be anonymized
        let events = analytics.events.read().await;
        assert!(events[0].user_id.is_none());
    }

    #[tokio::test]
    async fn test_insights_generation() {
        let analytics = UsageAnalytics::new(AnalyticsConfig::default());

        // Add some test events
        for i in 0..10 {
            analytics
                .track_gesture(
                    "session1".to_string(),
                    Some("user1".to_string()),
                    "tap",
                    0.8 + (i as f32 * 0.01),
                )
                .await
                .unwrap();
        }

        let insights = analytics.generate_insights(Some(1)).await.unwrap();
        assert_eq!(insights.total_events, 10);
        assert!(insights.performance_metrics.gesture_recognition_accuracy > 0.8);
    }
}
