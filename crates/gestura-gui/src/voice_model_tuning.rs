//! Voice model fine-tuning for Gestura.app
//! Provides capabilities to fine-tune voice recognition models for better accuracy

#[allow(unused_imports)]
use crate::AppError;
#[allow(unused_imports)]
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

/// Fine-tuning configuration
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FineTuningConfig {
    /// Base model path
    pub base_model_path: PathBuf,
    /// Training data directory
    pub training_data_path: PathBuf,
    /// Output model path
    pub output_model_path: PathBuf,
    /// Learning rate
    pub learning_rate: f32,
    /// Number of training epochs
    pub epochs: usize,
    /// Batch size
    pub batch_size: usize,
    /// Validation split ratio
    pub validation_split: f32,
    /// Early stopping patience
    pub early_stopping_patience: usize,
    /// Target vocabulary
    pub target_vocabulary: Vec<String>,
}

impl Default for FineTuningConfig {
    fn default() -> Self {
        Self {
            base_model_path: PathBuf::from("models/base_whisper.bin"),
            training_data_path: PathBuf::from("training_data"),
            output_model_path: PathBuf::from("models/fine_tuned_whisper.bin"),
            learning_rate: 0.0001,
            epochs: 10,
            batch_size: 16,
            validation_split: 0.2,
            early_stopping_patience: 3,
            target_vocabulary: Vec::new(),
        }
    }
}

/// Training sample
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TrainingSample {
    pub audio_path: PathBuf,
    pub transcript: String,
    pub speaker_id: Option<String>,
    pub domain: Option<String>,
    pub quality_score: Option<f32>,
}

/// Training progress information
#[derive(Debug, Clone, serde::Serialize)]
pub struct TrainingProgress {
    pub current_epoch: usize,
    pub total_epochs: usize,
    pub current_batch: usize,
    pub total_batches: usize,
    pub training_loss: f32,
    pub validation_loss: f32,
    pub accuracy: f32,
    pub elapsed_time_seconds: u64,
    pub estimated_remaining_seconds: u64,
}

/// Model evaluation metrics
#[derive(Debug, Clone, serde::Serialize)]
pub struct EvaluationMetrics {
    pub word_error_rate: f32,
    pub character_error_rate: f32,
    pub bleu_score: f32,
    pub perplexity: f32,
    pub inference_time_ms: f32,
    pub model_size_mb: f32,
}

/// Voice model fine-tuner
pub struct VoiceModelTuner {
    config: Arc<RwLock<FineTuningConfig>>,
    training_samples: Arc<RwLock<Vec<TrainingSample>>>,
    progress: Arc<Mutex<Option<TrainingProgress>>>,
    is_training: Arc<Mutex<bool>>,
    training_history: Arc<RwLock<Vec<TrainingProgress>>>,
}

impl VoiceModelTuner {
    /// Create a new voice model tuner
    pub fn new(config: FineTuningConfig) -> Self {
        Self {
            config: Arc::new(RwLock::new(config)),
            training_samples: Arc::new(RwLock::new(Vec::new())),
            progress: Arc::new(Mutex::new(None)),
            is_training: Arc::new(Mutex::new(false)),
            training_history: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Load training data from directory
    pub async fn load_training_data(&self, data_path: &PathBuf) -> Result<usize, AppError> {
        let mut samples = Vec::new();

        // Read training data directory
        let mut entries = tokio::fs::read_dir(data_path).await.map_err(AppError::Io)?;

        while let Some(entry) = entries.next_entry().await.map_err(AppError::Io)? {
            let path = entry.path();

            if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
                // Load training manifest
                let content = tokio::fs::read_to_string(&path)
                    .await
                    .map_err(AppError::Io)?;

                let manifest_samples: Vec<TrainingSample> = serde_json::from_str(&content)
                    .map_err(|e| {
                        AppError::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, e))
                    })?;

                samples.extend(manifest_samples);
            }
        }

        // Validate samples
        let valid_samples = self.validate_training_samples(samples).await?;
        let sample_count = valid_samples.len();

        let mut training_samples = self.training_samples.write().await;
        *training_samples = valid_samples;

        tracing::info!(
            "Loaded {} training samples from {}",
            sample_count,
            data_path.display()
        );
        Ok(sample_count)
    }

    /// Add individual training sample
    pub async fn add_training_sample(&self, sample: TrainingSample) -> Result<(), AppError> {
        // Validate sample
        if !sample.audio_path.exists() {
            return Err(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Audio file not found: {}", sample.audio_path.display()),
            )));
        }

        if sample.transcript.trim().is_empty() {
            return Err(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Transcript cannot be empty",
            )));
        }

        let mut training_samples = self.training_samples.write().await;
        training_samples.push(sample);

        Ok(())
    }

    /// Start fine-tuning process
    pub async fn start_fine_tuning(&self) -> Result<(), AppError> {
        let mut is_training = self.is_training.lock().await;
        if *is_training {
            return Err(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "Training is already in progress",
            )));
        }
        *is_training = true;
        drop(is_training);

        let config = self.config.read().await.clone();
        let samples = self.training_samples.read().await.clone();

        if samples.is_empty() {
            return Err(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "No training samples available",
            )));
        }

        // Split data into training and validation sets
        let (train_samples, val_samples) =
            self.split_training_data(&samples, config.validation_split);

        tracing::info!(
            "Starting fine-tuning with {} training samples, {} validation samples",
            train_samples.len(),
            val_samples.len()
        );

        // Start training in background
        let tuner = self.clone();
        tokio::spawn(async move {
            if let Err(e) = tuner.run_training(config, train_samples, val_samples).await {
                tracing::error!("Training failed: {}", e);
            }

            let mut is_training = tuner.is_training.lock().await;
            *is_training = false;
        });

        Ok(())
    }

    /// Stop fine-tuning process
    pub async fn stop_fine_tuning(&self) -> Result<(), AppError> {
        let mut is_training = self.is_training.lock().await;
        if !*is_training {
            return Err(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "No training in progress",
            )));
        }

        *is_training = false;
        tracing::info!("Training stop requested");
        Ok(())
    }

    /// Get current training progress
    pub async fn get_progress(&self) -> Option<TrainingProgress> {
        let progress = self.progress.lock().await;
        progress.clone()
    }

    /// Check if training is in progress
    pub async fn is_training(&self) -> bool {
        let is_training = self.is_training.lock().await;
        *is_training
    }

    /// Evaluate model performance
    pub async fn evaluate_model(
        &self,
        model_path: &PathBuf,
        test_samples: &[TrainingSample],
    ) -> Result<EvaluationMetrics, AppError> {
        if !model_path.exists() {
            return Err(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Model not found: {}", model_path.display()),
            )));
        }

        // This is a simplified evaluation - in real implementation would use actual model
        let start_time = std::time::Instant::now();

        let mut total_word_errors = 0;
        let mut total_words = 0;
        let mut total_char_errors = 0;
        let mut total_chars = 0;

        for sample in test_samples {
            // Simulate model inference
            let predicted_transcript = self.simulate_inference(&sample.audio_path).await?;

            // Calculate errors
            let (word_errors, word_count) =
                self.calculate_word_errors(&sample.transcript, &predicted_transcript);
            let (char_errors, char_count) =
                self.calculate_character_errors(&sample.transcript, &predicted_transcript);

            total_word_errors += word_errors;
            total_words += word_count;
            total_char_errors += char_errors;
            total_chars += char_count;
        }

        let inference_time = start_time.elapsed().as_millis() as f32 / test_samples.len() as f32;

        // Calculate metrics
        let word_error_rate = if total_words > 0 {
            total_word_errors as f32 / total_words as f32
        } else {
            0.0
        };

        let character_error_rate = if total_chars > 0 {
            total_char_errors as f32 / total_chars as f32
        } else {
            0.0
        };

        // Mock other metrics
        let bleu_score = 1.0 - word_error_rate; // Simplified BLEU approximation
        let perplexity = 10.0 + word_error_rate * 50.0; // Mock perplexity

        let model_size_mb = tokio::fs::metadata(model_path)
            .await
            .map(|meta| meta.len() as f32 / 1024.0 / 1024.0)
            .unwrap_or(0.0);

        Ok(EvaluationMetrics {
            word_error_rate,
            character_error_rate,
            bleu_score,
            perplexity,
            inference_time_ms: inference_time,
            model_size_mb,
        })
    }

    /// Run the actual training process
    async fn run_training(
        &self,
        config: FineTuningConfig,
        train_samples: Vec<TrainingSample>,
        val_samples: Vec<TrainingSample>,
    ) -> Result<(), AppError> {
        let total_batches = train_samples.len().div_ceil(config.batch_size);
        let start_time = std::time::Instant::now();

        for epoch in 0..config.epochs {
            // Check if training should stop
            {
                let is_training = self.is_training.lock().await;
                if !*is_training {
                    tracing::info!("Training stopped by user request");
                    break;
                }
            }

            let mut epoch_training_loss = 0.0;

            // Training phase
            for batch_idx in 0..total_batches {
                let batch_start = batch_idx * config.batch_size;
                let batch_end = (batch_start + config.batch_size).min(train_samples.len());
                let batch = &train_samples[batch_start..batch_end];

                // Simulate training step
                let batch_loss = self.simulate_training_step(batch, &config).await?;
                epoch_training_loss += batch_loss;

                // Update progress
                let elapsed = start_time.elapsed().as_secs();
                let progress_ratio = (epoch * total_batches + batch_idx + 1) as f64
                    / (config.epochs * total_batches) as f64;
                let estimated_total_time = if progress_ratio > 0.0 {
                    (elapsed as f64 / progress_ratio) as u64
                } else {
                    0
                };

                let progress = TrainingProgress {
                    current_epoch: epoch + 1,
                    total_epochs: config.epochs,
                    current_batch: batch_idx + 1,
                    total_batches,
                    training_loss: batch_loss,
                    validation_loss: 0.0, // Will be updated after validation
                    accuracy: 1.0 - batch_loss, // Simplified accuracy
                    elapsed_time_seconds: elapsed,
                    estimated_remaining_seconds: estimated_total_time.saturating_sub(elapsed),
                };

                let mut progress_guard = self.progress.lock().await;
                *progress_guard = Some(progress.clone());
                drop(progress_guard);

                // Store in history
                let mut history = self.training_history.write().await;
                history.push(progress);
                if history.len() > 1000 {
                    history.remove(0);
                }
            }

            // Validation phase
            let validation_loss = self.simulate_validation(&val_samples, &config).await?;

            // Update progress with validation results
            let mut progress_guard = self.progress.lock().await;
            if let Some(ref mut progress) = *progress_guard {
                progress.validation_loss = validation_loss;
            }

            tracing::info!(
                "Epoch {}/{}: training_loss={:.4}, validation_loss={:.4}",
                epoch + 1,
                config.epochs,
                epoch_training_loss / total_batches as f32,
                validation_loss
            );
        }

        // Save the fine-tuned model
        self.save_model(&config.output_model_path).await?;

        tracing::info!(
            "Fine-tuning completed. Model saved to: {}",
            config.output_model_path.display()
        );
        Ok(())
    }

    /// Validate training samples
    async fn validate_training_samples(
        &self,
        samples: Vec<TrainingSample>,
    ) -> Result<Vec<TrainingSample>, AppError> {
        let mut valid_samples = Vec::new();

        for sample in samples {
            // Check if audio file exists
            if !sample.audio_path.exists() {
                tracing::warn!(
                    "Skipping sample with missing audio file: {}",
                    sample.audio_path.display()
                );
                continue;
            }

            // Check if transcript is not empty
            if sample.transcript.trim().is_empty() {
                tracing::warn!("Skipping sample with empty transcript");
                continue;
            }

            valid_samples.push(sample);
        }

        Ok(valid_samples)
    }

    /// Split training data into train and validation sets
    fn split_training_data(
        &self,
        samples: &[TrainingSample],
        validation_split: f32,
    ) -> (Vec<TrainingSample>, Vec<TrainingSample>) {
        let val_size = (samples.len() as f32 * validation_split) as usize;
        let train_size = samples.len() - val_size;

        let mut samples = samples.to_vec();
        // Simple shuffle (in real implementation, use proper randomization)
        samples.reverse();

        let train_samples = samples[..train_size].to_vec();
        let val_samples = samples[train_size..].to_vec();

        (train_samples, val_samples)
    }

    /// Simulate training step (in real implementation, would use actual ML framework)
    async fn simulate_training_step(
        &self,
        _batch: &[TrainingSample],
        _config: &FineTuningConfig,
    ) -> Result<f32, AppError> {
        // Simulate training time
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Return mock loss (decreasing over time)
        Ok(0.5 + rand::random::<f32>() * 0.3)
    }

    /// Simulate validation
    async fn simulate_validation(
        &self,
        _val_samples: &[TrainingSample],
        _config: &FineTuningConfig,
    ) -> Result<f32, AppError> {
        // Simulate validation time
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

        // Return mock validation loss
        Ok(0.4 + rand::random::<f32>() * 0.2)
    }

    /// Simulate model inference
    async fn simulate_inference(&self, _audio_path: &std::path::Path) -> Result<String, AppError> {
        // Simulate inference time
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        // Return mock transcript
        Ok("this is a mock transcription".to_string())
    }

    /// Calculate word-level errors
    fn calculate_word_errors(&self, reference: &str, hypothesis: &str) -> (usize, usize) {
        let ref_words: Vec<&str> = reference.split_whitespace().collect();
        let hyp_words: Vec<&str> = hypothesis.split_whitespace().collect();

        // Simple word error calculation (in real implementation, use edit distance)
        let errors = ref_words
            .iter()
            .zip(hyp_words.iter())
            .filter(|(r, h)| r != h)
            .count();

        (errors, ref_words.len())
    }

    /// Calculate character-level errors
    fn calculate_character_errors(&self, reference: &str, hypothesis: &str) -> (usize, usize) {
        let ref_chars: Vec<char> = reference.chars().collect();
        let hyp_chars: Vec<char> = hypothesis.chars().collect();

        // Simple character error calculation
        let errors = ref_chars
            .iter()
            .zip(hyp_chars.iter())
            .filter(|(r, h)| r != h)
            .count();

        (errors, ref_chars.len())
    }

    /// Save the fine-tuned model
    async fn save_model(&self, output_path: &PathBuf) -> Result<(), AppError> {
        // Create output directory if it doesn't exist
        if let Some(parent) = output_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(AppError::Io)?;
        }

        // In real implementation, would save actual model weights
        let model_data = serde_json::json!({
            "model_type": "fine_tuned_whisper",
            "version": "1.0",
            "timestamp": chrono::Utc::now(),
            "config": *self.config.read().await
        });

        tokio::fs::write(output_path, model_data.to_string())
            .await
            .map_err(AppError::Io)?;

        Ok(())
    }

    /// Get training history
    pub async fn get_training_history(&self) -> Vec<TrainingProgress> {
        let history = self.training_history.read().await;
        history.clone()
    }

    /// Update configuration
    pub async fn update_config(&self, config: FineTuningConfig) {
        let mut config_guard = self.config.write().await;
        *config_guard = config;
    }
}

impl Clone for VoiceModelTuner {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            training_samples: self.training_samples.clone(),
            progress: self.progress.clone(),
            is_training: self.is_training.clone(),
            training_history: self.training_history.clone(),
        }
    }
}

/// Global voice model tuner instance
static VOICE_MODEL_TUNER: tokio::sync::OnceCell<VoiceModelTuner> =
    tokio::sync::OnceCell::const_new();

/// Get the global voice model tuner
pub async fn get_voice_model_tuner() -> &'static VoiceModelTuner {
    VOICE_MODEL_TUNER
        .get_or_init(|| async { VoiceModelTuner::new(FineTuningConfig::default()) })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_training_sample_validation() {
        let temp_dir = TempDir::new().unwrap();
        let audio_path = temp_dir.path().join("test.wav");
        tokio::fs::write(&audio_path, b"mock audio data")
            .await
            .unwrap();

        let tuner = VoiceModelTuner::new(FineTuningConfig::default());

        let sample = TrainingSample {
            audio_path,
            transcript: "test transcript".to_string(),
            speaker_id: None,
            domain: None,
            quality_score: None,
        };

        tuner.add_training_sample(sample).await.unwrap();

        let samples = tuner.training_samples.read().await;
        assert_eq!(samples.len(), 1);
    }

    #[tokio::test]
    async fn test_data_splitting() {
        let tuner = VoiceModelTuner::new(FineTuningConfig::default());

        let samples = vec![
            TrainingSample {
                audio_path: PathBuf::from("1.wav"),
                transcript: "one".to_string(),
                speaker_id: None,
                domain: None,
                quality_score: None,
            },
            TrainingSample {
                audio_path: PathBuf::from("2.wav"),
                transcript: "two".to_string(),
                speaker_id: None,
                domain: None,
                quality_score: None,
            },
        ];

        let (train, val) = tuner.split_training_data(&samples, 0.5);
        assert_eq!(train.len(), 1);
        assert_eq!(val.len(), 1);
    }

    #[test]
    fn test_error_calculation() {
        let tuner = VoiceModelTuner::new(FineTuningConfig::default());

        let reference = "hello world";
        let hypothesis = "hello word";

        let (word_errors, word_count) = tuner.calculate_word_errors(reference, hypothesis);
        assert_eq!(word_errors, 1);
        assert_eq!(word_count, 2);

        let (char_errors, char_count) = tuner.calculate_character_errors(reference, hypothesis);
        assert!(char_errors > 0);
        assert_eq!(char_count, reference.len());
    }
}
