//! Voice engine selection utilities: prefer local engine when available
use crate::{AppConfig, AppError};
use crate::voice::{VoiceProcessor, MockVoice, OpenAiWhisperVoice};
#[cfg(feature = "voice-local")]
use crate::voice::WhisperLocal;

/// Select a voice processor based on configuration, preferring local.
/// - If `voice.local_model_path` is Some, select WhisperLocal (requires feature `voice-local`).
/// - Else, if `voice.openai_api_key` is Some, select OpenAI Whisper.
/// - Else, fall back to MockVoice.
pub fn select_voice(config: &AppConfig) -> Box<dyn VoiceProcessor> {
    // Prefer Faster-Whisper when enabled
    if config.voice.local_model_path.is_some() {
        #[cfg(feature = "voice-faster-whisper")]
        {
            return Box::new(crate::voice::WhisperFasterLocal::new(config.voice.local_model_path.clone().unwrap()));
        }
        #[cfg(feature = "voice-local")]
        {
            return Box::new(WhisperLocal { model_path: config.voice.local_model_path.clone().unwrap() });
        }
    }

    // Fallback to OpenAI Whisper if configured
    if let Some(api_key) = &config.voice.openai_api_key {
        let base = config.voice.openai_base_url.clone().unwrap_or_else(|| "https://api.openai.com".into());
        return Box::new(OpenAiWhisperVoice { api_key: api_key.clone(), base_url: base });
    }

    // Last resort: mock
    Box::new(MockVoice)
}

/// Validate the minimal configuration required to run a one-shot transcription with the selected engine.
/// Returns Ok(()) if the configuration is sufficient, otherwise a descriptive error.
pub fn validate_voice_config_for_run(config: &AppConfig, engine: &dyn VoiceProcessor) -> Result<(), AppError> {
    let name = engine.engine_name();
    // All real engines require an input wav path
    if name != "mock" && config.voice.input_path.is_none() {
        return Err(AppError::Voice("voice.input_path must be set to a WAV file path".into()));
    }
    if name == "whisper-local" && config.voice.local_model_path.is_none() {
        return Err(AppError::Voice("voice.local_model_path must be set to a whisper.cpp .bin model".into()));
    }
    if name == "openai-whisper" && config.voice.openai_api_key.as_deref().unwrap_or("").is_empty() {
        return Err(AppError::Voice("voice.openai_api_key must be set".into()));
    }
    Ok(())
}

