//! Speech processing module for Gestura
//!
//! This module provides core speech-to-text and LLM processing functionality.
//! It is designed to be used by both GUI and CLI applications.
//!
//! The Tauri-specific event handling (window management, event emission) should
//! be implemented in the gestura-gui crate.

use crate::audio_capture::{AudioCaptureConfig, record_audio};
use crate::config::AppConfig;
use crate::error::AppError;
use crate::llm_provider::{AgentContext, select_provider};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Speech processing configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeechConfig {
    /// STT provider: "local-whisper" or "openai-whisper"
    pub stt_provider: String,
    /// LLM provider for processing
    pub llm_provider: String,
    /// OpenAI API key for Whisper API
    pub openai_api_key: String,
    /// Anthropic API key
    pub anthropic_api_key: String,
    /// Google API key
    pub google_api_key: String,
    /// Azure API key
    pub azure_api_key: String,
    /// Local LLM endpoint (e.g., Ollama)
    pub local_llm_endpoint: String,
    /// STT timeout in seconds
    pub stt_timeout: u64,
    /// LLM timeout in seconds
    pub llm_timeout: u64,
    /// Enable fallback to alternative providers
    pub enable_fallback: bool,
    /// Cache LLM responses
    pub cache_responses: bool,
}

impl Default for SpeechConfig {
    fn default() -> Self {
        Self {
            stt_provider: "local-whisper".to_string(),
            llm_provider: "openai".to_string(),
            openai_api_key: String::new(),
            anthropic_api_key: String::new(),
            google_api_key: String::new(),
            azure_api_key: String::new(),
            local_llm_endpoint: "http://localhost:11434".to_string(),
            stt_timeout: 30,
            llm_timeout: 60,
            enable_fallback: true,
            cache_responses: true,
        }
    }
}

impl SpeechConfig {
    /// Create SpeechConfig from AppConfig
    pub fn from_app_config(app_config: &AppConfig) -> Self {
        Self {
            stt_provider: match app_config.voice.provider.as_str() {
                "local" => "local-whisper".to_string(),
                "openai" => "openai-whisper".to_string(),
                _ => "local-whisper".to_string(),
            },
            llm_provider: app_config.llm.primary.clone(),
            openai_api_key: app_config.voice.openai_api_key.clone().unwrap_or_default(),
            anthropic_api_key: app_config
                .llm
                .anthropic
                .as_ref()
                .map(|a| a.api_key.clone())
                .unwrap_or_default(),
            google_api_key: String::new(),
            azure_api_key: String::new(),
            local_llm_endpoint: app_config
                .llm
                .ollama
                .as_ref()
                .map(|o| o.base_url.clone())
                .unwrap_or_else(|| "http://localhost:11434".to_string()),
            stt_timeout: 30,
            llm_timeout: 60,
            enable_fallback: true,
            cache_responses: true,
        }
    }
}

/// Result of speech transcription
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptionResult {
    /// The transcribed text
    pub text: String,
    /// Duration of the audio in seconds
    pub duration_secs: f32,
    /// Path to the temporary audio file (if retained)
    pub audio_path: Option<PathBuf>,
    /// Provider used for transcription
    pub provider: String,
}

/// Result of LLM processing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmResponse {
    /// The AI response text
    pub text: String,
    /// Provider used for processing
    pub provider: String,
    /// Whether this was a cached response
    pub cached: bool,
}

/// Core speech processor without Tauri dependencies
///
/// This processor handles:
/// - Audio recording with VAD
/// - Speech-to-text transcription
/// - LLM processing
///
/// Event emission and window management should be handled by the caller.
#[derive(Debug, Clone)]
pub struct SpeechProcessor {
    config: Arc<Mutex<SpeechConfig>>,
    is_recording: Arc<Mutex<bool>>,
}

impl Default for SpeechProcessor {
    fn default() -> Self {
        Self::new()
    }
}

impl SpeechProcessor {
    /// Create a new speech processor with default configuration
    pub fn new() -> Self {
        Self {
            config: Arc::new(Mutex::new(SpeechConfig::default())),
            is_recording: Arc::new(Mutex::new(false)),
        }
    }

    /// Create a new speech processor with custom configuration
    pub fn with_config(config: SpeechConfig) -> Self {
        Self {
            config: Arc::new(Mutex::new(config)),
            is_recording: Arc::new(Mutex::new(false)),
        }
    }

    /// Update the speech processor configuration
    pub fn update_config(&self, config: SpeechConfig) {
        let mut current_config = self.config.lock().unwrap();
        *current_config = config;
        tracing::info!("Speech processor configuration updated");
    }

    /// Get the current configuration
    pub fn get_config(&self) -> SpeechConfig {
        self.config.lock().unwrap().clone()
    }

    /// Check if currently recording
    pub fn is_recording(&self) -> bool {
        *self.is_recording.lock().unwrap()
    }

    /// Set recording state
    pub fn set_recording(&self, recording: bool) {
        let mut is_recording = self.is_recording.lock().unwrap();
        *is_recording = recording;
    }

    /// Stop the current recording
    pub fn stop_recording(&self) -> Result<(), AppError> {
        let mut recording = self.is_recording.lock().unwrap();
        if !*recording {
            return Err(AppError::Voice("Not currently recording".to_string()));
        }
        *recording = false;

        // Signal the audio capture to stop immediately
        crate::audio_capture::request_stop_recording();

        tracing::info!("Stopped speech capture and requested audio recording stop");
        Ok(())
    }

    /// Record audio from microphone and return the path to the audio file
    ///
    /// Returns the duration and path to the recorded audio file.
    /// The caller is responsible for cleaning up the temp file.
    pub async fn record_audio_to_file(
        &self,
        device_name: Option<String>,
    ) -> Result<(f32, PathBuf), AppError> {
        // Check if already recording
        {
            let mut recording = self.is_recording.lock().unwrap();
            if *recording {
                return Err(AppError::Voice("Already recording".to_string()));
            }
            *recording = true;
        }

        let temp_dir = std::env::temp_dir();
        let audio_path = temp_dir.join(format!(
            "gestura_audio_{}.wav",
            chrono::Utc::now().timestamp()
        ));

        let config = AudioCaptureConfig {
            device_name,
            ..Default::default()
        };

        let result = record_audio(Duration::from_secs(0), &audio_path, config).await;

        // Reset recording state
        {
            let mut recording = self.is_recording.lock().unwrap();
            *recording = false;
        }

        match result {
            Ok(duration) => {
                tracing::info!("Recorded {:.2}s of audio to {:?}", duration, audio_path);
                if duration < 0.5 {
                    let _ = std::fs::remove_file(&audio_path);
                    return Err(AppError::Voice(
                        "Recording too short - no audio captured".to_string(),
                    ));
                }
                Ok((duration, audio_path))
            }
            Err(e) => {
                let _ = std::fs::remove_file(&audio_path);
                Err(e)
            }
        }
    }

    /// Transcribe audio file to text
    ///
    /// Uses the unified STT provider abstraction from `stt_provider` module.
    /// The provider is selected based on `AppConfig.voice.provider` and respects
    /// configured base_url and model settings.
    pub async fn transcribe_audio(
        &self,
        audio_path: &Path,
    ) -> Result<TranscriptionResult, AppError> {
        let app_config = AppConfig::load();
        let provider = crate::stt_provider::select_provider(&app_config);

        tracing::info!(
            "Transcribing audio with provider: {} (config.voice.provider={})",
            provider.provider_id(),
            app_config.voice.provider
        );

        provider.transcribe_file(audio_path).await
    }

    /// Transcribe audio using local Whisper model (whisper-rs)
    #[cfg(feature = "voice-local")]
    #[allow(dead_code)]
    async fn transcribe_with_local_whisper(
        &self,
        audio_path: &Path,
    ) -> Result<TranscriptionResult, AppError> {
        use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

        // Get model path from config or use default
        let model_path = self.get_whisper_model_path()?;

        tracing::info!("Loading Whisper model from: {:?}", model_path);

        // Load the Whisper context (model)
        let ctx = WhisperContext::new_with_params(
            model_path
                .to_str()
                .ok_or_else(|| AppError::Voice("Invalid model path encoding".to_string()))?,
            WhisperContextParameters::default(),
        )
        .map_err(|e| AppError::Voice(format!("Failed to load Whisper model: {}", e)))?;

        // Read and convert audio to f32 samples
        let samples = self.load_audio_samples(audio_path)?;
        let duration_secs = samples.len() as f32 / 16000.0; // Whisper expects 16kHz

        // Create transcription parameters
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(Some("en"));
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_translate(false);
        params.set_no_context(true);
        params.set_single_segment(false);

        // Create state and run transcription
        let mut state = ctx
            .create_state()
            .map_err(|e| AppError::Voice(format!("Failed to create Whisper state: {}", e)))?;

        state
            .full(params, &samples)
            .map_err(|e| AppError::Voice(format!("Whisper transcription failed: {}", e)))?;

        // Collect all segments
        let num_segments = state
            .full_n_segments()
            .map_err(|e| AppError::Voice(format!("Failed to get segment count: {}", e)))?;

        let mut text = String::new();
        for i in 0..num_segments {
            if let Ok(segment_text) = state.full_get_segment_text(i) {
                text.push_str(&segment_text);
                text.push(' ');
            }
        }

        let text = text.trim().to_string();
        tracing::info!("Local Whisper transcription complete: {} chars", text.len());

        Ok(TranscriptionResult {
            text,
            duration_secs,
            audio_path: Some(audio_path.to_path_buf()),
            provider: "local-whisper".to_string(),
        })
    }

    /// Fallback when voice-local feature is not enabled
    #[cfg(not(feature = "voice-local"))]
    #[allow(dead_code)]
    async fn transcribe_with_local_whisper(
        &self,
        _audio_path: &Path,
    ) -> Result<TranscriptionResult, AppError> {
        Err(AppError::Voice(
            "Local Whisper transcription requires the 'whisper' feature. \
             Build with `cargo build --features whisper` or use OpenAI Whisper API instead."
                .to_string(),
        ))
    }

    /// Get the path to the Whisper model file
    #[allow(dead_code)]
    fn get_whisper_model_path(&self) -> Result<PathBuf, AppError> {
        // Check environment variable first
        if let Ok(path) = std::env::var("GESTURA_WHISPER_MODEL") {
            let path = PathBuf::from(path);
            if path.exists() {
                return Ok(path);
            }
        }

        // Check standard locations
        let model_names = ["ggml-base.en.bin", "ggml-small.en.bin", "ggml-tiny.en.bin"];
        let search_dirs = [
            // User data directory
            dirs::data_dir().map(|d| d.join("gestura").join("models")),
            // Home directory
            dirs::home_dir().map(|d| d.join(".gestura").join("models")),
            // Current directory
            Some(PathBuf::from("models")),
        ];

        for dir in search_dirs.iter().flatten() {
            for model_name in &model_names {
                let model_path = dir.join(model_name);
                if model_path.exists() {
                    tracing::info!("Found Whisper model at: {:?}", model_path);
                    return Ok(model_path);
                }
            }
        }

        Err(AppError::Voice(
            "Whisper model not found. Please download a model (e.g., ggml-base.en.bin) \
             and place it in ~/.gestura/models/ or set GESTURA_WHISPER_MODEL environment variable."
                .to_string(),
        ))
    }

    /// Load audio file and convert to f32 samples at 16kHz
    #[cfg(feature = "voice-local")]
    #[allow(dead_code)]
    fn load_audio_samples(&self, audio_path: &Path) -> Result<Vec<f32>, AppError> {
        use hound::WavReader;

        let mut reader = WavReader::open(audio_path)
            .map_err(|e| AppError::Voice(format!("Failed to open audio file: {}", e)))?;

        let spec = reader.spec();
        let sample_rate = spec.sample_rate;
        let channels = spec.channels as usize;

        // Read samples based on format, propagating decode errors
        let samples: Vec<f32> = match spec.sample_format {
            hound::SampleFormat::Int => {
                let max_val = (1 << (spec.bits_per_sample - 1)) as f32;
                let raw_samples: Result<Vec<i32>, _> = reader.samples::<i32>().collect();
                raw_samples
                    .map_err(|e| AppError::Voice(format!("Failed to decode audio samples: {}", e)))?
                    .into_iter()
                    .map(|s| s as f32 / max_val)
                    .collect()
            }
            hound::SampleFormat::Float => {
                let raw_samples: Result<Vec<f32>, _> = reader.samples::<f32>().collect();
                raw_samples.map_err(|e| {
                    AppError::Voice(format!("Failed to decode audio samples: {}", e))
                })?
            }
        };

        // Convert to mono if stereo
        let mono_samples: Vec<f32> = if channels > 1 {
            samples
                .chunks(channels)
                .map(|chunk| chunk.iter().sum::<f32>() / channels as f32)
                .collect()
        } else {
            samples
        };

        // Resample to 16kHz if needed (simple linear interpolation)
        let target_rate = 16000;
        if sample_rate != target_rate {
            let ratio = sample_rate as f32 / target_rate as f32;
            let new_len = (mono_samples.len() as f32 / ratio) as usize;
            let mut resampled = Vec::with_capacity(new_len);

            for i in 0..new_len {
                let src_idx = i as f32 * ratio;
                let idx = src_idx as usize;
                let frac = src_idx - idx as f32;

                let sample = if idx + 1 < mono_samples.len() {
                    mono_samples[idx] * (1.0 - frac) + mono_samples[idx + 1] * frac
                } else {
                    mono_samples[idx.min(mono_samples.len() - 1)]
                };
                resampled.push(sample);
            }

            Ok(resampled)
        } else {
            Ok(mono_samples)
        }
    }

    /// Process transcribed text with configured LLM provider
    pub async fn process_with_llm(&self, text: &str) -> Result<LlmResponse, AppError> {
        let app_config = AppConfig::load();
        let provider = select_provider(
            &app_config,
            &AgentContext {
                agent_id: "speech".into(),
            },
        );

        tracing::info!("Processing with LLM provider: {}", app_config.llm.primary);

        let response = provider
            .call(text)
            .await
            .map_err(|e| AppError::Llm(format!("LLM processing failed: {}", e)))?;

        Ok(LlmResponse {
            text: response,
            provider: app_config.llm.primary.clone(),
            cached: false,
        })
    }

    /// Determine if text is a conversation or command
    pub fn is_conversation(&self, text: &str) -> bool {
        let conversation_keywords = [
            "help", "what", "how", "can you", "please", "tell me", "explain",
        ];
        let command_keywords = [
            "open", "close", "start", "stop", "launch", "quit", "show", "hide",
        ];

        let text_lower = text.to_lowercase();
        let conversation_score = conversation_keywords
            .iter()
            .filter(|&keyword| text_lower.contains(keyword))
            .count();
        let command_score = command_keywords
            .iter()
            .filter(|&keyword| text_lower.contains(keyword))
            .count();

        conversation_score > command_score
    }
}

// Global speech processor instance
lazy_static::lazy_static! {
    static ref SPEECH_PROCESSOR: SpeechProcessor = SpeechProcessor::new();
}

/// Get the global speech processor instance
pub fn get_speech_processor() -> &'static SpeechProcessor {
    &SPEECH_PROCESSOR
}

/// Check if speech is currently being recorded
pub fn is_speech_recording() -> bool {
    SPEECH_PROCESSOR.is_recording()
}

/// Update the global speech processor configuration
pub fn update_speech_config(config: SpeechConfig) {
    SPEECH_PROCESSOR.update_config(config);
}

/// Resolve the path to the local Whisper model file.
///
/// Checks (in order):
/// 1. `config.voice.local_model_path` if set
/// 2. `GESTURA_WHISPER_MODEL` environment variable
/// 3. Default search directories (~/.gestura/models/, ./models/)
///
/// Returns an error with an actionable message if no model is found.
#[cfg(feature = "voice-local")]
pub fn resolve_whisper_model_path(config: &AppConfig) -> Result<PathBuf, AppError> {
    // 1. Check config-provided path first
    if let Some(ref path_str) = config.voice.local_model_path {
        let path = PathBuf::from(path_str);
        if path.exists() {
            return Ok(path);
        }
        // Config path set but doesn't exist — warn and continue searching
        tracing::warn!(
            "Configured local_model_path '{}' does not exist; searching defaults",
            path_str
        );
    }

    // 2. Check environment variable
    if let Ok(path_str) = std::env::var("GESTURA_WHISPER_MODEL") {
        let path = PathBuf::from(&path_str);
        if path.exists() {
            return Ok(path);
        }
    }

    // 3. Search default directories
    let model_names = [
        "ggml-base.en.bin",
        "ggml-base.bin",
        "ggml-small.en.bin",
        "ggml-small.bin",
        "ggml-medium.en.bin",
        "ggml-medium.bin",
        "ggml-large.bin",
    ];

    let search_dirs: Vec<Option<PathBuf>> = vec![
        dirs::home_dir().map(|h| h.join(".gestura").join("models")),
        Some(PathBuf::from("models")),
    ];

    for dir in search_dirs.iter().flatten() {
        for model_name in &model_names {
            let model_path = dir.join(model_name);
            if model_path.exists() {
                tracing::info!("Found Whisper model at: {:?}", model_path);
                return Ok(model_path);
            }
        }
    }

    Err(AppError::Voice(
        "Whisper model not found. Please download a model (e.g., ggml-base.en.bin) \
         and place it in ~/.gestura/models/ or set GESTURA_WHISPER_MODEL environment variable, \
         or configure voice.local_model_path in your settings."
            .to_string(),
    ))
}

/// Load audio file and convert to 16kHz mono f32 samples.
///
/// This is the format required by whisper.cpp / whisper-rs for transcription.
/// Supports WAV files with integer or float samples, any sample rate, mono or stereo.
#[cfg(feature = "voice-local")]
pub fn load_audio_samples_16khz_mono(audio_path: &Path) -> Result<Vec<f32>, AppError> {
    use hound::WavReader;

    let mut reader = WavReader::open(audio_path)
        .map_err(|e| AppError::Voice(format!("Failed to open audio file: {}", e)))?;

    let spec = reader.spec();
    let sample_rate = spec.sample_rate;
    let channels = spec.channels as usize;

    // Read samples based on format, propagating decode errors
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => {
            let max_val = (1 << (spec.bits_per_sample - 1)) as f32;
            let raw_samples: Result<Vec<i32>, _> = reader.samples::<i32>().collect();
            raw_samples
                .map_err(|e| AppError::Voice(format!("Failed to decode audio samples: {}", e)))?
                .into_iter()
                .map(|s| s as f32 / max_val)
                .collect()
        }
        hound::SampleFormat::Float => {
            let raw_samples: Result<Vec<f32>, _> = reader.samples::<f32>().collect();
            raw_samples
                .map_err(|e| AppError::Voice(format!("Failed to decode audio samples: {}", e)))?
        }
    };

    // Convert to mono if stereo (average channels)
    let mono_samples: Vec<f32> = if channels > 1 {
        samples
            .chunks(channels)
            .map(|chunk| chunk.iter().sum::<f32>() / channels as f32)
            .collect()
    } else {
        samples
    };

    // Resample to 16kHz if needed (simple linear interpolation)
    let target_rate = 16000u32;
    if sample_rate != target_rate {
        let ratio = sample_rate as f32 / target_rate as f32;
        let new_len = (mono_samples.len() as f32 / ratio) as usize;
        let mut resampled = Vec::with_capacity(new_len);

        for i in 0..new_len {
            let src_idx = i as f32 * ratio;
            let idx = src_idx as usize;
            let frac = src_idx - idx as f32;

            let sample = if idx + 1 < mono_samples.len() {
                mono_samples[idx] * (1.0 - frac) + mono_samples[idx + 1] * frac
            } else {
                mono_samples[idx.min(mono_samples.len() - 1)]
            };
            resampled.push(sample);
        }

        Ok(resampled)
    } else {
        Ok(mono_samples)
    }
}
