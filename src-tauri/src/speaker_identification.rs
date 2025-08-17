//! Speaker identification and verification for Gestura.app
//! Identifies speakers based on voice characteristics

#[allow(unused_imports)]
use crate::AppError;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Speaker profile containing voice characteristics
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SpeakerProfile {
    pub speaker_id: String,
    pub name: String,
    pub voice_features: VoiceFeatures,
    pub enrollment_samples: usize,
    pub last_updated: chrono::DateTime<chrono::Utc>,
    pub confidence_threshold: f32,
}

/// Voice features for speaker identification
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VoiceFeatures {
    /// Fundamental frequency statistics
    pub f0_mean: f32,
    pub f0_std: f32,
    pub f0_range: (f32, f32),
    
    /// Formant frequencies
    pub formants: Vec<f32>,
    
    /// Spectral features
    pub spectral_centroid: f32,
    pub spectral_rolloff: f32,
    pub spectral_flux: f32,
    
    /// Mel-frequency cepstral coefficients
    pub mfcc: Vec<f32>,
    
    /// Voice quality measures
    pub jitter: f32,
    pub shimmer: f32,
    pub hnr: f32, // Harmonics-to-noise ratio
}

impl Default for VoiceFeatures {
    fn default() -> Self {
        Self {
            f0_mean: 0.0,
            f0_std: 0.0,
            f0_range: (0.0, 0.0),
            formants: vec![0.0; 4],
            spectral_centroid: 0.0,
            spectral_rolloff: 0.0,
            spectral_flux: 0.0,
            mfcc: vec![0.0; 13],
            jitter: 0.0,
            shimmer: 0.0,
            hnr: 0.0,
        }
    }
}

/// Speaker identification result
#[derive(Debug, Clone, serde::Serialize)]
pub struct IdentificationResult {
    pub speaker_id: Option<String>,
    pub speaker_name: Option<String>,
    pub confidence: f32,
    pub alternatives: Vec<(String, f32)>, // (speaker_id, confidence)
}

/// Speaker identification system
pub struct SpeakerIdentifier {
    profiles: Arc<RwLock<HashMap<String, SpeakerProfile>>>,
    feature_extractor: VoiceFeatureExtractor,
    min_enrollment_samples: usize,
    identification_threshold: f32,
}

impl SpeakerIdentifier {
    /// Create a new speaker identifier
    pub fn new(min_enrollment_samples: usize, identification_threshold: f32) -> Self {
        Self {
            profiles: Arc::new(RwLock::new(HashMap::new())),
            feature_extractor: VoiceFeatureExtractor::new(),
            min_enrollment_samples,
            identification_threshold,
        }
    }

    /// Enroll a new speaker
    pub async fn enroll_speaker(&self, speaker_id: String, name: String, audio_samples: Vec<Vec<f32>>) -> Result<(), AppError> {
        if audio_samples.len() < self.min_enrollment_samples {
            return Err(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("Need at least {} samples for enrollment", self.min_enrollment_samples)
            )));
        }

        // Extract features from all samples
        let mut all_features = Vec::new();
        for sample in &audio_samples {
            let features = self.feature_extractor.extract_features(sample)?;
            all_features.push(features);
        }

        // Average features across samples
        let averaged_features = self.average_features(&all_features);

        let profile = SpeakerProfile {
            speaker_id: speaker_id.clone(),
            name,
            voice_features: averaged_features,
            enrollment_samples: audio_samples.len(),
            last_updated: chrono::Utc::now(),
            confidence_threshold: self.identification_threshold,
        };

        let mut profiles = self.profiles.write().await;
        profiles.insert(speaker_id.clone(), profile);

        tracing::info!("Enrolled speaker: {} with {} samples", speaker_id, audio_samples.len());
        Ok(())
    }

    /// Identify speaker from audio sample
    pub async fn identify_speaker(&self, audio_sample: &[f32]) -> Result<IdentificationResult, AppError> {
        // Extract features from the sample
        let sample_features = self.feature_extractor.extract_features(audio_sample)?;

        let profiles = self.profiles.read().await;
        let mut scores = Vec::new();

        // Compare with all enrolled speakers
        for (speaker_id, profile) in profiles.iter() {
            let similarity = self.calculate_similarity(&sample_features, &profile.voice_features);
            scores.push((speaker_id.clone(), profile.name.clone(), similarity));
        }

        // Sort by similarity (highest first)
        scores.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));

        let result = if let Some((best_id, best_name, best_score)) = scores.first() {
            if *best_score >= self.identification_threshold {
                IdentificationResult {
                    speaker_id: Some(best_id.clone()),
                    speaker_name: Some(best_name.clone()),
                    confidence: *best_score,
                    alternatives: scores.iter().skip(1).take(3)
                        .map(|(id, _, score)| (id.clone(), *score))
                        .collect(),
                }
            } else {
                IdentificationResult {
                    speaker_id: None,
                    speaker_name: None,
                    confidence: *best_score,
                    alternatives: scores.iter().take(3)
                        .map(|(id, _, score)| (id.clone(), *score))
                        .collect(),
                }
            }
        } else {
            IdentificationResult {
                speaker_id: None,
                speaker_name: None,
                confidence: 0.0,
                alternatives: Vec::new(),
            }
        };

        Ok(result)
    }

    /// Verify if audio sample matches a specific speaker
    pub async fn verify_speaker(&self, speaker_id: &str, audio_sample: &[f32]) -> Result<f32, AppError> {
        let profiles = self.profiles.read().await;
        
        if let Some(profile) = profiles.get(speaker_id) {
            let sample_features = self.feature_extractor.extract_features(audio_sample)?;
            let similarity = self.calculate_similarity(&sample_features, &profile.voice_features);
            Ok(similarity)
        } else {
            Err(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Speaker not found: {}", speaker_id)
            )))
        }
    }

    /// Update speaker profile with additional samples
    pub async fn update_speaker(&self, speaker_id: &str, audio_samples: Vec<Vec<f32>>) -> Result<(), AppError> {
        let mut profiles = self.profiles.write().await;
        
        if let Some(profile) = profiles.get_mut(speaker_id) {
            // Extract features from new samples
            let mut new_features = Vec::new();
            for sample in &audio_samples {
                let features = self.feature_extractor.extract_features(sample)?;
                new_features.push(features);
            }

            // Combine with existing features (weighted average)
            let existing_weight = profile.enrollment_samples as f32;
            let new_weight = audio_samples.len() as f32;
            let total_weight = existing_weight + new_weight;

            profile.voice_features = self.weighted_average_features(
                &profile.voice_features, existing_weight,
                &self.average_features(&new_features), new_weight,
                total_weight
            );

            profile.enrollment_samples += audio_samples.len();
            profile.last_updated = chrono::Utc::now();

            tracing::info!("Updated speaker {} with {} additional samples", speaker_id, audio_samples.len());
            Ok(())
        } else {
            Err(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Speaker not found: {}", speaker_id)
            )))
        }
    }

    /// Remove speaker profile
    pub async fn remove_speaker(&self, speaker_id: &str) -> Result<(), AppError> {
        let mut profiles = self.profiles.write().await;
        
        if profiles.remove(speaker_id).is_some() {
            tracing::info!("Removed speaker: {}", speaker_id);
            Ok(())
        } else {
            Err(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Speaker not found: {}", speaker_id)
            )))
        }
    }

    /// Get all enrolled speakers
    pub async fn get_speakers(&self) -> Vec<SpeakerProfile> {
        let profiles = self.profiles.read().await;
        profiles.values().cloned().collect()
    }

    /// Calculate similarity between two feature sets
    fn calculate_similarity(&self, features1: &VoiceFeatures, features2: &VoiceFeatures) -> f32 {
        let mut similarity = 0.0;
        let mut weight_sum = 0.0;

        // F0 similarity (weight: 0.2)
        let f0_sim = 1.0 - ((features1.f0_mean - features2.f0_mean).abs() / 
            (features1.f0_mean.max(features2.f0_mean) + 1.0));
        similarity += f0_sim * 0.2;
        weight_sum += 0.2;

        // Formant similarity (weight: 0.3)
        let formant_sim = self.cosine_similarity(&features1.formants, &features2.formants);
        similarity += formant_sim * 0.3;
        weight_sum += 0.3;

        // MFCC similarity (weight: 0.4)
        let mfcc_sim = self.cosine_similarity(&features1.mfcc, &features2.mfcc);
        similarity += mfcc_sim * 0.4;
        weight_sum += 0.4;

        // Spectral similarity (weight: 0.1)
        let spectral_sim = 1.0 - ((features1.spectral_centroid - features2.spectral_centroid).abs() / 
            (features1.spectral_centroid.max(features2.spectral_centroid) + 1.0));
        similarity += spectral_sim * 0.1;
        weight_sum += 0.1;

        similarity / weight_sum
    }

    /// Calculate cosine similarity between two vectors
    fn cosine_similarity(&self, vec1: &[f32], vec2: &[f32]) -> f32 {
        if vec1.len() != vec2.len() {
            return 0.0;
        }

        let dot_product: f32 = vec1.iter().zip(vec2.iter()).map(|(a, b)| a * b).sum();
        let norm1: f32 = vec1.iter().map(|x| x * x).sum::<f32>().sqrt();
        let norm2: f32 = vec2.iter().map(|x| x * x).sum::<f32>().sqrt();

        if norm1 == 0.0 || norm2 == 0.0 {
            0.0
        } else {
            dot_product / (norm1 * norm2)
        }
    }

    /// Average multiple feature sets
    fn average_features(&self, features_list: &[VoiceFeatures]) -> VoiceFeatures {
        if features_list.is_empty() {
            return VoiceFeatures::default();
        }

        let count = features_list.len() as f32;
        let mut averaged = VoiceFeatures::default();

        for features in features_list {
            averaged.f0_mean += features.f0_mean / count;
            averaged.f0_std += features.f0_std / count;
            averaged.spectral_centroid += features.spectral_centroid / count;
            averaged.spectral_rolloff += features.spectral_rolloff / count;
            averaged.spectral_flux += features.spectral_flux / count;
            averaged.jitter += features.jitter / count;
            averaged.shimmer += features.shimmer / count;
            averaged.hnr += features.hnr / count;

            // Average vectors
            for (i, &val) in features.formants.iter().enumerate() {
                if i < averaged.formants.len() {
                    averaged.formants[i] += val / count;
                }
            }

            for (i, &val) in features.mfcc.iter().enumerate() {
                if i < averaged.mfcc.len() {
                    averaged.mfcc[i] += val / count;
                }
            }
        }

        // Calculate F0 range
        let f0_values: Vec<f32> = features_list.iter().map(|f| f.f0_mean).collect();
        averaged.f0_range = (
            f0_values.iter().fold(f32::INFINITY, |a, &b| a.min(b)),
            f0_values.iter().fold(f32::NEG_INFINITY, |a, &b| a.max(b))
        );

        averaged
    }

    /// Weighted average of two feature sets
    fn weighted_average_features(&self, features1: &VoiceFeatures, weight1: f32, 
                                features2: &VoiceFeatures, weight2: f32, total_weight: f32) -> VoiceFeatures {
        let w1 = weight1 / total_weight;
        let w2 = weight2 / total_weight;

        VoiceFeatures {
            f0_mean: features1.f0_mean * w1 + features2.f0_mean * w2,
            f0_std: features1.f0_std * w1 + features2.f0_std * w2,
            f0_range: (
                features1.f0_range.0.min(features2.f0_range.0),
                features1.f0_range.1.max(features2.f0_range.1)
            ),
            formants: features1.formants.iter().zip(features2.formants.iter())
                .map(|(a, b)| a * w1 + b * w2).collect(),
            spectral_centroid: features1.spectral_centroid * w1 + features2.spectral_centroid * w2,
            spectral_rolloff: features1.spectral_rolloff * w1 + features2.spectral_rolloff * w2,
            spectral_flux: features1.spectral_flux * w1 + features2.spectral_flux * w2,
            mfcc: features1.mfcc.iter().zip(features2.mfcc.iter())
                .map(|(a, b)| a * w1 + b * w2).collect(),
            jitter: features1.jitter * w1 + features2.jitter * w2,
            shimmer: features1.shimmer * w1 + features2.shimmer * w2,
            hnr: features1.hnr * w1 + features2.hnr * w2,
        }
    }
}

/// Voice feature extractor
pub struct VoiceFeatureExtractor {
    #[allow(dead_code)]
    sample_rate: f32,
}

impl VoiceFeatureExtractor {
    pub fn new() -> Self {
        Self {
            sample_rate: 16000.0,
        }
    }

    /// Extract voice features from audio sample
    pub fn extract_features(&self, audio: &[f32]) -> Result<VoiceFeatures, AppError> {
        // This is a simplified feature extraction
        // In a real implementation, you would use proper DSP libraries
        
        let mut features = VoiceFeatures::default();

        // Calculate basic statistics
        let mean: f32 = audio.iter().sum::<f32>() / audio.len() as f32;
        let variance: f32 = audio.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / audio.len() as f32;
        
        // Mock feature extraction (replace with real DSP)
        features.f0_mean = 150.0 + (variance * 100.0); // Mock F0
        features.f0_std = variance.sqrt() * 10.0;
        features.f0_range = (100.0, 300.0);
        
        // Mock formants
        features.formants = vec![800.0, 1200.0, 2400.0, 3200.0];
        
        // Mock spectral features
        features.spectral_centroid = 1000.0 + variance * 500.0;
        features.spectral_rolloff = 2000.0 + variance * 1000.0;
        features.spectral_flux = variance;
        
        // Mock MFCC (would normally use FFT and mel filterbank)
        features.mfcc = (0..13).map(|i| variance * (i as f32 + 1.0)).collect();
        
        // Mock voice quality
        features.jitter = variance * 0.01;
        features.shimmer = variance * 0.1;
        features.hnr = 20.0 - variance * 10.0;

        Ok(features)
    }
}

/// Global speaker identifier instance
static SPEAKER_IDENTIFIER: tokio::sync::OnceCell<SpeakerIdentifier> = tokio::sync::OnceCell::const_new();

/// Get the global speaker identifier
pub async fn get_speaker_identifier() -> &'static SpeakerIdentifier {
    SPEAKER_IDENTIFIER.get_or_init(|| async {
        SpeakerIdentifier::new(3, 0.7) // Require 3 samples, 70% confidence threshold
    }).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_speaker_enrollment() {
        let identifier = SpeakerIdentifier::new(2, 0.7);
        
        let samples = vec![
            vec![0.1, 0.2, 0.3, 0.4],
            vec![0.2, 0.3, 0.4, 0.5],
        ];
        
        let result = identifier.enroll_speaker(
            "speaker1".to_string(),
            "Test Speaker".to_string(),
            samples
        ).await;
        
        assert!(result.is_ok());
        
        let speakers = identifier.get_speakers().await;
        assert_eq!(speakers.len(), 1);
        assert_eq!(speakers[0].speaker_id, "speaker1");
    }

    #[tokio::test]
    async fn test_feature_extraction() {
        let extractor = VoiceFeatureExtractor::new();
        let audio = vec![0.1, -0.1, 0.2, -0.2, 0.3, -0.3];
        
        let features = extractor.extract_features(&audio).unwrap();
        assert!(features.f0_mean > 0.0);
        assert_eq!(features.mfcc.len(), 13);
        assert_eq!(features.formants.len(), 4);
    }

    #[test]
    fn test_cosine_similarity() {
        let identifier = SpeakerIdentifier::new(1, 0.5);
        
        let vec1 = vec![1.0, 2.0, 3.0];
        let vec2 = vec![1.0, 2.0, 3.0];
        let similarity = identifier.cosine_similarity(&vec1, &vec2);
        assert!((similarity - 1.0).abs() < 0.001); // Should be 1.0 for identical vectors
        
        let vec3 = vec![0.0, 0.0, 0.0];
        let similarity2 = identifier.cosine_similarity(&vec1, &vec3);
        assert_eq!(similarity2, 0.0); // Should be 0.0 for zero vector
    }
}
