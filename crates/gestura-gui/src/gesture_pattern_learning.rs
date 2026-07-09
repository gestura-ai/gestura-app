//! Gesture pattern learning for Gestura.app
//! Machine learning system for recognizing and learning gesture patterns

#[allow(unused_imports)]
use crate::AppError;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Gesture data point
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GestureDataPoint {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub accelerometer: [f32; 3], // x, y, z
    pub gyroscope: [f32; 3],     // x, y, z
    pub magnetometer: [f32; 3],  // x, y, z
    pub quaternion: [f32; 4],    // w, x, y, z
}

/// Gesture sequence
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GestureSequence {
    pub id: String,
    pub user_id: String,
    pub gesture_type: String,
    pub data_points: Vec<GestureDataPoint>,
    pub duration_ms: u64,
    pub confidence: f32,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Learned gesture pattern
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GesturePattern {
    pub pattern_id: String,
    pub gesture_type: String,
    pub user_id: Option<String>, // None for global patterns
    pub feature_vector: Vec<f32>,
    pub confidence_threshold: f32,
    pub sample_count: usize,
    pub accuracy: f32,
    pub last_updated: chrono::DateTime<chrono::Utc>,
}

/// Gesture recognition result
#[derive(Debug, Clone, serde::Serialize)]
pub struct GestureRecognitionResult {
    pub recognized_gesture: Option<String>,
    pub confidence: f32,
    pub alternatives: Vec<(String, f32)>, // (gesture_type, confidence)
    pub processing_time_ms: f32,
}

/// Learning configuration
#[derive(Debug, Clone)]
pub struct LearningConfig {
    pub min_samples_per_pattern: usize,
    pub max_pattern_age_days: i64,
    pub confidence_threshold: f32,
    pub feature_window_size: usize,
    pub learning_rate: f32,
    pub enable_online_learning: bool,
    pub enable_user_adaptation: bool,
}

impl Default for LearningConfig {
    fn default() -> Self {
        Self {
            min_samples_per_pattern: 10,
            max_pattern_age_days: 30,
            confidence_threshold: 0.7,
            feature_window_size: 50,
            learning_rate: 0.01,
            enable_online_learning: true,
            enable_user_adaptation: true,
        }
    }
}

/// Gesture pattern learning system
pub struct GesturePatternLearner {
    patterns: Arc<RwLock<HashMap<String, GesturePattern>>>,
    training_data: Arc<RwLock<Vec<GestureSequence>>>,
    config: Arc<RwLock<LearningConfig>>,
    feature_extractor: GestureFeatureExtractor,
    classifier: GestureClassifier,
}

impl GesturePatternLearner {
    /// Create a new gesture pattern learner
    pub fn new(config: LearningConfig) -> Self {
        Self {
            patterns: Arc::new(RwLock::new(HashMap::new())),
            training_data: Arc::new(RwLock::new(Vec::new())),
            config: Arc::new(RwLock::new(config)),
            feature_extractor: GestureFeatureExtractor::new(),
            classifier: GestureClassifier::new(),
        }
    }

    /// Add training data
    pub async fn add_training_data(&self, sequence: GestureSequence) -> Result<(), AppError> {
        // Validate sequence
        if sequence.data_points.is_empty() {
            return Err(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Gesture sequence cannot be empty",
            )));
        }

        if sequence.gesture_type.is_empty() {
            return Err(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Gesture type cannot be empty",
            )));
        }

        let mut training_data = self.training_data.write().await;
        training_data.push(sequence.clone());

        // Trigger learning if we have enough samples
        let config = self.config.read().await;
        let gesture_count = training_data
            .iter()
            .filter(|s| s.gesture_type == sequence.gesture_type)
            .count();

        if gesture_count >= config.min_samples_per_pattern {
            drop(config);
            drop(training_data);
            self.update_pattern(&sequence.gesture_type).await?;
        }

        tracing::debug!("Added training data for gesture: {}", sequence.gesture_type);
        Ok(())
    }

    /// Recognize gesture from sequence
    pub async fn recognize_gesture(
        &self,
        sequence: &GestureSequence,
    ) -> Result<GestureRecognitionResult, AppError> {
        let start_time = std::time::Instant::now();

        // Extract features from the sequence
        let features = self
            .feature_extractor
            .extract_features(&sequence.data_points)?;

        // Get all patterns
        let patterns = self.patterns.read().await;
        let mut scores = Vec::new();

        // Compare with all learned patterns
        for pattern in patterns.values() {
            // Skip user-specific patterns if not matching user
            if let Some(pattern_user) = &pattern.user_id
                && *pattern_user != sequence.user_id
            {
                continue;
            }

            let similarity = self
                .classifier
                .calculate_similarity(&features, &pattern.feature_vector);
            if similarity >= pattern.confidence_threshold {
                scores.push((pattern.gesture_type.clone(), similarity));
            }
        }

        // Sort by confidence (highest first)
        scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let processing_time = start_time.elapsed().as_millis() as f32;
        let config = self.config.read().await;

        let result = if let Some((best_gesture, best_confidence)) = scores.first() {
            if *best_confidence >= config.confidence_threshold {
                GestureRecognitionResult {
                    recognized_gesture: Some(best_gesture.clone()),
                    confidence: *best_confidence,
                    alternatives: scores.iter().skip(1).take(3).cloned().collect(),
                    processing_time_ms: processing_time,
                }
            } else {
                GestureRecognitionResult {
                    recognized_gesture: None,
                    confidence: *best_confidence,
                    alternatives: scores.iter().take(3).cloned().collect(),
                    processing_time_ms: processing_time,
                }
            }
        } else {
            GestureRecognitionResult {
                recognized_gesture: None,
                confidence: 0.0,
                alternatives: Vec::new(),
                processing_time_ms: processing_time,
            }
        };

        // Online learning: update pattern if recognition was confident
        if config.enable_online_learning
            && result.confidence > 0.8
            && let Some(ref gesture_type) = result.recognized_gesture
        {
            drop(config);
            drop(patterns);
            self.update_pattern_online(gesture_type, &features).await?;
        }

        Ok(result)
    }

    /// Update pattern for a specific gesture type
    async fn update_pattern(&self, gesture_type: &str) -> Result<(), AppError> {
        let training_data = self.training_data.read().await;

        // Get all sequences for this gesture type
        let gesture_sequences: Vec<&GestureSequence> = training_data
            .iter()
            .filter(|s| s.gesture_type == gesture_type)
            .collect();

        if gesture_sequences.is_empty() {
            return Ok(());
        }

        // Extract features from all sequences
        let mut all_features = Vec::new();
        for sequence in &gesture_sequences {
            let features = self
                .feature_extractor
                .extract_features(&sequence.data_points)?;
            all_features.push(features);
        }

        // Calculate average feature vector
        let feature_vector = self.classifier.average_features(&all_features);

        // Calculate accuracy (simplified)
        let accuracy = self.calculate_pattern_accuracy(&feature_vector, &all_features);

        // Create or update pattern
        let pattern_id = format!(
            "{}_{}",
            gesture_type,
            &uuid::Uuid::new_v4().to_string()[..8]
        );
        let pattern = GesturePattern {
            pattern_id: pattern_id.clone(),
            gesture_type: gesture_type.to_string(),
            user_id: None, // Global pattern
            feature_vector,
            confidence_threshold: 0.7,
            sample_count: gesture_sequences.len(),
            accuracy,
            last_updated: chrono::Utc::now(),
        };

        let mut patterns = self.patterns.write().await;
        patterns.insert(pattern_id.clone(), pattern);

        tracing::info!(
            "Updated pattern for gesture '{}' with {} samples (accuracy: {:.2})",
            gesture_type,
            gesture_sequences.len(),
            accuracy
        );

        Ok(())
    }

    /// Online learning update
    async fn update_pattern_online(
        &self,
        gesture_type: &str,
        new_features: &[f32],
    ) -> Result<(), AppError> {
        let mut patterns = self.patterns.write().await;
        let config = self.config.read().await;

        // Find existing pattern for this gesture type
        let pattern_key = patterns
            .iter()
            .find(|(_, p)| p.gesture_type == gesture_type)
            .map(|(k, _)| k.clone());

        if let Some(key) = pattern_key
            && let Some(pattern) = patterns.get_mut(&key)
        {
            // Update feature vector using exponential moving average
            for (i, &new_val) in new_features.iter().enumerate() {
                if i < pattern.feature_vector.len() {
                    pattern.feature_vector[i] = (1.0 - config.learning_rate)
                        * pattern.feature_vector[i]
                        + config.learning_rate * new_val;
                }
            }

            pattern.last_updated = chrono::Utc::now();
            pattern.sample_count += 1;

            tracing::debug!("Updated pattern '{}' with online learning", gesture_type);
        }

        Ok(())
    }

    /// Calculate pattern accuracy
    fn calculate_pattern_accuracy(
        &self,
        pattern_features: &[f32],
        all_features: &[Vec<f32>],
    ) -> f32 {
        if all_features.is_empty() {
            return 0.0;
        }

        let mut correct_predictions = 0;
        for features in all_features {
            let similarity = self
                .classifier
                .calculate_similarity(pattern_features, features);
            if similarity > 0.7 {
                correct_predictions += 1;
            }
        }

        correct_predictions as f32 / all_features.len() as f32
    }

    /// Get all learned patterns
    pub async fn get_patterns(&self) -> Vec<GesturePattern> {
        let patterns = self.patterns.read().await;
        patterns.values().cloned().collect()
    }

    /// Get learning statistics
    pub async fn get_learning_stats(&self) -> serde_json::Value {
        let patterns = self.patterns.read().await;
        let training_data = self.training_data.read().await;

        let gesture_types: std::collections::HashSet<String> = training_data
            .iter()
            .map(|s| s.gesture_type.clone())
            .collect();

        let total_samples = training_data.len();
        let total_patterns = patterns.len();
        let average_accuracy = if !patterns.is_empty() {
            patterns.values().map(|p| p.accuracy).sum::<f32>() / patterns.len() as f32
        } else {
            0.0
        };

        serde_json::json!({
            "total_samples": total_samples,
            "total_patterns": total_patterns,
            "unique_gesture_types": gesture_types.len(),
            "average_accuracy": average_accuracy,
            "gesture_types": gesture_types.into_iter().collect::<Vec<_>>()
        })
    }

    /// Clear old patterns
    pub async fn cleanup_old_patterns(&self) -> Result<usize, AppError> {
        let config = self.config.read().await;
        let cutoff_date = chrono::Utc::now() - chrono::Duration::days(config.max_pattern_age_days);

        let mut patterns = self.patterns.write().await;
        let initial_count = patterns.len();

        patterns.retain(|_, pattern| pattern.last_updated > cutoff_date);

        let removed_count = initial_count - patterns.len();
        if removed_count > 0 {
            tracing::info!("Cleaned up {} old patterns", removed_count);
        }

        Ok(removed_count)
    }

    /// Update configuration
    pub async fn update_config(&self, new_config: LearningConfig) {
        let mut config = self.config.write().await;
        *config = new_config;
    }
}

/// Feature extractor for gesture data
pub struct GestureFeatureExtractor {
    #[allow(dead_code)]
    window_size: usize,
}

impl Default for GestureFeatureExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl GestureFeatureExtractor {
    pub fn new() -> Self {
        Self { window_size: 50 }
    }

    /// Extract features from gesture data points
    pub fn extract_features(&self, data_points: &[GestureDataPoint]) -> Result<Vec<f32>, AppError> {
        if data_points.is_empty() {
            return Err(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Cannot extract features from empty data",
            )));
        }

        let mut features = Vec::new();

        // Statistical features for accelerometer
        let acc_x: Vec<f32> = data_points.iter().map(|p| p.accelerometer[0]).collect();
        let acc_y: Vec<f32> = data_points.iter().map(|p| p.accelerometer[1]).collect();
        let acc_z: Vec<f32> = data_points.iter().map(|p| p.accelerometer[2]).collect();

        features.extend(self.calculate_statistical_features(&acc_x));
        features.extend(self.calculate_statistical_features(&acc_y));
        features.extend(self.calculate_statistical_features(&acc_z));

        // Statistical features for gyroscope
        let gyro_x: Vec<f32> = data_points.iter().map(|p| p.gyroscope[0]).collect();
        let gyro_y: Vec<f32> = data_points.iter().map(|p| p.gyroscope[1]).collect();
        let gyro_z: Vec<f32> = data_points.iter().map(|p| p.gyroscope[2]).collect();

        features.extend(self.calculate_statistical_features(&gyro_x));
        features.extend(self.calculate_statistical_features(&gyro_y));
        features.extend(self.calculate_statistical_features(&gyro_z));

        // Magnitude features
        let acc_magnitude: Vec<f32> = data_points
            .iter()
            .map(|p| {
                (p.accelerometer[0].powi(2)
                    + p.accelerometer[1].powi(2)
                    + p.accelerometer[2].powi(2))
                .sqrt()
            })
            .collect();
        features.extend(self.calculate_statistical_features(&acc_magnitude));

        let gyro_magnitude: Vec<f32> = data_points
            .iter()
            .map(|p| {
                (p.gyroscope[0].powi(2) + p.gyroscope[1].powi(2) + p.gyroscope[2].powi(2)).sqrt()
            })
            .collect();
        features.extend(self.calculate_statistical_features(&gyro_magnitude));

        // Temporal features
        features.push(data_points.len() as f32); // Sequence length

        if data_points.len() > 1 {
            let duration = (data_points.last().unwrap().timestamp
                - data_points.first().unwrap().timestamp)
                .num_milliseconds() as f32;
            features.push(duration); // Total duration
            features.push(data_points.len() as f32 / duration * 1000.0); // Sampling rate
        } else {
            features.push(0.0);
            features.push(0.0);
        }

        Ok(features)
    }

    /// Calculate statistical features for a signal
    fn calculate_statistical_features(&self, signal: &[f32]) -> Vec<f32> {
        if signal.is_empty() {
            return vec![0.0; 5]; // mean, std, min, max, range
        }

        let mean = signal.iter().sum::<f32>() / signal.len() as f32;
        let variance = signal.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / signal.len() as f32;
        let std_dev = variance.sqrt();
        let min_val = signal.iter().fold(f32::INFINITY, |a, &b| a.min(b));
        let max_val = signal.iter().fold(f32::NEG_INFINITY, |a, &b| a.max(b));
        let range = max_val - min_val;

        vec![mean, std_dev, min_val, max_val, range]
    }
}

/// Gesture classifier
pub struct GestureClassifier;

impl Default for GestureClassifier {
    fn default() -> Self {
        Self::new()
    }
}

impl GestureClassifier {
    pub fn new() -> Self {
        Self
    }

    /// Calculate similarity between two feature vectors
    pub fn calculate_similarity(&self, features1: &[f32], features2: &[f32]) -> f32 {
        if features1.len() != features2.len() {
            return 0.0;
        }

        // Cosine similarity
        let dot_product: f32 = features1
            .iter()
            .zip(features2.iter())
            .map(|(a, b)| a * b)
            .sum();
        let norm1: f32 = features1.iter().map(|x| x * x).sum::<f32>().sqrt();
        let norm2: f32 = features2.iter().map(|x| x * x).sum::<f32>().sqrt();

        if norm1 == 0.0 || norm2 == 0.0 {
            0.0
        } else {
            dot_product / (norm1 * norm2)
        }
    }

    /// Average multiple feature vectors
    pub fn average_features(&self, features_list: &[Vec<f32>]) -> Vec<f32> {
        if features_list.is_empty() {
            return Vec::new();
        }

        let feature_count = features_list[0].len();
        let mut averaged = vec![0.0; feature_count];

        for features in features_list {
            for (i, &val) in features.iter().enumerate() {
                if i < averaged.len() {
                    averaged[i] += val;
                }
            }
        }

        let count = features_list.len() as f32;
        for avg in averaged.iter_mut() {
            *avg /= count;
        }

        averaged
    }
}

/// Global gesture pattern learner instance
static GESTURE_PATTERN_LEARNER: tokio::sync::OnceCell<GesturePatternLearner> =
    tokio::sync::OnceCell::const_new();

/// Get the global gesture pattern learner
pub async fn get_gesture_pattern_learner() -> &'static GesturePatternLearner {
    GESTURE_PATTERN_LEARNER
        .get_or_init(|| async { GesturePatternLearner::new(LearningConfig::default()) })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_feature_extraction() {
        let extractor = GestureFeatureExtractor::new();

        let data_points = vec![
            GestureDataPoint {
                timestamp: chrono::Utc::now(),
                accelerometer: [1.0, 2.0, 3.0],
                gyroscope: [0.1, 0.2, 0.3],
                magnetometer: [10.0, 20.0, 30.0],
                quaternion: [1.0, 0.0, 0.0, 0.0],
            },
            GestureDataPoint {
                timestamp: chrono::Utc::now(),
                accelerometer: [1.1, 2.1, 3.1],
                gyroscope: [0.11, 0.21, 0.31],
                magnetometer: [10.1, 20.1, 30.1],
                quaternion: [0.99, 0.01, 0.01, 0.01],
            },
        ];

        let features = extractor.extract_features(&data_points).unwrap();
        assert!(!features.is_empty());
        assert!(features.len() > 30); // Should have many features
    }

    #[test]
    fn test_similarity_calculation() {
        let classifier = GestureClassifier::new();

        let features1 = vec![1.0, 2.0, 3.0];
        let features2 = vec![1.0, 2.0, 3.0];
        let similarity = classifier.calculate_similarity(&features1, &features2);
        assert!((similarity - 1.0).abs() < 0.001); // Should be 1.0 for identical vectors

        let features3 = vec![0.0, 0.0, 0.0];
        let similarity2 = classifier.calculate_similarity(&features1, &features3);
        assert_eq!(similarity2, 0.0); // Should be 0.0 for zero vector
    }

    #[tokio::test]
    async fn test_pattern_learning() {
        let learner = GesturePatternLearner::new(LearningConfig {
            min_samples_per_pattern: 2,
            ..LearningConfig::default()
        });

        let sequence = GestureSequence {
            id: "test1".to_string(),
            user_id: "user1".to_string(),
            gesture_type: "tap".to_string(),
            data_points: vec![GestureDataPoint {
                timestamp: chrono::Utc::now(),
                accelerometer: [1.0, 0.0, 0.0],
                gyroscope: [0.0, 0.0, 0.0],
                magnetometer: [0.0, 0.0, 0.0],
                quaternion: [1.0, 0.0, 0.0, 0.0],
            }],
            duration_ms: 100,
            confidence: 1.0,
            created_at: chrono::Utc::now(),
        };

        learner.add_training_data(sequence.clone()).await.unwrap();
        learner.add_training_data(sequence).await.unwrap();

        let patterns = learner.get_patterns().await;
        assert!(!patterns.is_empty());
        assert_eq!(patterns[0].gesture_type, "tap");
    }
}
