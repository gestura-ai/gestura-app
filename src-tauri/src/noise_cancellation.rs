//! Noise cancellation and audio enhancement for Gestura.app
//! Reduces background noise and enhances speech quality

#[allow(unused_imports)]
use crate::AppError;
use std::collections::VecDeque;

/// Noise cancellation configuration
#[derive(Debug, Clone)]
pub struct NoiseCancellationConfig {
    /// Sample rate in Hz
    pub sample_rate: u32,
    /// Frame size for processing
    pub frame_size: usize,
    /// Noise floor estimation window size
    pub noise_window_size: usize,
    /// Spectral subtraction factor
    pub subtraction_factor: f32,
    /// Minimum gain to prevent over-subtraction
    pub min_gain: f32,
    /// Smoothing factor for gain updates
    pub smoothing_factor: f32,
    /// Enable adaptive noise estimation
    pub adaptive_estimation: bool,
}

impl Default for NoiseCancellationConfig {
    fn default() -> Self {
        Self {
            sample_rate: 16000,
            frame_size: 512,
            noise_window_size: 20,
            subtraction_factor: 2.0,
            min_gain: 0.1,
            smoothing_factor: 0.8,
            adaptive_estimation: true,
        }
    }
}

/// Noise cancellation processor
pub struct NoiseCancellationProcessor {
    config: NoiseCancellationConfig,
    noise_spectrum: Vec<f32>,
    gain_history: VecDeque<Vec<f32>>,
    frame_buffer: Vec<f32>,
    window_function: Vec<f32>,
    noise_frames: VecDeque<Vec<f32>>,
    is_noise_estimated: bool,
}

impl NoiseCancellationProcessor {
    /// Create a new noise cancellation processor
    pub fn new(config: NoiseCancellationConfig) -> Self {
        let window_function = Self::create_hann_window(config.frame_size);
        
        Self {
            noise_spectrum: vec![0.0; config.frame_size / 2 + 1],
            gain_history: VecDeque::with_capacity(10),
            frame_buffer: Vec::with_capacity(config.frame_size * 2),
            window_function,
            noise_frames: VecDeque::with_capacity(config.noise_window_size),
            is_noise_estimated: false,
            config,
        }
    }

    /// Process audio frame with noise cancellation
    pub fn process_frame(&mut self, input_frame: &[f32]) -> Result<Vec<f32>, AppError> {
        if input_frame.len() != self.config.frame_size {
            return Err(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("Frame size mismatch: expected {}, got {}", 
                    self.config.frame_size, input_frame.len())
            )));
        }

        // Apply window function
        let windowed_frame: Vec<f32> = input_frame.iter()
            .zip(self.window_function.iter())
            .map(|(sample, window)| sample * window)
            .collect();

        // Compute FFT (simplified - in real implementation use proper FFT library)
        let spectrum = self.compute_spectrum(&windowed_frame);
        
        // Update noise estimation if needed
        if !self.is_noise_estimated || self.config.adaptive_estimation {
            self.update_noise_estimation(&spectrum);
        }

        // Apply spectral subtraction
        let enhanced_spectrum = self.apply_spectral_subtraction(&spectrum);

        // Convert back to time domain (simplified IFFT)
        let enhanced_frame = self.spectrum_to_time_domain(&enhanced_spectrum);

        Ok(enhanced_frame)
    }

    /// Process streaming audio with overlap-add
    pub fn process_stream(&mut self, audio_data: &[f32]) -> Result<Vec<f32>, AppError> {
        let mut output = Vec::new();
        
        // Add new data to buffer
        self.frame_buffer.extend_from_slice(audio_data);
        
        // Process complete frames with 50% overlap
        let hop_size = self.config.frame_size / 2;
        
        while self.frame_buffer.len() >= self.config.frame_size {
            let frame: Vec<f32> = self.frame_buffer.iter().take(self.config.frame_size).cloned().collect();
            let processed_frame = self.process_frame(&frame)?;
            
            // Overlap-add
            if output.len() < hop_size {
                output.extend_from_slice(&processed_frame[..hop_size]);
            } else {
                let start_idx = output.len() - hop_size;
                for (i, &sample) in processed_frame.iter().take(hop_size).enumerate() {
                    output[start_idx + i] += sample;
                }
                output.extend_from_slice(&processed_frame[hop_size..]);
            }
            
            // Remove processed samples
            self.frame_buffer.drain(..hop_size);
        }
        
        Ok(output)
    }

    /// Estimate noise spectrum from initial frames
    pub fn estimate_noise(&mut self, noise_samples: &[Vec<f32>]) -> Result<(), AppError> {
        if noise_samples.is_empty() {
            return Err(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Need at least one noise sample"
            )));
        }

        let mut accumulated_spectrum = vec![0.0; self.config.frame_size / 2 + 1];
        
        for sample in noise_samples {
            if sample.len() != self.config.frame_size {
                continue;
            }
            
            let windowed: Vec<f32> = sample.iter()
                .zip(self.window_function.iter())
                .map(|(s, w)| s * w)
                .collect();
            
            let spectrum = self.compute_spectrum(&windowed);
            
            for (i, &mag) in spectrum.iter().enumerate() {
                accumulated_spectrum[i] += mag;
            }
        }

        // Average the accumulated spectrum
        let count = noise_samples.len() as f32;
        for magnitude in accumulated_spectrum.iter_mut() {
            *magnitude /= count;
        }

        self.noise_spectrum = accumulated_spectrum;
        self.is_noise_estimated = true;
        
        tracing::info!("Noise spectrum estimated from {} samples", noise_samples.len());
        Ok(())
    }

    /// Update noise estimation adaptively
    fn update_noise_estimation(&mut self, current_spectrum: &[f32]) {
        if !self.config.adaptive_estimation {
            return;
        }

        // Store current frame for noise estimation
        self.noise_frames.push_back(current_spectrum.to_vec());
        
        if self.noise_frames.len() > self.config.noise_window_size {
            self.noise_frames.pop_front();
        }

        // Update noise spectrum (simple moving average)
        if self.noise_frames.len() >= self.config.noise_window_size / 2 {
            let mut updated_noise = vec![0.0; current_spectrum.len()];
            
            for frame in &self.noise_frames {
                for (i, &mag) in frame.iter().enumerate() {
                    updated_noise[i] += mag;
                }
            }
            
            let count = self.noise_frames.len() as f32;
            for magnitude in updated_noise.iter_mut() {
                *magnitude /= count;
            }
            
            // Smooth update with existing noise estimate
            if self.is_noise_estimated {
                let alpha = 0.1; // Update rate
                for (i, &new_mag) in updated_noise.iter().enumerate() {
                    self.noise_spectrum[i] = (1.0 - alpha) * self.noise_spectrum[i] + alpha * new_mag;
                }
            } else {
                self.noise_spectrum = updated_noise;
                self.is_noise_estimated = true;
            }
        }
    }

    /// Apply spectral subtraction for noise reduction
    fn apply_spectral_subtraction(&mut self, input_spectrum: &[f32]) -> Vec<f32> {
        let mut enhanced_spectrum = Vec::with_capacity(input_spectrum.len());
        let mut current_gains = Vec::with_capacity(input_spectrum.len());
        
        for (i, &input_mag) in input_spectrum.iter().enumerate() {
            let noise_mag = if i < self.noise_spectrum.len() {
                self.noise_spectrum[i]
            } else {
                0.0
            };
            
            // Spectral subtraction
            let subtracted_mag = input_mag - self.config.subtraction_factor * noise_mag;
            
            // Calculate gain
            let gain = if input_mag > 0.0 {
                (subtracted_mag / input_mag).max(self.config.min_gain)
            } else {
                self.config.min_gain
            };
            
            current_gains.push(gain);
            enhanced_spectrum.push(input_mag * gain);
        }
        
        // Apply gain smoothing
        if let Some(previous_gains) = self.gain_history.back() {
            for (i, gain) in current_gains.iter_mut().enumerate() {
                if i < previous_gains.len() {
                    *gain = self.config.smoothing_factor * previous_gains[i] + 
                           (1.0 - self.config.smoothing_factor) * *gain;
                    enhanced_spectrum[i] = input_spectrum[i] * *gain;
                }
            }
        }
        
        // Store gains for next frame
        self.gain_history.push_back(current_gains);
        if self.gain_history.len() > 5 {
            self.gain_history.pop_front();
        }
        
        enhanced_spectrum
    }

    /// Compute magnitude spectrum (simplified)
    fn compute_spectrum(&self, frame: &[f32]) -> Vec<f32> {
        // This is a simplified spectrum computation
        // In a real implementation, use proper FFT library like rustfft
        
        let mut spectrum = Vec::with_capacity(self.config.frame_size / 2 + 1);
        
        for k in 0..=self.config.frame_size / 2 {
            let mut real = 0.0;
            let mut imag = 0.0;
            
            for (n, &sample) in frame.iter().enumerate() {
                let angle = -2.0 * std::f32::consts::PI * (k as f32) * (n as f32) / (self.config.frame_size as f32);
                real += sample * angle.cos();
                imag += sample * angle.sin();
            }
            
            let magnitude = (real * real + imag * imag).sqrt();
            spectrum.push(magnitude);
        }
        
        spectrum
    }

    /// Convert spectrum back to time domain (simplified IFFT)
    fn spectrum_to_time_domain(&self, spectrum: &[f32]) -> Vec<f32> {
        // This is a simplified inverse transform
        // In a real implementation, use proper IFFT
        
        let mut time_domain = vec![0.0; self.config.frame_size];
        
        for (n, sample) in time_domain.iter_mut().enumerate() {
            for (k, &magnitude) in spectrum.iter().enumerate() {
                let angle = 2.0 * std::f32::consts::PI * (k as f32) * (n as f32) / (self.config.frame_size as f32);
                *sample += magnitude * angle.cos() / (self.config.frame_size as f32);
            }
        }
        
        // Apply window function again
        for (i, sample) in time_domain.iter_mut().enumerate() {
            *sample *= self.window_function[i];
        }
        
        time_domain
    }

    /// Create Hann window function
    fn create_hann_window(size: usize) -> Vec<f32> {
        (0..size)
            .map(|n| {
                0.5 * (1.0 - (2.0 * std::f32::consts::PI * n as f32 / (size - 1) as f32).cos())
            })
            .collect()
    }

    /// Get noise reduction statistics
    pub fn get_stats(&self) -> NoiseReductionStats {
        let noise_level = if !self.noise_spectrum.is_empty() {
            self.noise_spectrum.iter().sum::<f32>() / self.noise_spectrum.len() as f32
        } else {
            0.0
        };

        NoiseReductionStats {
            is_noise_estimated: self.is_noise_estimated,
            noise_level,
            frames_processed: self.gain_history.len(),
            subtraction_factor: self.config.subtraction_factor,
            min_gain: self.config.min_gain,
        }
    }

    /// Reset processor state
    pub fn reset(&mut self) {
        self.noise_spectrum.fill(0.0);
        self.gain_history.clear();
        self.frame_buffer.clear();
        self.noise_frames.clear();
        self.is_noise_estimated = false;
    }

    /// Update configuration
    pub fn update_config(&mut self, config: NoiseCancellationConfig) {
        if config.frame_size != self.config.frame_size {
            // Frame size changed, need to recreate window and reset state
            self.window_function = Self::create_hann_window(config.frame_size);
            self.noise_spectrum = vec![0.0; config.frame_size / 2 + 1];
            self.reset();
        }
        
        self.config = config;
    }
}

/// Noise reduction statistics
#[derive(Debug, Clone, serde::Serialize)]
pub struct NoiseReductionStats {
    pub is_noise_estimated: bool,
    pub noise_level: f32,
    pub frames_processed: usize,
    pub subtraction_factor: f32,
    pub min_gain: f32,
}

/// Create a noise cancellation processor with speech-optimized settings
pub fn create_speech_noise_canceller() -> NoiseCancellationProcessor {
    let config = NoiseCancellationConfig {
        sample_rate: 16000,
        frame_size: 512,
        noise_window_size: 30,
        subtraction_factor: 1.5,
        min_gain: 0.15,
        smoothing_factor: 0.85,
        adaptive_estimation: true,
    };
    NoiseCancellationProcessor::new(config)
}

/// Create a noise cancellation processor with music-optimized settings
pub fn create_music_noise_canceller() -> NoiseCancellationProcessor {
    let config = NoiseCancellationConfig {
        sample_rate: 44100,
        frame_size: 1024,
        noise_window_size: 50,
        subtraction_factor: 1.2,
        min_gain: 0.2,
        smoothing_factor: 0.9,
        adaptive_estimation: true,
    };
    NoiseCancellationProcessor::new(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_noise_canceller_creation() {
        let config = NoiseCancellationConfig::default();
        let processor = NoiseCancellationProcessor::new(config);
        
        let stats = processor.get_stats();
        assert!(!stats.is_noise_estimated);
        assert_eq!(stats.frames_processed, 0);
    }

    #[test]
    fn test_hann_window() {
        let window = NoiseCancellationProcessor::create_hann_window(4);
        assert_eq!(window.len(), 4);
        assert!(window[0] < window[1]); // Should increase from edges
        assert!(window[2] > window[3]); // Should decrease to edges
    }

    #[test]
    fn test_noise_estimation() {
        let config = NoiseCancellationConfig {
            frame_size: 4,
            ..NoiseCancellationConfig::default()
        };
        let mut processor = NoiseCancellationProcessor::new(config);
        
        let noise_samples = vec![
            vec![0.1, 0.1, 0.1, 0.1],
            vec![0.2, 0.2, 0.2, 0.2],
        ];
        
        processor.estimate_noise(&noise_samples).unwrap();
        
        let stats = processor.get_stats();
        assert!(stats.is_noise_estimated);
        assert!(stats.noise_level > 0.0);
    }

    #[test]
    fn test_frame_processing() {
        let config = NoiseCancellationConfig {
            frame_size: 4,
            ..NoiseCancellationConfig::default()
        };
        let mut processor = NoiseCancellationProcessor::new(config);
        
        // Estimate noise first
        let noise_samples = vec![vec![0.01, 0.01, 0.01, 0.01]];
        processor.estimate_noise(&noise_samples).unwrap();
        
        // Process a frame
        let input_frame = vec![0.5, 0.4, 0.3, 0.2];
        let output_frame = processor.process_frame(&input_frame).unwrap();
        
        assert_eq!(output_frame.len(), input_frame.len());
    }

    #[test]
    fn test_speech_optimized_settings() {
        let processor = create_speech_noise_canceller();
        let stats = processor.get_stats();
        
        assert_eq!(stats.subtraction_factor, 1.5);
        assert_eq!(stats.min_gain, 0.15);
    }
}
