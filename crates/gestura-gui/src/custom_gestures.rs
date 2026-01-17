//! Custom gesture definitions for Gestura.app
//! Allows users to define and train custom gestures

#[allow(unused_imports)]
use crate::AppError;
use chrono::Timelike;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Custom gesture definition
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CustomGesture {
    pub id: String,
    pub name: String,
    pub description: String,
    pub user_id: String,
    pub gesture_type: GestureType,
    pub trigger_conditions: Vec<TriggerCondition>,
    pub actions: Vec<GestureAction>,
    pub training_samples: Vec<GestureTrainingSample>,
    pub recognition_threshold: f32,
    pub is_enabled: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub last_modified: chrono::DateTime<chrono::Utc>,
    pub usage_count: u32,
}

/// Types of custom gestures
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum GestureType {
    Motion,      // Hand/arm movements
    Tap,         // Finger taps
    Swipe,       // Directional swipes
    Pinch,       // Pinch gestures
    Rotation,    // Rotational movements
    Hold,        // Hold gestures
    Sequence,    // Sequence of gestures
    Combination, // Multiple simultaneous gestures
}

/// Trigger conditions for gestures
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum TriggerCondition {
    ApplicationActive(String),
    WindowTitle(String),
    TimeOfDay { start: u8, end: u8 },
    DeviceOrientation(String),
    BatteryLevel { min: u8, max: u8 },
    LocationContext(String),
    UserState(String),
}

/// Actions to perform when gesture is recognized
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum GestureAction {
    KeyboardShortcut(String),
    LaunchApplication(String),
    SendNotification {
        title: String,
        message: String,
    },
    ExecuteCommand(String),
    TriggerHapticFeedback {
        pattern: String,
        intensity: f32,
    },
    PlaySound(String),
    ChangeSystemSetting {
        setting: String,
        value: String,
    },
    SendWebhook {
        url: String,
        payload: serde_json::Value,
    },
    RunScript(String),
    Custom {
        plugin_id: String,
        command: String,
        args: serde_json::Value,
    },
}

/// Training sample for gesture recognition
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GestureTrainingSample {
    pub sample_id: String,
    pub sensor_data: Vec<SensorReading>,
    pub duration_ms: u64,
    pub quality_score: f32,
    pub recorded_at: chrono::DateTime<chrono::Utc>,
}

/// Sensor reading for gesture training
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SensorReading {
    pub timestamp_ms: u64,
    pub accelerometer: [f32; 3],
    pub gyroscope: [f32; 3],
    pub magnetometer: [f32; 3],
    pub quaternion: [f32; 4],
}

/// Gesture recognition result
#[derive(Debug, Clone, serde::Serialize)]
pub struct GestureRecognitionResult {
    pub gesture_id: Option<String>,
    pub gesture_name: Option<String>,
    pub confidence: f32,
    pub execution_time_ms: f32,
    pub actions_executed: Vec<String>,
}

/// Custom gesture manager
pub struct CustomGestureManager {
    gestures: Arc<RwLock<HashMap<String, CustomGesture>>>,
    user_gestures: Arc<RwLock<HashMap<String, Vec<String>>>>, // user_id -> gesture_ids
    active_training: Arc<RwLock<Option<TrainingSession>>>,
    recognition_engine: GestureRecognitionEngine,
}

/// Training session for recording gesture samples
#[derive(Debug, Clone)]
struct TrainingSession {
    gesture_id: String,
    #[allow(dead_code)]
    user_id: String,
    samples_recorded: usize,
    target_samples: usize,
    #[allow(dead_code)]
    started_at: chrono::DateTime<chrono::Utc>,
}

impl Default for CustomGestureManager {
    fn default() -> Self {
        Self::new()
    }
}

impl CustomGestureManager {
    /// Create a new custom gesture manager
    pub fn new() -> Self {
        Self {
            gestures: Arc::new(RwLock::new(HashMap::new())),
            user_gestures: Arc::new(RwLock::new(HashMap::new())),
            active_training: Arc::new(RwLock::new(None)),
            recognition_engine: GestureRecognitionEngine::new(),
        }
    }

    /// Create a new custom gesture
    pub async fn create_gesture(
        &self,
        user_id: String,
        name: String,
        description: String,
        gesture_type: GestureType,
    ) -> Result<String, AppError> {
        let gesture_id = uuid::Uuid::new_v4().to_string();

        let gesture = CustomGesture {
            id: gesture_id.clone(),
            name: name.clone(),
            description,
            user_id: user_id.clone(),
            gesture_type,
            trigger_conditions: Vec::new(),
            actions: Vec::new(),
            training_samples: Vec::new(),
            recognition_threshold: 0.8,
            is_enabled: false, // Disabled until trained
            created_at: chrono::Utc::now(),
            last_modified: chrono::Utc::now(),
            usage_count: 0,
        };

        // Store gesture
        let mut gestures = self.gestures.write().await;
        gestures.insert(gesture_id.clone(), gesture);

        // Add to user's gesture list
        let mut user_gestures = self.user_gestures.write().await;
        user_gestures
            .entry(user_id.clone())
            .or_insert_with(Vec::new)
            .push(gesture_id.clone());

        tracing::info!("Created custom gesture '{}' for user {}", name, user_id);
        Ok(gesture_id)
    }

    /// Start training session for a gesture
    pub async fn start_training(
        &self,
        gesture_id: String,
        target_samples: usize,
    ) -> Result<(), AppError> {
        let gestures = self.gestures.read().await;
        let gesture = gestures.get(&gesture_id).ok_or_else(|| {
            AppError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Gesture not found",
            ))
        })?;

        let session = TrainingSession {
            gesture_id: gesture_id.clone(),
            user_id: gesture.user_id.clone(),
            samples_recorded: 0,
            target_samples,
            started_at: chrono::Utc::now(),
        };

        let mut active_training = self.active_training.write().await;
        *active_training = Some(session);

        tracing::info!("Started training session for gesture: {}", gesture_id);
        Ok(())
    }

    /// Record a training sample
    pub async fn record_training_sample(
        &self,
        sensor_data: Vec<SensorReading>,
    ) -> Result<bool, AppError> {
        let mut active_training = self.active_training.write().await;

        let session = active_training.as_mut().ok_or_else(|| {
            AppError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "No active training session",
            ))
        })?;

        // Calculate sample quality
        let quality_score = self.calculate_sample_quality(&sensor_data);

        if quality_score < 0.5 {
            return Err(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Sample quality too low",
            )));
        }

        // Create training sample
        let sample = GestureTrainingSample {
            sample_id: uuid::Uuid::new_v4().to_string(),
            sensor_data,
            duration_ms: 1000, // Calculate from sensor data
            quality_score,
            recorded_at: chrono::Utc::now(),
        };

        // Add sample to gesture
        let mut gestures = self.gestures.write().await;
        if let Some(gesture) = gestures.get_mut(&session.gesture_id) {
            gesture.training_samples.push(sample);
            gesture.last_modified = chrono::Utc::now();
        }

        session.samples_recorded += 1;
        let is_complete = session.samples_recorded >= session.target_samples;

        if is_complete {
            let gesture_id = session.gesture_id.clone();
            *active_training = None;
            drop(active_training);
            drop(gestures);

            // Train the gesture model
            self.train_gesture_model(&gesture_id).await?;
        }

        Ok(is_complete)
    }

    /// Train the gesture recognition model
    async fn train_gesture_model(&self, gesture_id: &str) -> Result<(), AppError> {
        let mut gestures = self.gestures.write().await;
        let gesture = gestures.get_mut(gesture_id).ok_or_else(|| {
            AppError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Gesture not found",
            ))
        })?;

        if gesture.training_samples.len() < 3 {
            return Err(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Need at least 3 training samples",
            )));
        }

        // Train the model (simplified)
        let model_accuracy = self
            .recognition_engine
            .train_model(&gesture.training_samples)
            .await?;

        // Enable gesture if accuracy is good enough
        if model_accuracy > 0.8 {
            gesture.is_enabled = true;
            tracing::info!(
                "Gesture '{}' trained successfully with {:.2}% accuracy",
                gesture.name,
                model_accuracy * 100.0
            );
        } else {
            tracing::warn!(
                "Gesture '{}' training completed but accuracy too low: {:.2}%",
                gesture.name,
                model_accuracy * 100.0
            );
        }

        Ok(())
    }

    /// Calculate quality score for a training sample
    fn calculate_sample_quality(&self, sensor_data: &[SensorReading]) -> f32 {
        if sensor_data.is_empty() {
            return 0.0;
        }

        // Calculate signal strength and consistency
        let mut total_magnitude = 0.0;
        let mut variance = 0.0;

        for reading in sensor_data {
            let acc_mag = (reading.accelerometer[0].powi(2)
                + reading.accelerometer[1].powi(2)
                + reading.accelerometer[2].powi(2))
            .sqrt();
            total_magnitude += acc_mag;
        }

        let avg_magnitude = total_magnitude / sensor_data.len() as f32;

        // Calculate variance
        for reading in sensor_data {
            let acc_mag = (reading.accelerometer[0].powi(2)
                + reading.accelerometer[1].powi(2)
                + reading.accelerometer[2].powi(2))
            .sqrt();
            variance += (acc_mag - avg_magnitude).powi(2);
        }
        variance /= sensor_data.len() as f32;

        // Quality score based on signal strength and consistency
        let signal_score = (avg_magnitude / 10.0).min(1.0); // Normalize to 0-1
        let consistency_score = 1.0 / (1.0 + variance); // Higher variance = lower score

        (signal_score + consistency_score) / 2.0
    }

    /// Recognize gesture from sensor data
    pub async fn recognize_gesture(
        &self,
        user_id: &str,
        sensor_data: Vec<SensorReading>,
    ) -> Result<GestureRecognitionResult, AppError> {
        let start_time = std::time::Instant::now();

        // Get user's gestures
        let user_gestures = self.user_gestures.read().await;
        let gesture_ids = user_gestures.get(user_id).cloned().unwrap_or_default();

        let gestures = self.gestures.read().await;
        let mut best_match: Option<(String, String, f32)> = None; // (id, name, confidence)

        // Test against each enabled gesture
        for gesture_id in &gesture_ids {
            if let Some(gesture) = gestures.get(gesture_id) {
                if !gesture.is_enabled {
                    continue;
                }

                // Check trigger conditions
                if !self
                    .check_trigger_conditions(&gesture.trigger_conditions)
                    .await
                {
                    continue;
                }

                // Calculate recognition confidence
                let confidence = self
                    .recognition_engine
                    .calculate_confidence(&sensor_data, &gesture.training_samples)
                    .await;

                if confidence > gesture.recognition_threshold
                    && (best_match.is_none() || confidence > best_match.as_ref().unwrap().2)
                {
                    best_match = Some((gesture_id.clone(), gesture.name.clone(), confidence));
                }
            }
        }

        let execution_time = start_time.elapsed().as_millis() as f32;

        if let Some((gesture_id, gesture_name, confidence)) = best_match {
            // Execute gesture actions
            let actions_executed = self.execute_gesture_actions(&gesture_id).await?;

            // Update usage count
            drop(gestures);
            let mut gestures_mut = self.gestures.write().await;
            if let Some(gesture) = gestures_mut.get_mut(&gesture_id) {
                gesture.usage_count += 1;
            }

            Ok(GestureRecognitionResult {
                gesture_id: Some(gesture_id),
                gesture_name: Some(gesture_name),
                confidence,
                execution_time_ms: execution_time,
                actions_executed,
            })
        } else {
            Ok(GestureRecognitionResult {
                gesture_id: None,
                gesture_name: None,
                confidence: 0.0,
                execution_time_ms: execution_time,
                actions_executed: Vec::new(),
            })
        }
    }

    /// Check if trigger conditions are met
    async fn check_trigger_conditions(&self, conditions: &[TriggerCondition]) -> bool {
        if conditions.is_empty() {
            return true; // No conditions = always trigger
        }

        for condition in conditions {
            match condition {
                TriggerCondition::TimeOfDay { start, end } => {
                    let now = chrono::Local::now().hour() as u8;
                    if now < *start || now > *end {
                        return false;
                    }
                }
                TriggerCondition::ApplicationActive(app) => {
                    // In real implementation, would check active application
                    tracing::debug!("Checking if application '{}' is active", app);
                }
                _ => {
                    // Other conditions would be implemented based on system capabilities
                }
            }
        }

        true
    }

    /// Execute gesture actions
    async fn execute_gesture_actions(&self, gesture_id: &str) -> Result<Vec<String>, AppError> {
        let gestures = self.gestures.read().await;
        let gesture = gestures.get(gesture_id).ok_or_else(|| {
            AppError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Gesture not found",
            ))
        })?;

        let mut executed_actions = Vec::new();

        for action in &gesture.actions {
            match action {
                GestureAction::SendNotification { title, message } => {
                    tracing::info!("Notification: {} - {}", title, message);
                    executed_actions.push("notification".to_string());
                }
                GestureAction::TriggerHapticFeedback { pattern, intensity } => {
                    tracing::info!("Haptic feedback: {} at {:.1}%", pattern, intensity * 100.0);
                    executed_actions.push("haptic".to_string());
                }
                GestureAction::KeyboardShortcut(shortcut) => {
                    tracing::info!("Keyboard shortcut: {}", shortcut);
                    executed_actions.push("keyboard".to_string());
                }
                GestureAction::LaunchApplication(app) => {
                    tracing::info!("Launch application: {}", app);
                    executed_actions.push("launch_app".to_string());
                }
                GestureAction::ExecuteCommand(command) => {
                    tracing::info!("Execute command: {}", command);
                    executed_actions.push("command".to_string());
                }
                _ => {
                    tracing::debug!("Executing action: {:?}", action);
                    executed_actions.push("other".to_string());
                }
            }
        }

        Ok(executed_actions)
    }

    /// Get user's gestures
    pub async fn get_user_gestures(&self, user_id: &str) -> Vec<CustomGesture> {
        let user_gestures = self.user_gestures.read().await;
        let gesture_ids = user_gestures.get(user_id).cloned().unwrap_or_default();

        let gestures = self.gestures.read().await;
        gesture_ids
            .iter()
            .filter_map(|id| gestures.get(id).cloned())
            .collect()
    }

    /// Delete a custom gesture
    pub async fn delete_gesture(&self, user_id: &str, gesture_id: &str) -> Result<(), AppError> {
        let mut gestures = self.gestures.write().await;
        let mut user_gestures = self.user_gestures.write().await;

        // Verify ownership
        if let Some(gesture) = gestures.get(gesture_id)
            && gesture.user_id != user_id
        {
            return Err(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "Not authorized to delete this gesture",
            )));
        }

        gestures.remove(gesture_id);

        if let Some(user_gesture_list) = user_gestures.get_mut(user_id) {
            user_gesture_list.retain(|id| id != gesture_id);
        }

        tracing::info!("Deleted custom gesture: {}", gesture_id);
        Ok(())
    }

    /// Get gesture statistics
    pub async fn get_stats(&self) -> serde_json::Value {
        let gestures = self.gestures.read().await;
        let user_gestures = self.user_gestures.read().await;

        let total_gestures = gestures.len();
        let enabled_gestures = gestures.values().filter(|g| g.is_enabled).count();
        let total_users = user_gestures.len();
        let total_usage = gestures.values().map(|g| g.usage_count).sum::<u32>();

        serde_json::json!({
            "total_gestures": total_gestures,
            "enabled_gestures": enabled_gestures,
            "total_users": total_users,
            "total_usage": total_usage
        })
    }
}

/// Gesture recognition engine
struct GestureRecognitionEngine;

impl GestureRecognitionEngine {
    fn new() -> Self {
        Self
    }

    async fn train_model(&self, _samples: &[GestureTrainingSample]) -> Result<f32, AppError> {
        // Simplified training - in real implementation would use ML algorithms
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        Ok(0.85) // Mock accuracy
    }

    async fn calculate_confidence(
        &self,
        _input: &[SensorReading],
        _training_samples: &[GestureTrainingSample],
    ) -> f32 {
        // Simplified confidence calculation
        0.9 // Mock confidence
    }
}

/// Global custom gesture manager instance
static CUSTOM_GESTURE_MANAGER: tokio::sync::OnceCell<CustomGestureManager> =
    tokio::sync::OnceCell::const_new();

/// Get the global custom gesture manager
pub async fn get_custom_gesture_manager() -> &'static CustomGestureManager {
    CUSTOM_GESTURE_MANAGER
        .get_or_init(|| async { CustomGestureManager::new() })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_gesture_creation() {
        let manager = CustomGestureManager::new();

        let gesture_id = manager
            .create_gesture(
                "user1".to_string(),
                "Test Gesture".to_string(),
                "A test gesture".to_string(),
                GestureType::Tap,
            )
            .await
            .unwrap();

        let gestures = manager.get_user_gestures("user1").await;
        assert_eq!(gestures.len(), 1);
        assert_eq!(gestures[0].id, gesture_id);
    }

    #[tokio::test]
    async fn test_training_session() {
        let manager = CustomGestureManager::new();

        let gesture_id = manager
            .create_gesture(
                "user1".to_string(),
                "Test Gesture".to_string(),
                "A test gesture".to_string(),
                GestureType::Tap,
            )
            .await
            .unwrap();

        manager.start_training(gesture_id, 3).await.unwrap();

        // Record training samples
        let sensor_data = vec![SensorReading {
            timestamp_ms: 0,
            accelerometer: [1.0, 0.0, 0.0],
            gyroscope: [0.0, 0.0, 0.0],
            magnetometer: [0.0, 0.0, 1.0],
            quaternion: [1.0, 0.0, 0.0, 0.0],
        }];

        let result = manager.record_training_sample(sensor_data).await;
        assert!(result.is_ok());
    }
}
