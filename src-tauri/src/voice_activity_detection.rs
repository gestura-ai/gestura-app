//! Voice Activity Detection (VAD) for Gestura.app
//! Detects when speech is present in audio streams

#[allow(unused_imports)]
use crate::AppError;
use std::collections::VecDeque;

/// Voice activity detection result
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub enum VadResult {
    Speech,
    Silence,
    Unknown,
}

/// VAD configuration
#[derive(Debug, Clone)]
pub struct VadConfig {
    /// Sample rate in Hz
    pub sample_rate: u32,
    /// Frame size in samples
    pub frame_size: usize,
    /// Energy threshold for speech detection
    pub energy_threshold: f32,
    /// Zero crossing rate threshold
    pub zcr_threshold: f32,
    /// Minimum speech duration in frames
    pub min_speech_frames: usize,
    /// Minimum silence duration in frames
    pub min_silence_frames: usize,
    /// Smoothing window size
    pub smoothing_window: usize,
}

impl Default for VadConfig {
    fn default() -> Self {
        Self {
            sample_rate: 16000,
            frame_size: 320, // 20ms at 16kHz
            energy_threshold: 0.01,
            zcr_threshold: 0.3,
            min_speech_frames: 5,
            min_silence_frames: 10,
            smoothing_window: 5,
        }
    }
}

/// Voice Activity Detector
pub struct VoiceActivityDetector {
    config: VadConfig,
    energy_history: VecDeque<f32>,
    zcr_history: VecDeque<f32>,
    decision_history: VecDeque<VadResult>,
    current_state: VadResult,
    state_duration: usize,
}

impl VoiceActivityDetector {
    /// Create a new VAD instance
    pub fn new(config: VadConfig) -> Self {
        Self {
            config: config.clone(),
            energy_history: VecDeque::with_capacity(config.smoothing_window),
            zcr_history: VecDeque::with_capacity(config.smoothing_window),
            decision_history: VecDeque::with_capacity(config.smoothing_window),
            current_state: VadResult::Silence,
            state_duration: 0,
        }
    }

    /// Process audio frame and return VAD result
    pub fn process_frame(&mut self, audio_frame: &[f32]) -> VadResult {
        if audio_frame.len() != self.config.frame_size {
            tracing::warn!("Frame size mismatch: expected {}, got {}", 
                self.config.frame_size, audio_frame.len());
            return VadResult::Unknown;
        }

        // Calculate features
        let energy = self.calculate_energy(audio_frame);
        let zcr = self.calculate_zero_crossing_rate(audio_frame);

        // Update history
        self.update_history(energy, zcr);

        // Make decision
        let raw_decision = self.make_decision(energy, zcr);
        let smoothed_decision = self.apply_smoothing(raw_decision);

        // Update state tracking
        self.update_state(smoothed_decision.clone());

        smoothed_decision
    }

    /// Calculate frame energy
    fn calculate_energy(&self, frame: &[f32]) -> f32 {
        let sum_squares: f32 = frame.iter().map(|&x| x * x).sum();
        sum_squares / frame.len() as f32
    }

    /// Calculate zero crossing rate
    fn calculate_zero_crossing_rate(&self, frame: &[f32]) -> f32 {
        let mut crossings = 0;
        for i in 1..frame.len() {
            if (frame[i] >= 0.0) != (frame[i-1] >= 0.0) {
                crossings += 1;
            }
        }
        crossings as f32 / (frame.len() - 1) as f32
    }

    /// Update feature history
    fn update_history(&mut self, energy: f32, zcr: f32) {
        // Add new values
        self.energy_history.push_back(energy);
        self.zcr_history.push_back(zcr);

        // Remove old values if window is full
        if self.energy_history.len() > self.config.smoothing_window {
            self.energy_history.pop_front();
        }
        if self.zcr_history.len() > self.config.smoothing_window {
            self.zcr_history.pop_front();
        }
    }

    /// Make raw VAD decision based on features
    fn make_decision(&self, energy: f32, zcr: f32) -> VadResult {
        // Simple threshold-based decision
        if energy > self.config.energy_threshold && zcr < self.config.zcr_threshold {
            VadResult::Speech
        } else {
            VadResult::Silence
        }
    }

    /// Apply temporal smoothing to reduce false positives
    fn apply_smoothing(&mut self, raw_decision: VadResult) -> VadResult {
        self.decision_history.push_back(raw_decision);
        
        if self.decision_history.len() > self.config.smoothing_window {
            self.decision_history.pop_front();
        }

        // Count speech vs silence decisions in window
        let speech_count = self.decision_history.iter()
            .filter(|&d| *d == VadResult::Speech)
            .count();
        
        let silence_count = self.decision_history.iter()
            .filter(|&d| *d == VadResult::Silence)
            .count();

        // Majority vote
        if speech_count > silence_count {
            VadResult::Speech
        } else {
            VadResult::Silence
        }
    }

    /// Update state tracking with minimum duration constraints
    fn update_state(&mut self, decision: VadResult) {
        if decision == self.current_state {
            self.state_duration += 1;
        } else {
            // State change requested
            let min_duration = match self.current_state {
                VadResult::Speech => self.config.min_speech_frames,
                VadResult::Silence => self.config.min_silence_frames,
                VadResult::Unknown => 1,
            };

            if self.state_duration >= min_duration {
                // Allow state change
                self.current_state = decision;
                self.state_duration = 1;
            } else {
                // Stay in current state
                self.state_duration += 1;
            }
        }
    }

    /// Get current VAD state
    pub fn get_current_state(&self) -> VadResult {
        self.current_state.clone()
    }

    /// Get state duration in frames
    pub fn get_state_duration(&self) -> usize {
        self.state_duration
    }

    /// Reset VAD state
    pub fn reset(&mut self) {
        self.energy_history.clear();
        self.zcr_history.clear();
        self.decision_history.clear();
        self.current_state = VadResult::Silence;
        self.state_duration = 0;
    }

    /// Get VAD statistics
    pub fn get_stats(&self) -> VadStats {
        let avg_energy = if !self.energy_history.is_empty() {
            self.energy_history.iter().sum::<f32>() / self.energy_history.len() as f32
        } else {
            0.0
        };

        let avg_zcr = if !self.zcr_history.is_empty() {
            self.zcr_history.iter().sum::<f32>() / self.zcr_history.len() as f32
        } else {
            0.0
        };

        VadStats {
            current_state: self.current_state.clone(),
            state_duration: self.state_duration,
            average_energy: avg_energy,
            average_zcr: avg_zcr,
            energy_threshold: self.config.energy_threshold,
            zcr_threshold: self.config.zcr_threshold,
        }
    }

    /// Update configuration
    pub fn update_config(&mut self, config: VadConfig) {
        self.config = config;
        // Clear history as frame size might have changed
        self.reset();
    }
}

/// VAD statistics
#[derive(Debug, Clone, serde::Serialize)]
pub struct VadStats {
    pub current_state: VadResult,
    pub state_duration: usize,
    pub average_energy: f32,
    pub average_zcr: f32,
    pub energy_threshold: f32,
    pub zcr_threshold: f32,
}

/// Streaming VAD processor for continuous audio
pub struct StreamingVad {
    vad: VoiceActivityDetector,
    buffer: Vec<f32>,
    speech_segments: Vec<SpeechSegment>,
    current_segment: Option<SpeechSegment>,
}

/// Speech segment information
#[derive(Debug, Clone, serde::Serialize)]
pub struct SpeechSegment {
    pub start_frame: usize,
    pub end_frame: Option<usize>,
    pub duration_frames: usize,
    pub confidence: f32,
}

impl StreamingVad {
    /// Create a new streaming VAD
    pub fn new(config: VadConfig) -> Self {
        Self {
            vad: VoiceActivityDetector::new(config.clone()),
            buffer: Vec::with_capacity(config.frame_size * 2),
            speech_segments: Vec::new(),
            current_segment: None,
        }
    }

    /// Process streaming audio data
    pub fn process_audio(&mut self, audio_data: &[f32]) -> Vec<SpeechSegment> {
        let mut completed_segments = Vec::new();
        
        // Add new data to buffer
        self.buffer.extend_from_slice(audio_data);
        
        // Process complete frames
        while self.buffer.len() >= self.vad.config.frame_size {
            let frame: Vec<f32> = self.buffer.drain(..self.vad.config.frame_size).collect();
            let result = self.vad.process_frame(&frame);
            
            self.update_segments(result, &mut completed_segments);
        }
        
        completed_segments
    }

    /// Update speech segments based on VAD result
    fn update_segments(&mut self, vad_result: VadResult, completed_segments: &mut Vec<SpeechSegment>) {
        let current_frame = self.speech_segments.len() + 
            self.current_segment.as_ref().map(|s| s.duration_frames).unwrap_or(0);

        match vad_result {
            VadResult::Speech => {
                if self.current_segment.is_none() {
                    // Start new speech segment
                    self.current_segment = Some(SpeechSegment {
                        start_frame: current_frame,
                        end_frame: None,
                        duration_frames: 1,
                        confidence: 0.8, // TODO: Calculate actual confidence
                    });
                } else {
                    // Continue current segment
                    if let Some(ref mut segment) = self.current_segment {
                        segment.duration_frames += 1;
                    }
                }
            }
            VadResult::Silence => {
                if let Some(mut segment) = self.current_segment.take() {
                    // End current speech segment
                    segment.end_frame = Some(current_frame);
                    completed_segments.push(segment.clone());
                    self.speech_segments.push(segment);
                }
            }
            VadResult::Unknown => {
                // Continue current state
                if let Some(ref mut segment) = self.current_segment {
                    segment.duration_frames += 1;
                }
            }
        }
    }

    /// Get all detected speech segments
    pub fn get_speech_segments(&self) -> &[SpeechSegment] {
        &self.speech_segments
    }

    /// Get current segment if active
    pub fn get_current_segment(&self) -> Option<&SpeechSegment> {
        self.current_segment.as_ref()
    }

    /// Reset streaming VAD
    pub fn reset(&mut self) {
        self.vad.reset();
        self.buffer.clear();
        self.speech_segments.clear();
        self.current_segment = None;
    }

    /// Get VAD statistics
    pub fn get_stats(&self) -> VadStats {
        self.vad.get_stats()
    }
}

/// Create a VAD with optimal settings for speech recognition
pub fn create_speech_vad() -> VoiceActivityDetector {
    let config = VadConfig {
        sample_rate: 16000,
        frame_size: 320, // 20ms
        energy_threshold: 0.005,
        zcr_threshold: 0.35,
        min_speech_frames: 3,
        min_silence_frames: 8,
        smoothing_window: 5,
    };
    VoiceActivityDetector::new(config)
}

/// Create a VAD with settings optimized for wake word detection
pub fn create_wake_word_vad() -> VoiceActivityDetector {
    let config = VadConfig {
        sample_rate: 16000,
        frame_size: 160, // 10ms for faster response
        energy_threshold: 0.002, // More sensitive
        zcr_threshold: 0.4,
        min_speech_frames: 2,
        min_silence_frames: 5,
        smoothing_window: 3,
    };
    VoiceActivityDetector::new(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vad_creation() {
        let config = VadConfig::default();
        let vad = VoiceActivityDetector::new(config);
        assert_eq!(vad.get_current_state(), VadResult::Silence);
    }

    #[test]
    fn test_energy_calculation() {
        let config = VadConfig::default();
        let vad = VoiceActivityDetector::new(config);
        
        let frame = vec![0.1, -0.1, 0.2, -0.2]; // Simple test frame
        let energy = vad.calculate_energy(&frame);
        assert!(energy > 0.0);
    }

    #[test]
    fn test_zcr_calculation() {
        let config = VadConfig::default();
        let vad = VoiceActivityDetector::new(config);
        
        let frame = vec![1.0, -1.0, 1.0, -1.0]; // Alternating signal
        let zcr = vad.calculate_zero_crossing_rate(&frame);
        assert_eq!(zcr, 1.0); // Every sample crosses zero
    }

    #[test]
    fn test_streaming_vad() {
        let config = VadConfig {
            frame_size: 4,
            ..VadConfig::default()
        };
        let mut streaming_vad = StreamingVad::new(config);
        
        // Process some audio data
        let audio_data = vec![0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8];
        let _segments = streaming_vad.process_audio(&audio_data);
        
        // Should process 2 complete frames
        assert_eq!(streaming_vad.buffer.len(), 0); // All data processed
    }
}
