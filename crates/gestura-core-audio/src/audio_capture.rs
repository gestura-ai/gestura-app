//! Audio capture module for microphone input
//! Uses cpal for cross-platform audio recording with voice activity detection (VAD)

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use gestura_core_foundation::AppError;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Silence detection configuration
const SILENCE_THRESHOLD: f32 = 0.005; // RMS threshold for detecting silence (lowered for sensitivity)
const SILENCE_TIMEOUT_SECS: f32 = 4.0; // Stop recording after 4 seconds of silence
const MAX_RECORDING_SECS: u64 = 120; // Maximum recording duration (2 minutes)
const VAD_WINDOW_MS: u64 = 100; // Window size for VAD analysis
const WAIT_FOR_SPEECH_TIMEOUT_SECS: u64 = 30; // Timeout if no speech detected after 30 seconds
const WHISPER_SAMPLE_RATE: u32 = 16000; // Whisper requires 16kHz audio

// Global flag to signal external stop request (e.g., from "Stop Listening" button)
lazy_static::lazy_static! {
    static ref EXTERNAL_STOP_FLAG: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
}

/// Request the audio recording to stop from external code
pub fn request_stop_recording() {
    tracing::info!("External stop requested for audio recording");
    EXTERNAL_STOP_FLAG.store(true, Ordering::SeqCst);
}

/// Reset the external stop flag (call before starting a new recording)
pub fn reset_stop_flag() {
    EXTERNAL_STOP_FLAG.store(false, Ordering::SeqCst);
}

/// Check if external stop was requested
pub fn is_stop_requested() -> bool {
    EXTERNAL_STOP_FLAG.load(Ordering::SeqCst)
}

/// Audio capture configuration
#[derive(Debug, Clone)]
pub struct AudioCaptureConfig {
    /// Optional device name to use (None = default device)
    pub device_name: Option<String>,
    /// Silence threshold for VAD (RMS value)
    pub silence_threshold: f32,
    /// Seconds of silence before stopping
    pub silence_timeout_secs: f32,
    /// Maximum recording duration in seconds
    pub max_recording_secs: u64,
    /// Timeout for waiting for speech to start
    pub wait_for_speech_timeout_secs: u64,
}

impl Default for AudioCaptureConfig {
    fn default() -> Self {
        Self {
            device_name: None,
            silence_threshold: SILENCE_THRESHOLD,
            silence_timeout_secs: SILENCE_TIMEOUT_SECS,
            max_recording_secs: MAX_RECORDING_SECS,
            wait_for_speech_timeout_secs: WAIT_FOR_SPEECH_TIMEOUT_SECS,
        }
    }
}

/// Record audio from the microphone until user stops speaking (4 seconds of silence)
/// Returns the duration of recorded audio in seconds
///
/// This function runs the entire recording process in a blocking task
/// to avoid Send/Sync issues with cpal::Stream
pub async fn record_audio(
    _duration: Duration,
    output_path: &Path,
    config: AudioCaptureConfig,
) -> Result<f32, AppError> {
    let output_path = output_path.to_path_buf();

    // Reset the external stop flag before starting a new recording
    reset_stop_flag();

    // Run the entire recording process in a blocking task
    // because cpal::Stream is not Send
    tokio::task::spawn_blocking(move || record_audio_with_vad(&output_path, &config))
        .await
        .map_err(|e| AppError::Voice(format!("Recording task failed: {}", e)))?
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
    last_rms_log_time: Instant,
    peak_rms: f32,
}

impl VadState {
    fn new() -> Self {
        let now = Instant::now();
        Self {
            last_speech_time: now,
            has_detected_speech: false,
            recording_start: now,
            has_logged_max_duration: false,
            last_rms_log_time: now,
            peak_rms: 0.0,
        }
    }
}

/// Audio device information
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AudioDeviceInfo {
    pub name: String,
    pub is_default: bool,
}

/// Check if microphone is available
pub fn is_microphone_available() -> bool {
    let host = cpal::default_host();
    host.default_input_device().is_some()
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

/// Record audio with Voice Activity Detection - stops after silence timeout
fn record_audio_with_vad(output_path: &Path, config: &AudioCaptureConfig) -> Result<f32, AppError> {
    // Get default audio host
    let host = cpal::default_host();

    // Try to find the specified device, or fall back to default
    let device = if let Some(ref name) = config.device_name {
        let found = host
            .input_devices()
            .ok()
            .and_then(|mut devices| devices.find(|d| d.name().ok().as_deref() == Some(name)));

        if let Some(dev) = found {
            tracing::info!("Using configured audio input device: {}", name);
            dev
        } else {
            tracing::warn!("Configured device '{}' not found, using default", name);
            host.default_input_device()
                .ok_or_else(|| AppError::Voice("No input device available".into()))?
        }
    } else {
        host.default_input_device()
            .ok_or_else(|| AppError::Voice("No input device available".into()))?
    };

    tracing::info!("Using audio input device: {:?}", device.name());

    // Get supported config
    let device_config = device
        .default_input_config()
        .map_err(|e| AppError::Voice(format!("Failed to get config: {}", e)))?;

    let sample_rate = device_config.sample_rate().0;
    let channels = device_config.channels();

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

    // Capture config values for closure
    let silence_threshold = config.silence_threshold;
    let silence_timeout = config.silence_timeout_secs;
    let max_recording = config.max_recording_secs;
    let wait_for_speech = config.wait_for_speech_timeout_secs;

    // Build input stream based on sample format
    let stream = match device_config.sample_format() {
        cpal::SampleFormat::F32 => device
            .build_input_stream(
                &device_config.clone().into(),
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    process_audio_data(
                        data,
                        &samples_clone,
                        &vad_buffer_clone,
                        &vad_state_clone,
                        &should_stop_clone,
                        samples_per_window,
                        silence_threshold,
                        silence_timeout,
                        max_recording,
                        wait_for_speech,
                    );
                },
                |err| {
                    tracing::error!("Audio stream error: {}", err);
                },
                None,
            )
            .map_err(|e| AppError::Voice(format!("Failed to build stream: {}", e)))?,
        cpal::SampleFormat::I16 => {
            let samples_clone_i16 = Arc::clone(&samples);
            let vad_buffer_i16 = Arc::clone(&vad_buffer);
            let vad_state_i16 = Arc::clone(&vad_state);
            let should_stop_i16 = Arc::clone(&should_stop);

            device
                .build_input_stream(
                    &device_config.clone().into(),
                    move |data: &[i16], _: &cpal::InputCallbackInfo| {
                        let f32_data: Vec<f32> =
                            data.iter().map(|&s| s as f32 / i16::MAX as f32).collect();
                        process_audio_data(
                            &f32_data,
                            &samples_clone_i16,
                            &vad_buffer_i16,
                            &vad_state_i16,
                            &should_stop_i16,
                            samples_per_window,
                            silence_threshold,
                            silence_timeout,
                            max_recording,
                            wait_for_speech,
                        );
                    },
                    |err| {
                        tracing::error!("Audio stream error: {}", err);
                    },
                    None,
                )
                .map_err(|e| AppError::Voice(format!("Failed to build stream: {}", e)))?
        }
        _ => return Err(AppError::Voice("Unsupported sample format".into())),
    };

    // Start recording
    stream
        .play()
        .map_err(|e| AppError::Voice(format!("Failed to start stream: {}", e)))?;

    tracing::info!(
        "Recording with VAD - will stop after {}s of silence...",
        config.silence_timeout_secs
    );

    // Wait for speech to end (with silence timeout), max duration, or external stop request
    loop {
        std::thread::sleep(Duration::from_millis(100));

        // Check internal VAD stop flag
        if should_stop.load(Ordering::SeqCst) {
            tracing::info!("Recording stopped by VAD (silence/max duration)");
            break;
        }

        // Check external stop request (e.g., "Stop Listening" button)
        if is_stop_requested() {
            tracing::info!("Recording stopped by external request");
            should_stop.store(true, Ordering::SeqCst);
            break;
        }

        // Also check for max duration from main thread
        let state = vad_state.lock().unwrap();
        if state.recording_start.elapsed().as_secs() >= config.max_recording_secs {
            break;
        }
    }

    // Stop recording - explicitly pause and drop the stream to release the microphone
    let _ = stream.pause();
    drop(stream);
    tracing::info!("Audio stream stopped and microphone released");

    // Get recorded samples
    let recorded_samples = samples.lock().unwrap();
    let sample_count = recorded_samples.len();
    let duration_secs = sample_count as f32 / (sample_rate as f32 * channels as f32);

    tracing::info!("Recorded {} samples ({:.2}s)", sample_count, duration_secs);

    // If externally stopped with no audio, return early without error
    if sample_count == 0 {
        if is_stop_requested() {
            return Err(AppError::Voice("Recording cancelled by user".into()));
        }
        return Err(AppError::Voice("No audio captured".into()));
    }

    // Resample audio to 16kHz mono for Whisper compatibility
    let resampled = resample_to_16khz(&recorded_samples, sample_rate, channels);

    // Save to WAV file at 16kHz mono (what Whisper expects)
    save_samples_to_wav(&resampled, WHISPER_SAMPLE_RATE, 1, output_path)?;

    Ok(duration_secs)
}

/// Process audio data for VAD analysis
#[allow(clippy::too_many_arguments)]
fn process_audio_data(
    data: &[f32],
    samples: &Arc<Mutex<Vec<f32>>>,
    vad_buffer: &Arc<Mutex<Vec<f32>>>,
    vad_state: &Arc<Mutex<VadState>>,
    should_stop: &Arc<AtomicBool>,
    samples_per_window: usize,
    silence_threshold: f32,
    silence_timeout: f32,
    max_recording: u64,
    wait_for_speech: u64,
) {
    // Early exit if we should stop - don't process any more audio
    if should_stop.load(Ordering::SeqCst) {
        return;
    }

    // Store all samples
    {
        let mut buffer = samples.lock().unwrap();
        buffer.extend_from_slice(data);
    }

    // Add to VAD buffer for analysis
    {
        let mut vad_buf = vad_buffer.lock().unwrap();
        vad_buf.extend_from_slice(data);

        // Analyze when we have enough samples
        while vad_buf.len() >= samples_per_window {
            let window: Vec<f32> = vad_buf.drain(..samples_per_window).collect();
            let rms = calculate_rms(&window);

            let mut state = vad_state.lock().unwrap();
            let now = Instant::now();

            // Track peak RMS for debugging
            if rms > state.peak_rms {
                state.peak_rms = rms;
            }

            // Log RMS periodically (every 2 seconds) for debugging
            if now.duration_since(state.last_rms_log_time).as_secs() >= 2 {
                tracing::info!(
                    "VAD status: current_rms={:.4}, peak_rms={:.4}, threshold={:.4}, speech_detected={}",
                    rms,
                    state.peak_rms,
                    silence_threshold,
                    state.has_detected_speech
                );
                state.last_rms_log_time = now;
            }

            if rms > silence_threshold {
                // Speech detected
                state.last_speech_time = now;
                if !state.has_detected_speech {
                    state.has_detected_speech = true;
                    tracing::info!(
                        "🎤 Speech detected! (RMS: {:.4} > threshold: {:.4})",
                        rms,
                        silence_threshold
                    );
                }
            } else if state.has_detected_speech {
                // Check if silence timeout reached (only stop once)
                let silence_duration = now.duration_since(state.last_speech_time);
                if silence_duration.as_secs_f32() >= silence_timeout
                    && !should_stop.load(Ordering::SeqCst)
                {
                    tracing::info!(
                        "🔇 Silence timeout reached ({:.1}s) - stopping recording",
                        silence_duration.as_secs_f32()
                    );
                    should_stop.store(true, Ordering::SeqCst);
                }
            } else {
                // No speech detected yet - check for "waiting for speech" timeout
                let waiting_duration = now.duration_since(state.recording_start).as_secs();
                if waiting_duration >= wait_for_speech {
                    tracing::warn!(
                        "⏱️ No speech detected after {}s (peak_rms={:.4}, threshold={:.4}) - stopping",
                        waiting_duration,
                        state.peak_rms,
                        silence_threshold
                    );
                    should_stop.store(true, Ordering::SeqCst);
                }
            }

            // Check max recording duration (only log once)
            if now.duration_since(state.recording_start).as_secs() >= max_recording {
                if !state.has_logged_max_duration {
                    tracing::info!("Max recording duration reached");
                    state.has_logged_max_duration = true;
                }
                should_stop.store(true, Ordering::SeqCst);
            }
        }
    }
}

/// Resample audio from source sample rate to 16kHz mono for Whisper
/// Uses simple linear interpolation for resampling
fn resample_to_16khz(samples: &[f32], source_rate: u32, channels: u16) -> Vec<f32> {
    if samples.is_empty() {
        return Vec::new();
    }

    // First, convert to mono if stereo
    let mono_samples: Vec<f32> = if channels > 1 {
        samples
            .chunks(channels as usize)
            .map(|chunk| chunk.iter().sum::<f32>() / channels as f32)
            .collect()
    } else {
        samples.to_vec()
    };

    // If already at 16kHz, return mono samples
    if source_rate == WHISPER_SAMPLE_RATE {
        tracing::info!("Audio already at 16kHz, no resampling needed");
        return mono_samples;
    }

    // Calculate resampling ratio
    let ratio = source_rate as f64 / WHISPER_SAMPLE_RATE as f64;
    let output_len = (mono_samples.len() as f64 / ratio).ceil() as usize;
    let mut resampled = Vec::with_capacity(output_len);

    tracing::info!(
        "Resampling audio from {}Hz to {}Hz ({} -> {} samples)",
        source_rate,
        WHISPER_SAMPLE_RATE,
        mono_samples.len(),
        output_len
    );

    // Linear interpolation resampling
    for i in 0..output_len {
        let src_pos = i as f64 * ratio;
        let src_idx = src_pos.floor() as usize;
        let frac = (src_pos - src_idx as f64) as f32;

        if src_idx + 1 < mono_samples.len() {
            // Interpolate between two samples
            let sample = mono_samples[src_idx] * (1.0 - frac) + mono_samples[src_idx + 1] * frac;
            resampled.push(sample);
        } else if src_idx < mono_samples.len() {
            resampled.push(mono_samples[src_idx]);
        }
    }

    resampled
}

/// Save audio samples to a WAV file
fn save_samples_to_wav(
    samples: &[f32],
    sample_rate: u32,
    channels: u16,
    path: &Path,
) -> Result<(), AppError> {
    let spec = hound::WavSpec {
        channels,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };

    let mut writer = hound::WavWriter::create(path, spec)
        .map_err(|e| AppError::Voice(format!("Failed to create WAV: {}", e)))?;

    // Convert f32 samples to i16
    for sample in samples {
        let sample_i16 = (*sample * i16::MAX as f32) as i16;
        writer
            .write_sample(sample_i16)
            .map_err(|e| AppError::Voice(format!("Failed to write sample: {}", e)))?;
    }

    writer
        .finalize()
        .map_err(|e| AppError::Voice(format!("Failed to finalize WAV: {}", e)))?;

    tracing::info!(
        "Saved audio to {:?} ({}Hz, {} channels)",
        path,
        sample_rate,
        channels
    );
    Ok(())
}
