//! Speech processing (facade + integration bridge).
//!
//! Pure types and domain logic live in [`gestura_core_audio::speech`].
//! This module re-exports them and provides integration methods that
//! depend on core-only types (security, LLM providers).

pub use gestura_core_audio::speech::*;

use crate::config::{AppConfig, AppConfigSecurityExt};
use crate::error::AppError;
use crate::llm_provider::{AgentContext, select_provider};
use std::path::Path;

/// Extension trait providing integration methods on [`SpeechProcessor`]
/// that depend on core-only types (secure storage, LLM provider selection).
///
/// Callers must `use gestura_core::speech::SpeechProcessorCoreExt` (or
/// `use gestura_core::SpeechProcessorCoreExt`) to access these methods.
#[async_trait::async_trait]
pub trait SpeechProcessorCoreExt {
    /// Transcribe audio file to text using core-owned STT provider selection.
    ///
    /// Uses the unified STT provider abstraction from `stt_provider` module.
    /// The provider is selected based on `AppConfig.voice.provider` and respects
    /// configured base_url and model settings.
    async fn transcribe_audio(&self, audio_path: &Path) -> Result<TranscriptionResult, AppError>;

    /// Process transcribed text with configured LLM provider.
    async fn process_with_llm(&self, text: &str) -> Result<LlmResponse, AppError>;
}

#[async_trait::async_trait]
impl SpeechProcessorCoreExt for SpeechProcessor {
    async fn transcribe_audio(&self, audio_path: &Path) -> Result<TranscriptionResult, AppError> {
        let app_config = AppConfig::load();
        // Use secure storage (keychain when enabled) for API key fallback chains.
        let secret_provider = crate::secrets::SecureStorageSecretProvider::new(
            crate::security::create_secure_storage(),
        );
        let provider =
            crate::stt_provider::select_provider(&app_config, Some(&secret_provider)).await;

        tracing::info!(
            "Transcribing audio with provider: {} (config.voice.provider={})",
            provider.provider_id(),
            app_config.voice.provider
        );

        provider.transcribe_file(audio_path).await
    }

    async fn process_with_llm(&self, text: &str) -> Result<LlmResponse, AppError> {
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
