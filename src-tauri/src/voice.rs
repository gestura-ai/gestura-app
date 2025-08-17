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
        tracing::info!("Transcribing with faster-whisper model: {}", self.model_path);

        // Simulate processing time
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        Ok("Hello, this is a faster-whisper transcription.".to_string())
    }
}

#[cfg(feature = "voice-faster-whisper")]
#[async_trait::async_trait]
impl VoiceProcessor for WhisperFasterLocal {
    fn engine_name(&self) -> &'static str { "faster-whisper-local" }

    async fn process_command(&self, config: &AppConfig, nats: Option<&crate::NatsConn>) -> Result<String, AppError> {
        let input = if let Some(path) = &config.voice.input_path {
            path.clone()
        } else {
            return Err(AppError::Voice("no input_path configured".into()))
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
            let _ = nc.publish("events.voice", bytes::Bytes::from(payload.to_string())).await;
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
    fn engine_name(&self) -> &'static str { "unknown" }
    /// Process mic audio to text and optionally publish to NATS.
    async fn process_command(&self, _config: &AppConfig, nats: Option<&crate::NatsConn>) -> Result<String, AppError>;
}

/// Mock implementation that returns a fixed string and publishes to NATS if available.
pub struct MockVoice;

#[async_trait::async_trait]
impl VoiceProcessor for MockVoice {
    fn engine_name(&self) -> &'static str { "mock" }
    async fn process_command(&self, _config: &AppConfig, nats: Option<&crate::NatsConn>) -> Result<String, AppError> {
        let text = "mock voice command".to_string();
        if let Some(nc) = nats {
        #[cfg(feature = "nats")]
        {
            let payload = serde_json::json!({"text": text});
            nc.publish("events.voice", bytes::Bytes::from(payload.to_string())).await.map_err(|e| AppError::Nats(e.to_string()))?;
        }
        }
        Ok(text)
    }

}

#[cfg(not(feature = "voice-local"))]
pub struct WhisperLocal { pub model_path: String }
#[cfg(not(feature = "voice-local"))]
#[async_trait::async_trait]
impl VoiceProcessor for WhisperLocal {
    fn engine_name(&self) -> &'static str { "whisper-local" }
    async fn process_command(&self, _config: &AppConfig, _nats: Option<&crate::NatsConn>) -> Result<String, AppError> {
        Err(AppError::Voice("voice-local feature disabled".into()))
    }
}

#[cfg(feature = "voice-local")]
/// Voice engine that uses local Whisper.cpp (whisper-rs) for offline STT.
pub struct WhisperLocal { pub model_path: String }

#[cfg(feature = "voice-local")]
#[async_trait::async_trait]
impl VoiceProcessor for WhisperLocal {
    fn engine_name(&self) -> &'static str { "whisper-local" }
    async fn process_command(&self, config: &AppConfig, nats: Option<&crate::NatsConn>) -> Result<String, AppError> {
        // Local-first: read from configured input_path if available; in a real app this would capture mic audio
        let input = if let Some(path) = &config.voice.input_path { path.clone() } else { return Err(AppError::Voice("no input_path configured for local STT".into())) };
        // Load model with default parameters
        let params = whisper_rs::WhisperContextParameters::default();
        let ctx = whisper_rs::WhisperContext::new_with_params(&self.model_path, params).map_err(|e| AppError::Voice(format!("load whisper: {e}")))?;
        let mut state = ctx.create_state().map_err(|e| AppError::Voice(format!("state: {e}")))?;
        // Read audio samples from WAV
        let mut rdr = hound::WavReader::open(&input).map_err(|e| AppError::Voice(e.to_string()))?;
        let samples: Vec<f32> = rdr.samples::<i16>()
            .filter_map(Result::ok)
            .map(|s| s as f32 / i16::MAX as f32)
            .collect();
        // Run full transcribe
        state.full(whisper_rs::FullParams::new(whisper_rs::SamplingStrategy::Greedy { best_of: 1 }), &samples)
            .map_err(|e| AppError::Voice(format!("whisper run: {e}")))?;
        // Collect segments
        let num_segments = state.full_n_segments().map_err(|e| AppError::Voice(format!("segments: {e}")))?;
        let mut text = String::new();
        for i in 0..num_segments {
            let seg = state.full_get_segment_text(i).map_err(|e| AppError::Voice(format!("seg: {e}")))?;
            text.push_str(seg.trim());
            text.push(' ');
        }
        let text = text.trim().to_string();

        #[cfg(feature = "nats")]
        if let Some(nc) = nats {
            let payload = serde_json::json!({"text": text});
            nc.publish("events.voice", bytes::Bytes::from(payload.to_string())).await.map_err(|e| AppError::Nats(e.to_string()))?;
        }
        Ok(text)
    }
}

/// Voice engine that uses OpenAI Whisper API for STT.
pub struct OpenAiWhisperVoice { pub api_key: String, pub base_url: String }

#[async_trait::async_trait]
impl VoiceProcessor for OpenAiWhisperVoice {
    fn engine_name(&self) -> &'static str { "openai-whisper" }
    async fn process_command(&self, config: &AppConfig, nats: Option<&crate::NatsConn>) -> Result<String, AppError> {
        let input = config.voice.input_path.clone().ok_or_else(|| AppError::Voice("no input_path configured".into()))?;
        let client = reqwest::Client::new();
        let url = format!("{}/v1/audio/transcriptions", self.base_url);
        let bytes = std::fs::read(&input).map_err(|e| AppError::Voice(e.to_string()))?;
        let file_name = std::path::Path::new(&input).file_name().and_then(|s| s.to_str()).unwrap_or("audio.wav").to_string();
        let part = reqwest::multipart::Part::bytes(bytes)
            .file_name(file_name)
            .mime_str("audio/wav").map_err(|e| AppError::Voice(e.to_string()))?;
        let form = reqwest::multipart::Form::new()
            .text("model", "whisper-1")
            .part("file", part);
        let resp = client.post(url)
            .bearer_auth(&self.api_key)
            .multipart(form)
            .send().await?;
        if !resp.status().is_success() { return Err(AppError::Voice(format!("whisper http {}", resp.status()))); }
        let v: serde_json::Value = resp.json().await?;
        let text = v["text"].as_str().unwrap_or("").to_string();
        #[cfg(feature = "nats")]
        if let Some(nc) = nats {
            let payload = serde_json::json!({"text": text});
            nc.publish("events.voice", bytes::Bytes::from(payload.to_string())).await.map_err(|e| AppError::Nats(e.to_string()))?;
        }
        Ok(text)
    }
}

