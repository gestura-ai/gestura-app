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
    /// Uses the configured STT provider (local Whisper or OpenAI Whisper API)
    pub async fn transcribe_audio(
        &self,
        audio_path: &Path,
    ) -> Result<TranscriptionResult, AppError> {
        let config = self.get_config();
        let app_config = AppConfig::load();

        tracing::info!("Transcribing audio with provider: {}", config.stt_provider);

        match config.stt_provider.as_str() {
            "openai-whisper" => {
                // Use OpenAI Whisper API
                let api_key = if !config.openai_api_key.is_empty() {
                    config.openai_api_key.clone()
                } else if let Some(ref openai) = app_config.llm.openai {
                    openai.api_key.clone()
                } else {
                    return Err(AppError::Voice("OpenAI API key not configured".to_string()));
                };

                let client = reqwest::Client::new();
                let bytes = std::fs::read(audio_path)
                    .map_err(|e| AppError::Voice(format!("Failed to read audio file: {}", e)))?;

                let file_name = audio_path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("audio.wav")
                    .to_string();

                let part = reqwest::multipart::Part::bytes(bytes)
                    .file_name(file_name)
                    .mime_str("audio/wav")
                    .map_err(|e| AppError::Voice(format!("Failed to create multipart: {}", e)))?;

                let form = reqwest::multipart::Form::new()
                    .text("model", "whisper-1")
                    .part("file", part);

                let response = client
                    .post("https://api.openai.com/v1/audio/transcriptions")
                    .header("Authorization", format!("Bearer {}", api_key))
                    .multipart(form)
                    .send()
                    .await
                    .map_err(|e| AppError::Voice(format!("OpenAI API request failed: {}", e)))?;

                if !response.status().is_success() {
                    let error_text = response.text().await.unwrap_or_default();
                    return Err(AppError::Voice(format!("OpenAI API error: {}", error_text)));
                }

                #[derive(serde::Deserialize)]
                struct WhisperResponse {
                    text: String,
                }

                let result: WhisperResponse = response
                    .json()
                    .await
                    .map_err(|e| AppError::Voice(format!("Failed to parse response: {}", e)))?;

                Ok(TranscriptionResult {
                    text: result.text,
                    duration_secs: 0.0, // Duration not provided by API
                    audio_path: Some(audio_path.to_path_buf()),
                    provider: "openai-whisper".to_string(),
                })
            }
            "local-whisper" => {
                // Local Whisper transcription
                // For now, return a placeholder - full whisper-rs integration would go here
                tracing::warn!(
                    "Local Whisper transcription not yet implemented, returning placeholder"
                );
                Err(AppError::Voice(
	                    "Local Whisper transcription not yet implemented. Use --voice with OpenAI provider configured."
	                        .to_string(),
	                ))
            }
            other => {
                tracing::warn!(
                    "Unknown STT provider '{other}', falling back to local whisper placeholder"
                );
                Err(AppError::Voice(format!(
                    "Unknown STT provider '{other}'. Supported providers: openai-whisper, local-whisper"
                )))
            }
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
