//! Speech-to-text provider abstraction.
//!
//! This mirrors the `llm_provider` module: a small trait, provider implementations,
//! and a selection function based on `AppConfig`.

use std::path::{Path, PathBuf};

use crate::config::AppConfig;
use crate::error::AppError;
use crate::speech::TranscriptionResult;

/// Unified STT interface (async).
///
/// Provider implementations must be `Send + Sync` so they can be used behind
/// a `Box<dyn SttProvider>` across async boundaries.
#[async_trait::async_trait]
pub trait SttProvider: Send + Sync {
    /// Returns a stable provider id for logs/telemetry.
    fn provider_id(&self) -> &'static str;

    /// Transcribe an audio file into text.
    async fn transcribe_file(&self, audio_path: &Path) -> Result<TranscriptionResult, AppError>;
}

/// A provider that returns a helpful error when STT is not configured.
pub struct UnconfiguredSttProvider {
    message: String,
}

impl UnconfiguredSttProvider {
    /// Create a new unconfigured provider with a user-facing message.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[async_trait::async_trait]
impl SttProvider for UnconfiguredSttProvider {
    fn provider_id(&self) -> &'static str {
        "unconfigured"
    }

    async fn transcribe_file(&self, _audio_path: &Path) -> Result<TranscriptionResult, AppError> {
        Err(AppError::Voice(self.message.clone()))
    }
}

/// OpenAI (or OpenAI-compatible) STT provider.
pub struct OpenAiSttProvider {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
}

impl OpenAiSttProvider {
    /// Build the OpenAI transcription endpoint URL from the configured base URL.
    pub fn transcription_url(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        format!("{base}/v1/audio/transcriptions")
    }
}

#[async_trait::async_trait]
impl SttProvider for OpenAiSttProvider {
    fn provider_id(&self) -> &'static str {
        "openai"
    }

    async fn transcribe_file(&self, audio_path: &Path) -> Result<TranscriptionResult, AppError> {
        let client = reqwest::Client::new();

        let bytes = std::fs::read(audio_path)
            .map_err(|e| AppError::Voice(format!("Failed to read audio file: {e}")))?;
        let file_name = audio_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("audio.wav")
            .to_string();

        let part = reqwest::multipart::Part::bytes(bytes)
            .file_name(file_name)
            .mime_str("audio/wav")
            .map_err(|e| AppError::Voice(format!("Invalid multipart audio part: {e}")))?;

        let form = reqwest::multipart::Form::new()
            .text("model", self.model.clone())
            .part("file", part);

        let resp = client
            .post(self.transcription_url())
            .bearer_auth(&self.api_key)
            .multipart(form)
            .send()
            .await
            .map_err(|e| AppError::Voice(format!("OpenAI STT request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Voice(format!(
                "OpenAI STT API error {status}: {body}"
            )));
        }

        #[derive(serde::Deserialize)]
        struct WhisperResponse {
            text: String,
        }

        let result: WhisperResponse = resp
            .json()
            .await
            .map_err(|e| AppError::Voice(format!("Failed to parse OpenAI STT response: {e}")))?;

        Ok(TranscriptionResult {
            text: result.text,
            duration_secs: 0.0,
            audio_path: Some(audio_path.to_path_buf()),
            provider: "openai-whisper".to_string(),
        })
    }
}

/// Local Whisper provider (whisper-rs / whisper.cpp models).
#[cfg(feature = "voice-local")]
pub struct LocalWhisperProvider {
    pub model_path: PathBuf,
}

#[cfg(feature = "voice-local")]
#[async_trait::async_trait]
impl SttProvider for LocalWhisperProvider {
    fn provider_id(&self) -> &'static str {
        "local-whisper"
    }

    async fn transcribe_file(&self, audio_path: &Path) -> Result<TranscriptionResult, AppError> {
        use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

        let ctx = WhisperContext::new_with_params(
            self.model_path
                .to_str()
                .ok_or_else(|| AppError::Voice("Invalid model path encoding".to_string()))?,
            WhisperContextParameters::default(),
        )
        .map_err(|e| AppError::Voice(format!("Failed to load Whisper model: {e}")))?;

        let samples = crate::speech::load_audio_samples_16khz_mono(audio_path)?;
        let duration_secs = samples.len() as f32 / 16000.0;

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(Some("en"));
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_translate(false);
        params.set_no_context(true);
        params.set_single_segment(false);

        let mut state = ctx
            .create_state()
            .map_err(|e| AppError::Voice(format!("Failed to create Whisper state: {e}")))?;
        state
            .full(params, &samples)
            .map_err(|e| AppError::Voice(format!("Whisper transcription failed: {e}")))?;

        let num_segments = state
            .full_n_segments()
            .map_err(|e| AppError::Voice(format!("Failed to get segment count: {e}")))?;
        let mut text = String::new();
        for i in 0..num_segments {
            if let Ok(seg) = state.full_get_segment_text(i) {
                text.push_str(seg.trim());
                text.push(' ');
            }
        }
        let text = text.trim().to_string();

        Ok(TranscriptionResult {
            text,
            duration_secs,
            audio_path: Some(audio_path.to_path_buf()),
            provider: "local-whisper".to_string(),
        })
    }
}

/// Select an STT provider from app configuration.
///
/// This is intentionally conservative: if required fields (like API keys or
/// model paths) are missing, it returns an `UnconfiguredSttProvider` that
/// provides an actionable error message.
pub fn select_provider(config: &AppConfig) -> Box<dyn SttProvider> {
    match config.voice.provider.as_str() {
        "openai" => {
            let api_key = config.voice.openai_api_key.clone().unwrap_or_default();
            let api_key = if !api_key.is_empty() {
                api_key
            } else {
                // Back-compat: allow re-using LLM OpenAI key if voice key is not set.
                config
                    .llm
                    .openai
                    .as_ref()
                    .map(|c| c.api_key.clone())
                    .unwrap_or_default()
            };

            if api_key.is_empty() {
                return Box::new(UnconfiguredSttProvider::new(
                    "OpenAI STT selected but no API key configured (voice.openai_api_key).",
                ));
            }

            let base_url = config
                .voice
                .openai_base_url
                .clone()
                .unwrap_or_else(|| "https://api.openai.com".to_string());
            let model = config
                .voice
                .openai_model
                .clone()
                .unwrap_or_else(|| "gpt-4o-transcribe".to_string());

            Box::new(OpenAiSttProvider {
                api_key,
                base_url,
                model,
            })
        }
        "local" => {
            #[cfg(feature = "voice-local")]
            {
                match crate::speech::resolve_whisper_model_path(config) {
                    Ok(model_path) => Box::new(LocalWhisperProvider { model_path }),
                    Err(e) => Box::new(UnconfiguredSttProvider::new(e.to_string())),
                }
            }
            #[cfg(not(feature = "voice-local"))]
            {
                Box::new(UnconfiguredSttProvider::new(
                    "Local Whisper selected but the 'voice-local' feature is disabled.",
                ))
            }
        }
        "none" => Box::new(UnconfiguredSttProvider::new(
            "STT provider is disabled (voice.provider=none).",
        )),
        other => Box::new(UnconfiguredSttProvider::new(format!(
            "Unknown STT provider '{other}'. Supported: openai | local | none"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_transcription_url_uses_base_url() {
        let p = OpenAiSttProvider {
            api_key: "x".into(),
            base_url: "https://example.com/".into(),
            model: "whisper-1".into(),
        };
        assert_eq!(
            p.transcription_url(),
            "https://example.com/v1/audio/transcriptions"
        );
    }
}
