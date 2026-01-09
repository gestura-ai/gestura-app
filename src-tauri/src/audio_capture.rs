//! Audio capture module for microphone input
//! Uses cpal for cross-platform audio recording with voice activity detection (VAD)

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Silence detection configuration
const SILENCE_THRESHOLD: f32 = 0.02; // RMS threshold for detecting silence
const SILENCE_TIMEOUT_SECS: f32 = 4.0; // Stop recording after 4 seconds of silence
const MAX_RECORDING_SECS: u64 = 120; // Maximum recording duration (2 minutes)
const VAD_WINDOW_MS: u64 = 100; // Window size for VAD analysis

/// Record audio from the microphone until user stops speaking (4 seconds of silence)
/// Returns the duration of recorded audio in seconds
///
/// This function runs the entire recording process in a blocking task
/// to avoid Send/Sync issues with cpal::Stream
pub async fn record_audio(_duration: Duration, output_path: &Path) -> Result<f32, crate::AppError> {
    let output_path = output_path.to_path_buf();

    // Get the configured audio device from config
    let config = crate::config::AppConfig::load();
    let device_name = config.voice.audio_device.clone();

    // Run the entire recording process in a blocking task
    // because cpal::Stream is not Send
    tokio::task::spawn_blocking(move || record_audio_with_vad(&output_path, device_name.as_deref()))
        .await
        .map_err(|e| crate::AppError::Voice(format!("Recording task failed: {}", e)))?
}

/// Calculate RMS (Root Mean Square) of audio samples - a measure of audio energy
fn calculate_rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_squares: f32 = samples.iter().map(|s| s * s).sum();
    (sum_squares / samples.len() as f32).sqrt()
}

/// Voice Activity Detection state
struct VadState {
    last_speech_time: Instant,
    has_detected_speech: bool,
    recording_start: Instant,
    has_logged_max_duration: bool,
}

impl VadState {
    fn new() -> Self {
        let now = Instant::now();
        Self {
            last_speech_time: now,
            has_detected_speech: false,
            recording_start: now,
            has_logged_max_duration: false,
        }
    }
}

/// Record audio with Voice Activity Detection - stops after 4 seconds of silence
fn record_audio_with_vad(output_path: &Path, device_name: Option<&str>) -> Result<f32, crate::AppError> {
    // Get default audio host
    let host = cpal::default_host();

    // Try to find the specified device, or fall back to default
    let device = if let Some(name) = device_name {
        let found = host.input_devices()
            .ok()
            .and_then(|mut devices| devices.find(|d| d.name().ok().as_deref() == Some(name)));

        if let Some(dev) = found {
            tracing::info!("Using configured audio input device: {}", name);
            dev
        } else {
            tracing::warn!("Configured device '{}' not found, using default", name);
            host.default_input_device()
                .ok_or_else(|| crate::AppError::Voice("No input device available".into()))?
        }
    } else {
        host.default_input_device()
            .ok_or_else(|| crate::AppError::Voice("No input device available".into()))?
    };

    tracing::info!("Using audio input device: {:?}", device.name());

    // Get supported config
    let config = device
        .default_input_config()
        .map_err(|e| crate::AppError::Voice(format!("Failed to get config: {}", e)))?;

    let sample_rate = config.sample_rate().0;
    let channels = config.channels();

    tracing::info!("Audio config: {}Hz, {} channels", sample_rate, channels);

    // Create shared buffer for samples
    let samples: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    let samples_clone = Arc::clone(&samples);

    // Shared VAD state
    let vad_state = Arc::new(Mutex::new(VadState::new()));
    let vad_state_clone = Arc::clone(&vad_state);

    // Flag to signal when to stop recording
    let should_stop = Arc::new(AtomicBool::new(false));
    let should_stop_clone = Arc::clone(&should_stop);

    // Samples per VAD window
    let samples_per_window = (sample_rate as u64 * channels as u64 * VAD_WINDOW_MS / 1000) as usize;

    // Buffer for VAD analysis
    let vad_buffer: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    let vad_buffer_clone = Arc::clone(&vad_buffer);

    // Build input stream based on sample format
    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => device
            .build_input_stream(
                &config.into(),
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    // Store all samples
                    {
                        let mut buffer = samples_clone.lock().unwrap();
                        buffer.extend_from_slice(data);
                    }

                    // Add to VAD buffer for analysis
                    {
                        let mut vad_buf = vad_buffer_clone.lock().unwrap();
                        vad_buf.extend_from_slice(data);

                        // Analyze when we have enough samples
                        while vad_buf.len() >= samples_per_window {
                            let window: Vec<f32> = vad_buf.drain(..samples_per_window).collect();
                            let rms = calculate_rms(&window);

                            let mut state = vad_state_clone.lock().unwrap();
                            let now = Instant::now();

                            if rms > SILENCE_THRESHOLD {
                                // Speech detected
                                state.last_speech_time = now;
                                if !state.has_detected_speech {
                                    state.has_detected_speech = true;
                                    tracing::info!("Speech detected (RMS: {:.4})", rms);
                                }
                            } else if state.has_detected_speech {
                                // Check if silence timeout reached
                                let silence_duration = now.duration_since(state.last_speech_time);
                                if silence_duration.as_secs_f32() >= SILENCE_TIMEOUT_SECS {
                                    tracing::info!("Silence timeout reached ({:.1}s)", silence_duration.as_secs_f32());
                                    should_stop_clone.store(true, Ordering::SeqCst);
                                }
                            }

                            // Check max recording duration (only log once)
                            if now.duration_since(state.recording_start).as_secs() >= MAX_RECORDING_SECS {
                                if !state.has_logged_max_duration {
                                    tracing::info!("Max recording duration reached");
                                    state.has_logged_max_duration = true;
                                }
                                should_stop_clone.store(true, Ordering::SeqCst);
                            }
                        }
                    }
                },
                |err| {
                    tracing::error!("Audio stream error: {}", err);
                },
                None,
            )
            .map_err(|e| crate::AppError::Voice(format!("Failed to build stream: {}", e)))?,
        cpal::SampleFormat::I16 => {
            let samples_clone_i16 = Arc::clone(&samples);
            let vad_buffer_i16 = Arc::clone(&vad_buffer);
            let vad_state_i16 = Arc::clone(&vad_state);
            let should_stop_i16 = Arc::clone(&should_stop);

            device
                .build_input_stream(
                    &config.into(),
                    move |data: &[i16], _: &cpal::InputCallbackInfo| {
                        let f32_data: Vec<f32> =
                            data.iter().map(|&s| s as f32 / i16::MAX as f32).collect();

                        // Store all samples
                        {
                            let mut buffer = samples_clone_i16.lock().unwrap();
                            buffer.extend_from_slice(&f32_data);
                        }

                        // Add to VAD buffer for analysis
                        {
                            let mut vad_buf = vad_buffer_i16.lock().unwrap();
                            vad_buf.extend_from_slice(&f32_data);

                            while vad_buf.len() >= samples_per_window {
                                let window: Vec<f32> = vad_buf.drain(..samples_per_window).collect();
                                let rms = calculate_rms(&window);

                                let mut state = vad_state_i16.lock().unwrap();
                                let now = Instant::now();

                                if rms > SILENCE_THRESHOLD {
                                    state.last_speech_time = now;
                                    if !state.has_detected_speech {
                                        state.has_detected_speech = true;
                                        tracing::info!("Speech detected (RMS: {:.4})", rms);
                                    }
                                } else if state.has_detected_speech {
                                    let silence_duration = now.duration_since(state.last_speech_time);
                                    if silence_duration.as_secs_f32() >= SILENCE_TIMEOUT_SECS {
                                        tracing::info!("Silence timeout reached ({:.1}s)", silence_duration.as_secs_f32());
                                        should_stop_i16.store(true, Ordering::SeqCst);
                                    }
                                }

                                // Check max recording duration (only log once)
                                if now.duration_since(state.recording_start).as_secs() >= MAX_RECORDING_SECS {
                                    if !state.has_logged_max_duration {
                                        tracing::info!("Max recording duration reached");
                                        state.has_logged_max_duration = true;
                                    }
                                    should_stop_i16.store(true, Ordering::SeqCst);
                                }
                            }
                        }
                    },
                    |err| {
                        tracing::error!("Audio stream error: {}", err);
                    },
                    None,
                )
                .map_err(|e| crate::AppError::Voice(format!("Failed to build stream: {}", e)))?
        }
        _ => return Err(crate::AppError::Voice("Unsupported sample format".into())),
    };

    // Start recording
    stream
        .play()
        .map_err(|e| crate::AppError::Voice(format!("Failed to start stream: {}", e)))?;

    tracing::info!("Recording with VAD - will stop after {}s of silence...", SILENCE_TIMEOUT_SECS);

    // Wait for speech to end (with silence timeout) or max duration
    loop {
        std::thread::sleep(Duration::from_millis(100));

        if should_stop.load(Ordering::SeqCst) {
            break;
        }

        // Also check for max duration from main thread
        let state = vad_state.lock().unwrap();
        if state.recording_start.elapsed().as_secs() >= MAX_RECORDING_SECS {
            break;
        }
    }

    // Stop recording
    drop(stream);

    // Get recorded samples
    let recorded_samples = samples.lock().unwrap();
    let sample_count = recorded_samples.len();
    let duration_secs = sample_count as f32 / (sample_rate as f32 * channels as f32);

    tracing::info!("Recorded {} samples ({:.2}s)", sample_count, duration_secs);

    if sample_count == 0 {
        return Err(crate::AppError::Voice("No audio captured".into()));
    }

    // Save to WAV file
    save_samples_to_wav(&recorded_samples, sample_rate, channels, output_path)?;

    Ok(duration_secs)
}

/// Save audio samples to a WAV file
fn save_samples_to_wav(
    samples: &[f32],
    sample_rate: u32,
    channels: u16,
    path: &Path,
) -> Result<(), crate::AppError> {
    let spec = hound::WavSpec {
        channels,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };

    let mut writer = hound::WavWriter::create(path, spec)
        .map_err(|e| crate::AppError::Voice(format!("Failed to create WAV: {}", e)))?;

    // Convert f32 samples to i16
    for sample in samples {
        let sample_i16 = (*sample * i16::MAX as f32) as i16;
        writer
            .write_sample(sample_i16)
            .map_err(|e| crate::AppError::Voice(format!("Failed to write sample: {}", e)))?;
    }

    writer
        .finalize()
        .map_err(|e| crate::AppError::Voice(format!("Failed to finalize WAV: {}", e)))?;

    tracing::info!("Saved audio to {:?}", path);
    Ok(())
}

/// Check if microphone is available
pub fn is_microphone_available() -> bool {
    let host = cpal::default_host();
    host.default_input_device().is_some()
}

/// Audio device information
#[derive(Debug, Clone, serde::Serialize)]
pub struct AudioDeviceInfo {
    pub name: String,
    pub is_default: bool,
}

/// List all available audio input devices
pub fn list_audio_input_devices() -> Vec<AudioDeviceInfo> {
    let host = cpal::default_host();
    let default_device_name = host.default_input_device().and_then(|d| d.name().ok());

    let mut devices = Vec::new();

    if let Ok(input_devices) = host.input_devices() {
        for device in input_devices {
            if let Ok(name) = device.name() {
                let is_default = default_device_name
                    .as_ref()
                    .map(|d| d == &name)
                    .unwrap_or(false);
                devices.push(AudioDeviceInfo { name, is_default });
            }
        }
    }

    devices
}
