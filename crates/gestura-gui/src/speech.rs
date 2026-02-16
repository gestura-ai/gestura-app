use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

use crate::audio_capture::record_audio;
use crate::config::{AppConfig, AppConfigSecurityExt};

use gestura_core::secrets::SecureStorageSecretProvider;
use gestura_core::stt_provider::{SttProvider, select_provider_with_session_voice_config};

/// Poll interval (ms) for checking whether the user requested cancellation.
const CANCEL_POLL_INTERVAL_MS: u64 = 50;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeechConfig {
    pub stt_provider: String,
    pub llm_provider: String,
    pub openai_api_key: String,
    /// OpenAI base URL for Whisper API (defaults to https://api.openai.com)
    pub openai_base_url: String,
    /// OpenAI model for transcription (e.g., "whisper-1", "gpt-4o-transcribe")
    pub openai_model: String,
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
            openai_base_url: "https://api.openai.com".to_string(),
            openai_model: gestura_core::DEFAULT_OPENAI_STT_MODEL.to_string(),
            anthropic_api_key: String::new(),
            google_api_key: String::new(),
            azure_api_key: String::new(),
            local_llm_endpoint: gestura_core::DEFAULT_OLLAMA_BASE_URL.to_string(),
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

    /// Start microphone capture and run the speech workflow.
    ///
    /// ## Cancel / timeout semantics
    /// - Cancellation is requested via [`Self::stop_listening`].
    /// - Cancellation is **best-effort and idempotent**:
    ///   - recording is stopped via the core audio-capture stop flag
    ///   - transcription is cancelled by dropping the in-flight future when possible
    /// - A user-initiated cancel is treated as `Ok(())` (not an error) to avoid
    ///   showing error UX for intentional stops.
    pub async fn start_listening(&self, app: &AppHandle) -> Result<(), String> {
        {
            let mut recording = self.is_recording.lock().unwrap();
            if *recording {
                return Err("Already recording".to_string());
            }
            *recording = true;
        }

        tracing::info!("Starting speech capture and processing");

        // Use real microphone capture and voice processing.
        // Note: user cancellation is treated as success (see `process_speech_workflow`).
        let result = self.process_speech_workflow(app).await;

        {
            let mut recording = self.is_recording.lock().unwrap();
            *recording = false;
        }

        result
    }

    /// Request that an in-flight speech workflow stop.
    ///
    /// This method is intentionally **idempotent**: calling it when not recording
    /// still signals the core audio-capture stop flag, ensuring any in-flight
    /// recording loop can observe the request.
    pub fn stop_listening(&self) -> Result<(), String> {
        let was_recording = {
            let mut recording = self.is_recording.lock().unwrap();
            let was_recording = *recording;
            *recording = false;
            was_recording
        };

        // Signal the audio capture to stop immediately.
        crate::audio_capture::request_stop_recording();

        tracing::info!(was_recording, "Stop listening requested");
        Ok(())
    }

    pub fn is_recording(&self) -> bool {
        *self.is_recording.lock().unwrap()
    }

    /// Wait until the workflow should be cancelled.
    ///
    /// Cancellation is signaled either via the global audio-capture stop flag
    /// (set by `stop_listening`) or by `is_recording` being set to false.
    async fn wait_for_cancel_request(&self) {
        loop {
            if crate::audio_capture::is_stop_requested() || !self.is_recording() {
                return;
            }

            tokio::time::sleep(Duration::from_millis(CANCEL_POLL_INTERVAL_MS)).await;
        }
    }

    /// Real speech processing workflow using microphone capture and voice transcription
    async fn process_speech_workflow(&self, app: &AppHandle) -> Result<(), String> {
        // Load configuration from AppConfig instead of using stale SpeechConfig
        let app_config = AppConfig::load();

        // Resolve session-scoped voice/STT overrides for the active voice target session.
        //
        // Voice input is routed to the "active" agent session (focused or most-recent).
        // If the user selected a per-session STT provider/model in the Providers panel,
        // apply that override here before recording/transcribing.
        let active_voice_session_id = crate::window_manager::get_active_agent_for_voice();
        let session_voice_config = active_voice_session_id
            .as_deref()
            .and_then(crate::window_manager::get_session_voice_config);

        tracing::info!(
            active_voice_session_id = ?active_voice_session_id,
            session_voice_config = ?session_voice_config,
            config_voice_provider = %app_config.voice.provider,
            "Resolved voice/STT config inputs (core applies precedence rules)"
        );

        // Step 1: Record audio from microphone with VAD (Voice Activity Detection)
        // Recording will continue until 4 seconds of silence is detected
        let temp_dir = std::env::temp_dir();
        let audio_path = temp_dir.join(format!(
            "gestura_audio_{}.wav",
            chrono::Utc::now().timestamp()
        ));

        // Duration parameter is ignored - VAD handles stopping after silence.
        let duration = match record_audio(Duration::from_secs(0), &audio_path).await {
            Ok(d) => d,
            Err(e) => {
                // Best-effort cleanup if a file was created.
                let _ = std::fs::remove_file(&audio_path);

                // User-initiated cancel is treated as success.
                if crate::audio_capture::is_stop_requested() || !self.is_recording() {
                    tracing::info!("Speech workflow cancelled during recording");
                    return Ok(());
                }

                return Err(format!("Failed to record audio: {}", e));
            }
        };

        tracing::info!("Recorded {:.2}s of audio to {:?}", duration, audio_path);

        // If the user cancelled, stop here and do not proceed to transcription.
        if crate::audio_capture::is_stop_requested() || !self.is_recording() {
            let _ = std::fs::remove_file(&audio_path);
            tracing::info!("Speech workflow cancelled after recording; skipping transcription");
            return Ok(());
        }

        if duration < 0.5 {
            let _ = std::fs::remove_file(&audio_path);
            return Err("Recording too short - no audio captured".to_string());
        }

        // Step 2: Transcribe audio using core-owned STT selection + transcription.
        // Cancellation is best-effort: if the user clicks "Stop" while transcribing,
        // we return success and drop the in-flight future (cancels HTTP requests).
        let transcribed_text_result = tokio::select! {
            _ = self.wait_for_cancel_request() => {
                let _ = std::fs::remove_file(&audio_path);
                tracing::info!("Speech workflow cancelled before/during transcription");
                return Ok(());
            }
            r = self.transcribe_audio(&app_config, session_voice_config.as_ref(), &audio_path) => r,
        };

        let transcribed_text = match transcribed_text_result {
            Ok(text) => text,
            Err(e) => {
                let _ = std::fs::remove_file(&audio_path);
                return Err(e);
            }
        };

        tracing::info!("Transcription: '{}'", transcribed_text);

        // Clean up temp file
        let _ = std::fs::remove_file(&audio_path);

        if transcribed_text.trim().is_empty() {
            return Err("No speech detected in audio".to_string());
        }

        // Step 3: Create agent session with transcription
        // The frontend will handle the LLM call via streaming when it receives the agent-message event
        let session_id = self
            .create_agent_with_transcription(app, &transcribed_text)
            .await?;
        tracing::info!(
            "Created agent session {} with transcription - frontend will handle LLM streaming",
            session_id
        );

        // Note: Steps 4 and 5 (LLM processing and sending AI response) are now handled by the frontend
        // via process_chat_message_streaming when it receives the agent-message event with type: "user"


        Ok(())
    }

    /// Transcribe an audio file using core-owned STT selection.
    ///
    /// The GUI layer is intentionally a thin adapter:
    /// - it provides session-scoped overrides (if any)
    /// - it wires secure storage (keychain/mock) into core via `SecureStorageSecretProvider`
    /// - it maps core errors into user-facing strings
    async fn transcribe_audio(
        &self,
        app_config: &AppConfig,
        session_voice_config: Option<&crate::window_manager::SessionVoiceConfig>,
        audio_path: &Path,
    ) -> Result<String, String> {
        let storage = crate::security::create_secure_storage();
        let secrets = SecureStorageSecretProvider::new(storage);

        let provider: Box<dyn SttProvider> = select_provider_with_session_voice_config(
            app_config,
            session_voice_config,
            Some(&secrets),
        )
        .await;

        tracing::info!(
            provider_id = %provider.provider_id(),
            "Selected STT provider (core-owned selection)"
        );

        let result = provider
            .transcribe_file(audio_path)
            .await
            .map_err(|e| format!("Transcription failed: {e}"))?;

        Ok(result.text)
    }

    /// Process transcribed text with configured LLM provider via AgentPipeline.
    ///
    /// Note: Currently unused as the frontend handles LLM streaming directly.
    /// This method is available for backend-driven LLM processing scenarios:
    /// - Voice-only mode without GUI
    /// - Batch processing of voice commands
    /// - Fallback when frontend streaming is unavailable
    #[allow(dead_code)]
    pub async fn process_with_llm(&self, text: &str) -> Result<String, String> {
        use gestura_core::{AgentPipeline, AgentRequest, RequestSource};

        let app_config = AppConfig::load();
        tracing::info!(
            "Processing voice input with AgentPipeline, LLM provider: {}",
            app_config.llm.primary
        );

        // Build the agent request for voice input
        let request = AgentRequest::new(text)
            .with_streaming(false)
            .with_source(RequestSource::GuiVoice);

        // Create the pipeline and process the request
        let pipeline = AgentPipeline::new(app_config);
        let response = pipeline
            .process_blocking(request)
            .await
            .map_err(|e| format!("AgentPipeline processing failed: {}", e))?;

        Ok(response.content)
    }

    /// Determine if transcribed text is a conversation vs a direct command.
    /// Returns true for conversational queries that should go to LLM,
    /// false for direct system commands that can be executed immediately.
    fn is_conversation(&self, text: &str) -> bool {
        // Simple heuristic to determine if this is a conversation vs command
        let conversation_keywords = [
            "help", "what", "how", "can you", "please", "tell me", "explain",
        ];
        let command_keywords = [
            "open", "close", "start", "stop", "launch", "quit", "show", "hide", "search", "run",
            "volume", "mute", "louder", "quieter",
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

    /// Route voice input to either system command execution or LLM agent.
    /// Direct commands (open, search, volume, etc.) are executed immediately.
    /// Conversational queries are sent to the agent for LLM processing.
    pub async fn route_voice_input(&self, app: &AppHandle, text: &str) -> Result<(), String> {
        if self.is_conversation(text) {
            tracing::info!("Routing voice input to agent: '{}'", text);
            self.create_agent_with_transcription(app, text).await?;
        } else {
            tracing::info!("Routing voice input to system command: '{}'", text);
            self.execute_system_command(text).await?;
        }
        Ok(())
    }

    async fn create_agent_with_transcription(
        &self,
        app: &AppHandle,
        user_text: &str,
    ) -> Result<String, String> {
        tracing::info!("Setting up agent for transcription: '{}'", user_text);

        // Check if there's an active agent session to use
        let (session_id, is_new_session) =
            if let Some(existing_session) = crate::window_manager::get_active_agent_for_voice() {
                tracing::info!(
                    "Using existing agent session {} for voice input",
                    existing_session
                );
                (existing_session, false)
            } else {
                // Create a new agent session
                match crate::window_manager::create_new_agent_session() {
                    Ok(new_session_id) => {
                        tracing::info!(
                            "Created new agent session {} for voice input",
                            new_session_id
                        );
                        (new_session_id, true)
                    }
                    Err(e) => return Err(format!("Failed to create agent session: {}", e)),
                }
            };

        // Only wait for window initialization if we created a new session
        if is_new_session {
            // Give the window time to load and set up event listeners
            // The webview needs to initialize JavaScript and set up Tauri event listeners
            // 1.5 seconds should be enough for the window to fully load
            tracing::info!("Waiting for new agent window to initialize...");
            tokio::time::sleep(Duration::from_millis(1500)).await;
            tracing::info!("Agent window should be ready, emitting events");
        }

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

        // Send the transcribed text as a user message to the agent
        self.send_user_message_to_agent(app, &session_id, user_text)
            .await?;

        Ok(session_id)
    }

    async fn send_user_message_to_agent(
        &self,
        app: &AppHandle,
        session_id: &str,
        message: &str,
    ) -> Result<(), String> {
        tracing::info!("Sending user message to agent {}: '{}'", session_id, message);

        // Get the window label for this session to target the specific window
        let window_label = crate::window_manager::get_session_window_label(session_id)
            .ok_or_else(|| format!("No window found for session {}", session_id))?;

        tracing::info!(
            "Targeting window '{}' for session {}",
            window_label,
            session_id
        );

        // Emit a custom event to the specific agent window only
        let payload = serde_json::json!({
            "session_id": session_id,
            "type": "user",
            "message": message,
            "timestamp": chrono::Utc::now().to_rfc3339()
        });
        tracing::info!(
            "Emitting agent-message event to window '{}' with payload: {:?}",
            window_label,
            payload
        );

        if let Err(e) = app.emit_to(&window_label, "agent-message", payload) {
            tracing::error!(
                "Failed to emit agent-message to window '{}': {}",
                window_label,
                e
            );
            return Err(format!("Failed to emit user message: {}", e));
        }

        tracing::info!(
            "Successfully emitted agent-message event to window '{}'",
            window_label
        );
        Ok(())
    }

    /// Send AI response to agent window.
    ///
    /// Note: Currently unused as the frontend handles LLM streaming directly.
    /// This method is available for backend-driven agent responses:
    /// - Voice-only mode without GUI interaction
    /// - Batch processing results
    /// - Fallback when frontend streaming is unavailable
    #[allow(dead_code)]
    pub async fn send_ai_response_to_agent(
        &self,
        app: &AppHandle,
        session_id: &str,
        response: &str,
    ) -> Result<(), String> {
        tracing::info!("Sending AI response to agent {}: '{}'", session_id, response);

        // Get the window label for this session to target the specific window
        let window_label = crate::window_manager::get_session_window_label(session_id)
            .ok_or_else(|| format!("No window found for session {}", session_id))?;

        // Simulate AI thinking time
        tokio::time::sleep(Duration::from_millis(500)).await;

        if let Err(e) = app.emit_to(
            &window_label,
            "agent-message",
            serde_json::json!({
                "session_id": session_id,
                "type": "assistant",
                "message": response,
                "timestamp": chrono::Utc::now().to_rfc3339()
            }),
        ) {
            return Err(format!(
                "Failed to emit AI response to window '{}': {}",
                window_label, e
            ));
        }

        tracing::info!(
            "Successfully emitted AI response to window '{}'",
            window_label
        );
        Ok(())
    }

    /// Execute a system command from voice input.
    /// Parses the command intent and routes to the appropriate handler.
    async fn execute_system_command(&self, command: &str) -> Result<(), String> {
        tracing::info!("Executing system command: {}", command);

        let command_lower = command.to_lowercase();

        // Parse command intent and execute appropriate action
        if command_lower.starts_with("open ") {
            let target = command_lower.strip_prefix("open ").unwrap_or("");
            return self.execute_open_command(target).await;
        }
        if command_lower.starts_with("launch ") {
            let target = command_lower.strip_prefix("launch ").unwrap_or("");
            return self.execute_open_command(target).await;
        }
        if command_lower.starts_with("search ") {
            let query = command_lower.strip_prefix("search ").unwrap_or("");
            return self.execute_search_command(query).await;
        }
        if command_lower.starts_with("run ") {
            let cmd = command
                .strip_prefix("run ")
                .or_else(|| command.strip_prefix("Run "))
                .unwrap_or("");
            return self.execute_shell_command(cmd).await;
        }
        if command_lower.contains("volume up") || command_lower.contains("louder") {
            return self.execute_volume_command(true).await;
        }
        if command_lower.contains("volume down") || command_lower.contains("quieter") {
            return self.execute_volume_command(false).await;
        }
        if command_lower.contains("mute") {
            return self.execute_mute_command().await;
        }

        // Unknown command - log and return success (don't fail on unrecognized commands)
        tracing::warn!("Unrecognized system command: {}", command);
        Ok(())
    }

    /// Execute an "open" command to launch applications or URLs.
    /// Maps common voice targets to actual application names.
    async fn execute_open_command(&self, target: &str) -> Result<(), String> {
        use std::process::Command;

        // Map common voice targets to actual applications/URLs
        let (app_or_url, is_url) = match target.trim() {
            "calendar" | "my calendar" => ("Calendar", false),
            "mail" | "email" | "my email" => ("Mail", false),
            "browser" | "web browser" | "safari" => ("Safari", false),
            "chrome" | "google chrome" => ("Google Chrome", false),
            "firefox" => ("Firefox", false),
            "notes" | "my notes" => ("Notes", false),
            "music" | "apple music" => ("Music", false),
            "spotify" => ("Spotify", false),
            "terminal" | "command line" => ("Terminal", false),
            "settings" | "preferences" | "system preferences" => ("System Preferences", false),
            "finder" | "files" => ("Finder", false),
            "messages" | "imessage" => ("Messages", false),
            "slack" => ("Slack", false),
            "discord" => ("Discord", false),
            "zoom" => ("zoom.us", false),
            "vscode" | "visual studio code" | "code" => ("Visual Studio Code", false),
            other if other.starts_with("http://") || other.starts_with("https://") => (other, true),
            other => (other, false),
        };

        tracing::info!("Opening: {} (is_url: {})", app_or_url, is_url);

        #[cfg(target_os = "macos")]
        {
            let mut cmd = Command::new("open");
            if is_url {
                cmd.arg(app_or_url);
            } else {
                cmd.arg("-a").arg(app_or_url);
            }

            cmd.spawn()
                .map_err(|e| format!("Failed to open '{}': {}", app_or_url, e))?;
        }

        #[cfg(target_os = "windows")]
        {
            Command::new("cmd")
                .args(["/C", "start", "", app_or_url])
                .spawn()
                .map_err(|e| format!("Failed to open '{}': {}", app_or_url, e))?;
        }

        #[cfg(target_os = "linux")]
        {
            Command::new("xdg-open")
                .arg(app_or_url)
                .spawn()
                .map_err(|e| format!("Failed to open '{}': {}", app_or_url, e))?;
        }

        Ok(())
    }

    /// Execute a search command by opening a web search in the default browser.
    async fn execute_search_command(&self, query: &str) -> Result<(), String> {
        use std::process::Command;

        // Simple URL encoding for search query
        let encoded_query: String = query
            .chars()
            .map(|c| match c {
                'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
                ' ' => "+".to_string(),
                _ => format!("%{:02X}", c as u8),
            })
            .collect();
        let search_url = format!("https://www.google.com/search?q={}", encoded_query);

        tracing::info!("Searching for: {}", query);

        #[cfg(target_os = "macos")]
        {
            Command::new("open")
                .arg(&search_url)
                .spawn()
                .map_err(|e| format!("Failed to open search: {}", e))?;
        }

        #[cfg(target_os = "windows")]
        {
            Command::new("cmd")
                .args(["/C", "start", "", &search_url])
                .spawn()
                .map_err(|e| format!("Failed to open search: {}", e))?;
        }

        #[cfg(target_os = "linux")]
        {
            Command::new("xdg-open")
                .arg(&search_url)
                .spawn()
                .map_err(|e| format!("Failed to open search: {}", e))?;
        }

        Ok(())
    }

    /// Execute a shell command using the ShellTools utility.
    async fn execute_shell_command(&self, command: &str) -> Result<(), String> {
        use gestura_core::tools::shell::ShellTools;

        tracing::info!("Running shell command: {}", command);

        let shell = ShellTools::new();
        let result = shell
            .run(command, Some(30))
            .map_err(|e| format!("Shell command failed: {}", e))?;

        if !result.success {
            tracing::warn!(
                "Shell command exited with code {}: {}",
                result.exit_code,
                result.stderr
            );
        }

        Ok(())
    }

    /// Execute volume control command (increase or decrease by 10%).
    async fn execute_volume_command(&self, increase: bool) -> Result<(), String> {
        use std::process::Command;

        tracing::info!("Adjusting volume: {}", if increase { "up" } else { "down" });

        #[cfg(target_os = "macos")]
        {
            // AppleScript to adjust volume by 10%
            let script = if increase {
                "set volume output volume ((output volume of (get volume settings)) + 10)"
            } else {
                "set volume output volume ((output volume of (get volume settings)) - 10)"
            };

            Command::new("osascript")
                .args(["-e", script])
                .spawn()
                .map_err(|e| format!("Failed to adjust volume: {}", e))?;
        }

        #[cfg(target_os = "windows")]
        {
            // Use nircmd or PowerShell for volume control
            let adjustment = if increase { "+10" } else { "-10" };
            Command::new("powershell")
                .args([
                    "-Command",
                    "$obj = New-Object -ComObject WScript.Shell; $obj.SendKeys([char]0xAF)",
                ])
                .spawn()
                .map_err(|e| format!("Failed to adjust volume: {}", e))?;
            let _ = adjustment; // Suppress unused warning
        }

        #[cfg(target_os = "linux")]
        {
            let adjustment = if increase { "5%+" } else { "5%-" };
            Command::new("amixer")
                .args(["set", "Master", adjustment])
                .spawn()
                .map_err(|e| format!("Failed to adjust volume: {}", e))?;
        }

        Ok(())
    }

    /// Execute mute command to toggle audio mute state.
    async fn execute_mute_command(&self) -> Result<(), String> {
        use std::process::Command;

        tracing::info!("Toggling mute");

        #[cfg(target_os = "macos")]
        {
            Command::new("osascript")
                .args(["-e", "set volume with output muted"])
                .spawn()
                .map_err(|e| format!("Failed to mute: {}", e))?;
        }

        #[cfg(target_os = "windows")]
        {
            Command::new("powershell")
                .args([
                    "-Command",
                    "$obj = New-Object -ComObject WScript.Shell; $obj.SendKeys([char]0xAD)",
                ])
                .spawn()
                .map_err(|e| format!("Failed to mute: {}", e))?;
        }

        #[cfg(target_os = "linux")]
        {
            Command::new("amixer")
                .args(["set", "Master", "toggle"])
                .spawn()
                .map_err(|e| format!("Failed to mute: {}", e))?;
        }

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
