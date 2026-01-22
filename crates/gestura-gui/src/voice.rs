#![allow(unused)]
/// Voice engine feature scaffolding for faster-whisper (CTranslate2)
#[cfg(feature = "voice-faster-whisper")]
pub struct WhisperFasterLocal {
    pub model_path: String,
    pub device: String,
    pub compute_type: String,
}

#[cfg(feature = "voice-faster-whisper")]
impl WhisperFasterLocal {
    pub fn new(model_path: String) -> Self {
        Self {
            model_path,
            device: "cpu".to_string(),
            compute_type: "int8".to_string(),
        }
    }

    /// Load audio file and prepare for processing
    async fn load_audio(&self, path: &str) -> Result<Vec<f32>, AppError> {
        // For now, return mock audio data
        // In production, this would use faster-whisper's audio loading
        tracing::info!("Loading audio from: {}", path);
        let _ = std::fs::metadata(path).map_err(|e| AppError::Voice(e.to_string()))?;

        // Mock 1 second of audio at 16kHz
        Ok(vec![0.0; 16000])
    }

    /// Process audio with faster-whisper
    async fn transcribe_audio(&self, _audio: Vec<f32>) -> Result<String, AppError> {
        // Mock transcription - in production this would call faster-whisper
        tracing::info!(
            "Transcribing with faster-whisper model: {}",
            self.model_path
        );

        // Simulate processing time
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        Ok("Hello, this is a faster-whisper transcription.".to_string())
    }
}

#[cfg(feature = "voice-faster-whisper")]
#[async_trait::async_trait]
impl VoiceProcessor for WhisperFasterLocal {
    fn engine_name(&self) -> &'static str {
        "faster-whisper-local"
    }

    async fn process_command(
        &self,
        config: &AppConfig,
        nats: Option<&crate::NatsConn>,
    ) -> Result<String, AppError> {
        let input = if let Some(path) = &config.voice.input_path {
            path.clone()
        } else {
            return Err(AppError::Voice("no input_path configured".into()));
        };

        // Load and process audio
        let audio = self.load_audio(&input).await?;
        let text = self.transcribe_audio(audio).await?;

        // Publish to NATS if available
        #[cfg(feature = "nats")]
        if let Some(nc) = nats {
            let payload = serde_json::json!({
                "engine": "faster-whisper",
                "text": text,
                "model": self.model_path
            });
            let _ = nc
                .publish("events.voice", bytes::Bytes::from(payload.to_string()))
                .await;
        }

        Ok(text)
    }
}

// Voice processing trait and mock (Stage 2)
/// Real Faster-Whisper integration is deferred; we provide an interface for later.
use crate::{AppConfig, AppError};

/// Trait to process audio commands and return the recognized text.
#[async_trait::async_trait]
pub trait VoiceProcessor: Send + Sync {
    /// Returns a stable engine name for diagnostics and testing.
    fn engine_name(&self) -> &'static str {
        "unknown"
    }
    /// Process mic audio to text and optionally publish to NATS.
    async fn process_command(
        &self,
        _config: &AppConfig,
        nats: Option<&crate::NatsConn>,
    ) -> Result<String, AppError>;
}

/// Mock implementation that returns a fixed string and publishes to NATS if available.
pub struct MockVoice;

#[async_trait::async_trait]
impl VoiceProcessor for MockVoice {
    fn engine_name(&self) -> &'static str {
        "mock"
    }
    async fn process_command(
        &self,
        _config: &AppConfig,
        nats: Option<&crate::NatsConn>,
    ) -> Result<String, AppError> {
        let text = "mock voice command".to_string();
        if let Some(nc) = nats {
            #[cfg(feature = "nats")]
            {
                let payload = serde_json::json!({"text": text});
                nc.publish("events.voice", bytes::Bytes::from(payload.to_string()))
                    .await
                    .map_err(|e| AppError::Nats(e.to_string()))?;
            }
        }
        Ok(text)
    }
}

#[cfg(not(feature = "voice-local"))]
pub struct WhisperLocal {
    pub model_path: String,
}

#[cfg(not(feature = "voice-local"))]
impl WhisperLocal {
    /// Stub transcribe_file when voice-local feature is disabled
    pub fn transcribe_file(&self, _audio_path: &std::path::Path) -> Result<String, AppError> {
        Err(AppError::Voice(
            "Local Whisper is not available. Please enable the 'voice-local' feature and provide a whisper.cpp model file.".into(),
        ))
    }
}

#[cfg(not(feature = "voice-local"))]
#[async_trait::async_trait]
impl VoiceProcessor for WhisperLocal {
    fn engine_name(&self) -> &'static str {
        "whisper-local"
    }
    async fn process_command(
        &self,
        _config: &AppConfig,
        _nats: Option<&crate::NatsConn>,
    ) -> Result<String, AppError> {
        Err(AppError::Voice("voice-local feature disabled".into()))
    }
}

#[cfg(feature = "voice-local")]
/// Voice engine that uses local Whisper.cpp (whisper-rs) for offline STT.
pub struct WhisperLocal {
    pub model_path: String,
}

#[cfg(feature = "voice-local")]
impl WhisperLocal {
    /// Transcribe an audio file directly using local Whisper model
    pub fn transcribe_file(&self, audio_path: &std::path::Path) -> Result<String, AppError> {
        // Load model with default parameters
        let params = whisper_rs::WhisperContextParameters::default();
        let ctx = whisper_rs::WhisperContext::new_with_params(&self.model_path, params)
            .map_err(|e| AppError::Voice(format!("Failed to load whisper model: {e}")))?;
        let mut state = ctx
            .create_state()
            .map_err(|e| AppError::Voice(format!("Failed to create whisper state: {e}")))?;

        // Read audio samples from WAV, propagating decode errors
        let mut rdr =
            hound::WavReader::open(audio_path).map_err(|e| AppError::Voice(e.to_string()))?;
        let raw_samples: Result<Vec<i16>, _> = rdr.samples::<i16>().collect();
        let samples: Vec<f32> = raw_samples
            .map_err(|e| AppError::Voice(format!("Failed to decode audio samples: {}", e)))?
            .into_iter()
            .map(|s| s as f32 / i16::MAX as f32)
            .collect();

        // Run full transcribe
        state
            .full(
                whisper_rs::FullParams::new(whisper_rs::SamplingStrategy::Greedy { best_of: 1 }),
                &samples,
            )
            .map_err(|e| AppError::Voice(format!("Whisper transcription failed: {e}")))?;

        // Collect segments
        let num_segments = state
            .full_n_segments()
            .map_err(|e| AppError::Voice(format!("Failed to get segments: {e}")))?;
        let mut text = String::new();
        for i in 0..num_segments {
            let seg = state
                .full_get_segment_text(i)
                .map_err(|e| AppError::Voice(format!("Failed to get segment text: {e}")))?;
            text.push_str(seg.trim());
            text.push(' ');
        }

        let text = text.trim().to_string();
        tracing::info!("Local Whisper transcribed: '{}'", text);
        Ok(text)
    }
}

#[cfg(feature = "voice-local")]
#[async_trait::async_trait]
impl VoiceProcessor for WhisperLocal {
    fn engine_name(&self) -> &'static str {
        "whisper-local"
    }
    async fn process_command(
        &self,
        config: &AppConfig,
        nats: Option<&crate::NatsConn>,
    ) -> Result<String, AppError> {
        // Local-first: read from configured input_path if available; in a real app this would capture mic audio
        let input = if let Some(path) = &config.voice.input_path {
            path.clone()
        } else {
            return Err(AppError::Voice(
                "no input_path configured for local STT".into(),
            ));
        };
        // Load model with default parameters
        let params = whisper_rs::WhisperContextParameters::default();
        let ctx = whisper_rs::WhisperContext::new_with_params(&self.model_path, params)
            .map_err(|e| AppError::Voice(format!("load whisper: {e}")))?;
        let mut state = ctx
            .create_state()
            .map_err(|e| AppError::Voice(format!("state: {e}")))?;
        // Read audio samples from WAV, propagating decode errors
        let mut rdr = hound::WavReader::open(&input).map_err(|e| AppError::Voice(e.to_string()))?;
        let raw_samples: Result<Vec<i16>, _> = rdr.samples::<i16>().collect();
        let samples: Vec<f32> = raw_samples
            .map_err(|e| AppError::Voice(format!("Failed to decode audio samples: {}", e)))?
            .into_iter()
            .map(|s| s as f32 / i16::MAX as f32)
            .collect();
        // Run full transcribe
        state
            .full(
                whisper_rs::FullParams::new(whisper_rs::SamplingStrategy::Greedy { best_of: 1 }),
                &samples,
            )
            .map_err(|e| AppError::Voice(format!("whisper run: {e}")))?;
        // Collect segments
        let num_segments = state
            .full_n_segments()
            .map_err(|e| AppError::Voice(format!("segments: {e}")))?;
        let mut text = String::new();
        for i in 0..num_segments {
            let seg = state
                .full_get_segment_text(i)
                .map_err(|e| AppError::Voice(format!("seg: {e}")))?;
            text.push_str(seg.trim());
            text.push(' ');
        }
        let text = text.trim().to_string();

        #[cfg(feature = "nats")]
        if let Some(nc) = nats {
            let payload = serde_json::json!({"text": text});
            nc.publish("events.voice", bytes::Bytes::from(payload.to_string()))
                .await
                .map_err(|e| AppError::Nats(e.to_string()))?;
        }
        Ok(text)
    }
}

/// Voice engine that uses OpenAI Whisper API for STT.
pub struct OpenAiWhisperVoice {
    pub api_key: String,
    pub base_url: String,
    /// The model to use for transcription (e.g., "whisper-1", "gpt-4o-transcribe").
    pub model: String,
}

impl OpenAiWhisperVoice {
    /// Transcribe an audio file directly using OpenAI Whisper API
    pub async fn transcribe_file(&self, audio_path: &std::path::Path) -> Result<String, AppError> {
        let client = reqwest::Client::new();
        let url = format!("{}/v1/audio/transcriptions", self.base_url);

        let bytes = std::fs::read(audio_path).map_err(|e| AppError::Voice(e.to_string()))?;
        let file_name = audio_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("audio.wav")
            .to_string();

        let part = reqwest::multipart::Part::bytes(bytes)
            .file_name(file_name)
            .mime_str("audio/wav")
            .map_err(|e| AppError::Voice(e.to_string()))?;

        let form = reqwest::multipart::Form::new()
            .text("model", self.model.clone())
            .part("file", part);

        let resp = client
            .post(url)
            .bearer_auth(&self.api_key)
            .multipart(form)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Voice(format!(
                "Whisper API error {}: {}",
                status, body
            )));
        }

        let v: serde_json::Value = resp.json().await?;
        let text = v["text"].as_str().unwrap_or("").to_string();

        tracing::info!("Transcribed audio using model '{}': '{}'", self.model, text);
        Ok(text)
    }
}

#[async_trait::async_trait]
impl VoiceProcessor for OpenAiWhisperVoice {
    fn engine_name(&self) -> &'static str {
        "openai-whisper"
    }
    async fn process_command(
        &self,
        config: &AppConfig,
        nats: Option<&crate::NatsConn>,
    ) -> Result<String, AppError> {
        let input = config
            .voice
            .input_path
            .clone()
            .ok_or_else(|| AppError::Voice("no input_path configured".into()))?;
        let client = reqwest::Client::new();
        let url = format!("{}/v1/audio/transcriptions", self.base_url);
        let bytes = std::fs::read(&input).map_err(|e| AppError::Voice(e.to_string()))?;
        let file_name = std::path::Path::new(&input)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("audio.wav")
            .to_string();
        let part = reqwest::multipart::Part::bytes(bytes)
            .file_name(file_name)
            .mime_str("audio/wav")
            .map_err(|e| AppError::Voice(e.to_string()))?;
        let form = reqwest::multipart::Form::new()
            .text("model", self.model.clone())
            .part("file", part);
        let resp = client
            .post(url)
            .bearer_auth(&self.api_key)
            .multipart(form)
            .send()
            .await?;
        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Voice(format!("whisper http error: {}", body)));
        }
        let v: serde_json::Value = resp.json().await?;
        let text = v["text"].as_str().unwrap_or("").to_string();
        #[cfg(feature = "nats")]
        if let Some(nc) = nats {
            let payload = serde_json::json!({"text": text});
            nc.publish("events.voice", bytes::Bytes::from(payload.to_string()))
                .await
                .map_err(|e| AppError::Nats(e.to_string()))?;
        }
        Ok(text)
    }
}

// ============================================================================
// Whisper Model Validation
// ============================================================================

/// Result of validating a Whisper model file
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WhisperModelValidation {
    pub is_valid: bool,
    pub file_exists: bool,
    pub file_size_mb: f64,
    pub is_ggml_format: bool,
    pub error: Option<String>,
}

/// Validate a Whisper model file
///
/// Checks:
/// 1. File exists
/// 2. File has reasonable size (> 10MB for smallest model)
/// 3. File starts with GGML magic bytes (0x67676d6c = "ggml")
pub fn validate_whisper_model(path: &std::path::Path) -> WhisperModelValidation {
    use std::io::Read;

    // Check if file exists
    if !path.exists() {
        return WhisperModelValidation {
            is_valid: false,
            file_exists: false,
            file_size_mb: 0.0,
            is_ggml_format: false,
            error: Some("Model file does not exist".to_string()),
        };
    }

    // Get file metadata
    let metadata = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(e) => {
            return WhisperModelValidation {
                is_valid: false,
                file_exists: true,
                file_size_mb: 0.0,
                is_ggml_format: false,
                error: Some(format!("Cannot read file metadata: {}", e)),
            };
        }
    };

    let file_size_mb = metadata.len() as f64 / (1024.0 * 1024.0);

    // Check minimum size (tiny model is ~75MB)
    if file_size_mb < 10.0 {
        return WhisperModelValidation {
            is_valid: false,
            file_exists: true,
            file_size_mb,
            is_ggml_format: false,
            error: Some(format!(
                "File too small ({:.1} MB). Whisper models are at least 75 MB.",
                file_size_mb
            )),
        };
    }

    // Check GGML magic bytes
    let is_ggml = match std::fs::File::open(path) {
        Ok(mut file) => {
            let mut magic = [0u8; 4];
            if file.read_exact(&mut magic).is_ok() {
                // GGML magic bytes (little-endian): "lmgg" = [0x6c, 0x6d, 0x67, 0x67]
                // This is "ggml" reversed due to little-endian storage
                // Also check for GGUF format: "GGUF" = [0x47, 0x47, 0x55, 0x46]
                &magic == b"lmgg" || &magic == b"ggml" || &magic == b"GGUF"
            } else {
                false
            }
        }
        Err(_) => false,
    };

    if !is_ggml {
        return WhisperModelValidation {
            is_valid: false,
            file_exists: true,
            file_size_mb,
            is_ggml_format: false,
            error: Some("File does not appear to be a valid GGML/GGUF model".to_string()),
        };
    }

    WhisperModelValidation {
        is_valid: true,
        file_exists: true,
        file_size_mb,
        is_ggml_format: true,
        error: None,
    }
}

/// Get the status of the active/default Whisper model
///
/// This prefers the user-configured local model path (if set in AppConfig)
/// and falls back to the recommended default model path. This ensures the
/// configuration UI reflects whichever model is actually in use.
pub fn get_default_model_status() -> (bool, std::path::PathBuf) {
    let config = crate::config::AppConfig::load();
    let path = config
        .voice
        .local_model_path
        .as_ref()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(crate::config::AppConfig::default_whisper_model_path);
    let exists = path.exists();
    (exists, path)
}
