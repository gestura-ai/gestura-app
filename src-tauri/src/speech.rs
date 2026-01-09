use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

use crate::audio_capture::record_audio;
use crate::config::AppConfig;
use crate::llm_provider::{AgentContext, select_provider};
use crate::voice::{OpenAiWhisperVoice, WhisperLocal};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeechConfig {
    pub stt_provider: String,
    pub llm_provider: String,
    pub openai_api_key: String,
    pub anthropic_api_key: String,
    pub google_api_key: String,
    pub azure_api_key: String,
    pub local_llm_endpoint: String,
    pub stt_timeout: u64,
    pub llm_timeout: u64,
    pub enable_fallback: bool,
    pub cache_responses: bool,
}

impl Default for SpeechConfig {
    fn default() -> Self {
        Self {
            stt_provider: "openai-whisper".to_string(),
            llm_provider: "openai-gpt".to_string(),
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
    pub fn new() -> Self {
        Self {
            config: Arc::new(Mutex::new(SpeechConfig::default())),
            is_recording: Arc::new(Mutex::new(false)),
        }
    }

    pub fn update_config(&self, config: SpeechConfig) {
        let mut current_config = self.config.lock().unwrap();
        *current_config = config;
        tracing::info!("Speech processor configuration updated");
    }

    pub async fn start_listening(&self, app: &AppHandle) -> Result<(), String> {
        {
            let mut recording = self.is_recording.lock().unwrap();
            if *recording {
                return Err("Already recording".to_string());
            }
            *recording = true;
        }

        tracing::info!("Starting speech capture and processing");

        // Use real microphone capture and voice processing
        let result = self.process_speech_workflow(app).await;

        {
            let mut recording = self.is_recording.lock().unwrap();
            *recording = false;
        }

        result
    }

    pub fn stop_listening(&self) -> Result<(), String> {
        let mut recording = self.is_recording.lock().unwrap();
        if !*recording {
            return Err("Not currently recording".to_string());
        }
        *recording = false;
        tracing::info!("Stopped speech capture");
        Ok(())
    }

    pub fn is_recording(&self) -> bool {
        *self.is_recording.lock().unwrap()
    }

    /// Real speech processing workflow using microphone capture and voice transcription
    async fn process_speech_workflow(&self, app: &AppHandle) -> Result<(), String> {
        // Load configuration from AppConfig instead of using stale SpeechConfig
        let app_config = AppConfig::load();

        // Map VoiceSettings to SpeechConfig
        let speech_config = SpeechConfig {
            stt_provider: match app_config.voice.provider.as_str() {
                "local" => "local-whisper".to_string(),
                "openai" => "openai-whisper".to_string(),
                _ => "local-whisper".to_string(), // default to local
            },
            llm_provider: app_config.llm.primary.clone(),
            openai_api_key: app_config.voice.openai_api_key.clone().unwrap_or_default(),
            anthropic_api_key: app_config.llm.anthropic.as_ref()
                .map(|a| a.api_key.clone())
                .unwrap_or_default(),
            google_api_key: String::new(),
            azure_api_key: String::new(),
            local_llm_endpoint: app_config.llm.ollama.as_ref()
                .map(|o| o.base_url.clone())
                .unwrap_or_else(|| "http://localhost:11434".to_string()),
            stt_timeout: 30,
            llm_timeout: 60,
            enable_fallback: true,
            cache_responses: true,
        };

        tracing::info!(
            "Starting speech workflow with STT: {}, LLM: {}",
            speech_config.stt_provider,
            app_config.llm.primary
        );

        // Step 1: Record audio from microphone with VAD (Voice Activity Detection)
        // Recording will continue until 4 seconds of silence is detected
        let temp_dir = std::env::temp_dir();
        let audio_path = temp_dir.join(format!(
            "gestura_audio_{}.wav",
            chrono::Utc::now().timestamp()
        ));

        // Duration parameter is ignored - VAD handles stopping after silence
        let duration = record_audio(Duration::from_secs(0), &audio_path)
            .await
            .map_err(|e| format!("Failed to record audio: {}", e))?;

        tracing::info!("Recorded {:.2}s of audio to {:?}", duration, audio_path);

        if duration < 0.5 {
            let _ = std::fs::remove_file(&audio_path);
            return Err("Recording too short - no audio captured".to_string());
        }

        // Step 2: Transcribe audio using voice processor
        let transcribed_text = self.transcribe_audio(&speech_config, &audio_path).await?;
        tracing::info!("Transcription: '{}'", transcribed_text);

        // Clean up temp file
        let _ = std::fs::remove_file(&audio_path);

        if transcribed_text.trim().is_empty() {
            return Err("No speech detected in audio".to_string());
        }

        // Step 3: Create chat session with transcription
        let session_id = self
            .create_chat_with_transcription(app, &transcribed_text)
            .await?;
        tracing::info!("Created chat session {} with transcription", session_id);

        // Step 4: Process with LLM for response
        let ai_response = self.process_with_llm(&transcribed_text).await?;
        tracing::info!("AI response: '{}'", ai_response);

        // Step 5: Send AI response to chat
        self.send_ai_response_to_chat(app, &session_id, &ai_response)
            .await?;

        Ok(())
    }

    /// Transcribe audio file using configured STT provider
    async fn transcribe_audio(
        &self,
        config: &SpeechConfig,
        audio_path: &Path,
    ) -> Result<String, String> {
        let app_config = AppConfig::load();

        match config.stt_provider.as_str() {
            "openai-whisper" => {
                if config.openai_api_key.is_empty() {
                    return Err("OpenAI API key not configured".to_string());
                }

                let voice_processor = OpenAiWhisperVoice {
                    api_key: config.openai_api_key.clone(),
                    base_url: "https://api.openai.com".to_string(),
                };

                voice_processor
                    .transcribe_file(audio_path)
                    .await
                    .map_err(|e| format!("Transcription failed: {}", e))
            }
            "local-whisper" => {
                // Get the model path from app config
                let model_path = app_config
                    .voice
                    .local_model_path
                    .clone()
                    .ok_or_else(|| {
                        "Local Whisper model path not configured. Please set the model path in Settings > Voice > Local Whisper Model Path.".to_string()
                    })?;

                // Verify the model file exists
                if !std::path::Path::new(&model_path).exists() {
                    return Err(format!(
                        "Local Whisper model file not found at: {}. Please download a whisper.cpp compatible model (.bin file).",
                        model_path
                    ));
                }

                tracing::info!("Using local Whisper model: {}", model_path);

                let whisper_local = WhisperLocal { model_path };

                // Run transcription in a blocking task since whisper-rs is synchronous
                let audio_path_owned = audio_path.to_path_buf();
                tokio::task::spawn_blocking(move || {
                    whisper_local.transcribe_file(&audio_path_owned)
                })
                .await
                .map_err(|e| format!("Transcription task failed: {}", e))?
                .map_err(|e| format!("Local Whisper transcription failed: {}", e))
            }
            _ => Err(format!("Unsupported STT provider: {}", config.stt_provider)),
        }
    }

    /// Process transcribed text with configured LLM provider
    async fn process_with_llm(&self, text: &str) -> Result<String, String> {
        let app_config = AppConfig::load();
        let provider = select_provider(
            &app_config,
            &AgentContext {
                agent_id: "speech".into(),
            },
        );

        tracing::info!("Processing with LLM provider: {}", app_config.llm.primary);

        provider
            .call(text)
            .await
            .map_err(|e| format!("LLM processing failed: {}", e))
    }

    #[allow(dead_code)]
    fn is_conversation(&self, text: &str) -> bool {
        // Simple heuristic to determine if this is a conversation vs command
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

    async fn create_chat_with_transcription(
        &self,
        app: &AppHandle,
        user_text: &str,
    ) -> Result<String, String> {
        tracing::info!("Creating chat window with transcription: '{}'", user_text);

        // Create a new chat session
        match crate::window_manager::create_new_chat_session() {
            Ok(session_id) => {
                tracing::info!("Created chat session {} for voice input", session_id);

                // Give the window time to load and set up event listeners
                // The webview needs to initialize JavaScript and set up Tauri event listeners
                // 1.5 seconds should be enough for the window to fully load
                tracing::info!("Waiting for chat window to initialize...");
                tokio::time::sleep(Duration::from_millis(1500)).await;
                tracing::info!("Chat window should be ready, emitting events");

                // Emit voice session start event so the frontend knows to expect messages
                if let Err(e) = app.emit(
                    "voice-session-started",
                    serde_json::json!({
                        "session_id": session_id,
                        "timestamp": chrono::Utc::now().to_rfc3339()
                    }),
                ) {
                    tracing::warn!("Failed to emit voice-session-started: {}", e);
                } else {
                    tracing::info!("Emitted voice-session-started event");
                }

                // Send the transcribed text as a user message to the chat
                self.send_user_message_to_chat(app, &session_id, user_text)
                    .await?;

                Ok(session_id)
            }
            Err(e) => Err(format!("Failed to create chat session: {}", e)),
        }
    }

    async fn send_user_message_to_chat(
        &self,
        app: &AppHandle,
        session_id: &str,
        message: &str,
    ) -> Result<(), String> {
        tracing::info!("Sending user message to chat {}: '{}'", session_id, message);

        // Emit a custom event that the chat window listens to
        let payload = serde_json::json!({
            "session_id": session_id,
            "type": "user",
            "message": message,
            "timestamp": chrono::Utc::now().to_rfc3339()
        });
        tracing::info!("Emitting chat-message event with payload: {:?}", payload);

        if let Err(e) = app.emit("chat-message", payload) {
            tracing::error!("Failed to emit chat-message: {}", e);
            return Err(format!("Failed to emit user message: {}", e));
        }

        tracing::info!("Successfully emitted chat-message event for user input");
        Ok(())
    }

    async fn send_ai_response_to_chat(
        &self,
        app: &AppHandle,
        session_id: &str,
        response: &str,
    ) -> Result<(), String> {
        tracing::info!("Sending AI response to chat {}: '{}'", session_id, response);

        // Simulate AI thinking time
        tokio::time::sleep(Duration::from_millis(500)).await;

        if let Err(e) = app.emit(
            "chat-message",
            serde_json::json!({
                "session_id": session_id,
                "type": "assistant",
                "message": response,
                "timestamp": chrono::Utc::now().to_rfc3339()
            }),
        ) {
            return Err(format!("Failed to emit AI response: {}", e));
        }

        Ok(())
    }

    #[allow(dead_code)]
    async fn execute_system_command(&self, command: &str) -> Result<(), String> {
        tracing::info!("Executing system command: {}", command);

        // TODO: Implement actual system command execution
        // This would parse the command and execute appropriate actions
        // Examples: "open calendar", "show weather", "start music", etc.

        Ok(())
    }
}

// Global speech processor instance
lazy_static::lazy_static! {
    static ref SPEECH_PROCESSOR: SpeechProcessor = SpeechProcessor::new();
}

pub fn get_speech_processor() -> &'static SpeechProcessor {
    &SPEECH_PROCESSOR
}

pub async fn start_speech_listening(app: &AppHandle) -> Result<(), String> {
    SPEECH_PROCESSOR.start_listening(app).await
}

pub fn stop_speech_listening() -> Result<(), String> {
    SPEECH_PROCESSOR.stop_listening()
}

pub fn is_speech_recording() -> bool {
    SPEECH_PROCESSOR.is_recording()
}

pub fn update_speech_config(config: SpeechConfig) {
    SPEECH_PROCESSOR.update_config(config);
}
