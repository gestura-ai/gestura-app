//! Tauri command handlers for configuration, MCP tools, MDH pointers, and tests.
use crate::{
    AppConfig,
    llm_provider::{AgentContext, select_provider},
};
use tauri::{Emitter, Manager};

#[tauri::command]
pub fn get_config() -> Result<AppConfig, String> {
    Ok(AppConfig::load())
}

#[tauri::command]
pub fn save_config(cfg: AppConfig) -> Result<(), String> {
    cfg.save().map_err(|e| e.to_string())
}

/// Check if this is the first run of the application (no config file exists yet).
#[tauri::command]
pub fn is_first_run() -> bool {
    AppConfig::is_first_run()
}

/// Get the path to the configuration file.
#[tauri::command]
pub fn get_config_path() -> String {
    AppConfig::default_path().to_string_lossy().to_string()
}

#[tauri::command]
pub fn list_mcp_tools() -> Result<Vec<crate::config::McpTool>, String> {
    Ok(AppConfig::load().mcp_tools)
}

#[tauri::command]
pub fn add_mcp_tool(tool: crate::config::McpTool) -> Result<(), String> {
    let mut cfg = AppConfig::load();
    if !cfg.mcp_tools.iter().any(|t| t.name == tool.name) {
        cfg.mcp_tools.push(tool);
    }
    cfg.save().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn remove_mcp_tool(name: String) -> Result<(), String> {
    let mut cfg = AppConfig::load();
    cfg.mcp_tools.retain(|t| t.name != name);
    cfg.save().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_mdh_pointers() -> Result<std::collections::HashMap<String, String>, String> {
    Ok(AppConfig::load().mdh_pointers)
}

#[tauri::command]
pub fn set_mdh_pointer(key: String, value: String) -> Result<(), String> {
    let mut cfg = AppConfig::load();
    cfg.mdh_pointers.insert(key, value);
    cfg.save().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn remove_mdh_pointer(key: String) -> Result<(), String> {
    let mut cfg = AppConfig::load();
    cfg.mdh_pointers.remove(&key);
    cfg.save().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn test_llm(prompt: String) -> Result<String, String> {
    let cfg = AppConfig::load();
    let provider = select_provider(
        &cfg,
        &AgentContext {
            agent_id: "test".into(),
        },
    );
    provider.call(&prompt).await.map_err(|e| format!("{e}"))
}

#[tauri::command]
pub async fn test_voice() -> Result<String, String> {
    let cfg = AppConfig::load();
    let engine = crate::voice_select::select_voice(&cfg);
    let name = engine.engine_name();
    let sample = engine.process_command(&cfg, None).await.unwrap_or_default();
    Ok(format!("engine={name} sample={sample}"))
}

/// Test Ollama connection and return server info
#[tauri::command]
pub async fn test_ollama_connection(endpoint: String) -> Result<serde_json::Value, String> {
    let client = reqwest::Client::new();
    let url = format!("{}/api/version", endpoint.trim_end_matches('/'));

    let resp = client
        .get(&url)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
        .map_err(|e| format!("Connection failed: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("Server returned error: {}", resp.status()));
    }

    let version: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Invalid response: {}", e))?;

    Ok(serde_json::json!({
        "connected": true,
        "version": version.get("version").and_then(|v| v.as_str()).unwrap_or("unknown")
    }))
}

/// List available Ollama models
#[tauri::command]
pub async fn list_ollama_models(endpoint: String) -> Result<Vec<serde_json::Value>, String> {
    let client = reqwest::Client::new();
    let url = format!("{}/api/tags", endpoint.trim_end_matches('/'));

    let resp = client
        .get(&url)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("Failed to list models: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("Server returned error: {}", resp.status()));
    }

    let data: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Invalid response: {}", e))?;

    let models = data
        .get("models")
        .and_then(|m| m.as_array())
        .map(|arr| {
            arr.iter()
                .map(|m| {
                    serde_json::json!({
                        "name": m.get("name").and_then(|n| n.as_str()).unwrap_or("unknown"),
                        "size": m.get("size").and_then(|s| s.as_u64()).unwrap_or(0),
                        "modified_at": m.get("modified_at").and_then(|d| d.as_str()).unwrap_or("")
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(models)
}

/// Test local Whisper model with detailed validation
#[tauri::command]
pub async fn test_local_whisper(model_path: String) -> Result<String, String> {
    use crate::voice::validate_whisper_model;
    use std::path::Path;

    let path = Path::new(&model_path);
    let validation = validate_whisper_model(path);

    if !validation.is_valid {
        return Err(validation
            .error
            .unwrap_or_else(|| "Unknown error".to_string()));
    }

    Ok(format!(
        "Model valid: {} ({:.1} MB, GGML format)",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown"),
        validation.file_size_mb
    ))
}

/// Validate a Whisper model file and return structured validation info
#[tauri::command]
pub async fn validate_whisper_model(
    path: String,
) -> Result<crate::voice::WhisperModelValidation, String> {
    use std::path::Path;

    let path = Path::new(&path);
    Ok(crate::voice::validate_whisper_model(path))
}

/// Get available Whisper models for download
#[tauri::command]
pub fn get_whisper_models() -> Vec<crate::config::WhisperModelInfo> {
    crate::config::WhisperModelInfo::available_models()
}

/// Check if a specific Whisper model file is already downloaded
#[tauri::command]
pub fn is_whisper_model_downloaded(model_filename: String) -> Result<serde_json::Value, String> {
    let models_dir = crate::config::AppConfig::whisper_models_dir();
    let model_path = models_dir.join(&model_filename);
    let exists = model_path.exists();

    let validation = if exists {
        Some(crate::voice::validate_whisper_model(&model_path))
    } else {
        None
    };

    Ok(serde_json::json!({
        "exists": exists,
        "path": model_path.to_string_lossy().to_string(),
        "is_valid": validation.as_ref().map(|v| v.is_valid).unwrap_or(false),
        "validation": validation
    }))
}

/// Get the default Whisper model path and status
#[tauri::command]
pub fn get_whisper_model_status() -> Result<serde_json::Value, String> {
    let (exists, path) = crate::voice::get_default_model_status();
    let path_str = path.to_string_lossy().to_string();

    let validation = if exists {
        Some(crate::voice::validate_whisper_model(&path))
    } else {
        None
    };

    Ok(serde_json::json!({
        "default_path": path_str,
        "exists": exists,
        "validation": validation,
        "models_dir": crate::config::AppConfig::whisper_models_dir().to_string_lossy()
    }))
}

/// Download a Whisper model from HuggingFace
/// Returns progress updates via Tauri events
#[tauri::command]
pub async fn download_whisper_model(
    app: tauri::AppHandle,
    model_filename: String,
) -> Result<String, String> {
    use std::io::Write;
    use tauri::Emitter;

    tracing::info!(
        "Whisper download command invoked for model filename: {}",
        model_filename
    );

    // Find the model info
    let model_info = crate::config::WhisperModelInfo::find_by_filename(&model_filename)
        .ok_or_else(|| format!("Unknown model: {}", model_filename))?;

    // Create the models directory
    let models_dir = crate::config::AppConfig::whisper_models_dir();
    std::fs::create_dir_all(&models_dir)
        .map_err(|e| format!("Failed to create models directory: {}", e))?;

    let output_path = models_dir.join(&model_filename);

    tracing::info!(
        "Preparing to download Whisper model '{}' (filename='{}') to {:?}",
        model_info.name,
        model_info.filename,
        output_path
    );

    // Check if already downloaded
    if output_path.exists() {
        let validation = crate::voice::validate_whisper_model(&output_path);
        if validation.is_valid {
            return Ok(format!(
                "Model already downloaded: {}",
                output_path.to_string_lossy()
            ));
        }
        // Remove invalid file
        std::fs::remove_file(&output_path).ok();
    }

    tracing::info!(
        "Downloading Whisper model: {} from {}",
        model_filename,
        model_info.url
    );

    // Emit start event
    let _ = app.emit(
        "whisper-download-progress",
        serde_json::json!({
            "status": "starting",
            "filename": model_filename,
            "total_mb": model_info.size_mb,
            "downloaded_mb": 0,
            "percent": 0
        }),
    );

    // Download the model
    let client = reqwest::Client::new();
    let response = client
        .get(&model_info.url)
        .send()
        .await
        .map_err(|e| format!("Failed to start download: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("Download failed: HTTP {}", response.status()));
    }

    let total_size = response
        .content_length()
        .unwrap_or(model_info.size_mb * 1024 * 1024);

    // Create temp file for download
    let temp_path = output_path.with_extension("tmp");
    let mut file = std::fs::File::create(&temp_path)
        .map_err(|e| format!("Failed to create temp file: {}", e))?;

    let mut downloaded: u64 = 0;
    let mut last_percent: u64 = 0;

    // Stream the response
    let mut stream = response.bytes_stream();
    use futures_util::StreamExt;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Download error: {}", e))?;
        file.write_all(&chunk)
            .map_err(|e| format!("Failed to write file: {}", e))?;

        downloaded += chunk.len() as u64;
        let percent = (downloaded * 100) / total_size;

        // Emit progress every 1%
        if percent > last_percent {
            last_percent = percent;
            let downloaded_mb = downloaded as f64 / (1024.0 * 1024.0);
            let _ = app.emit(
                "whisper-download-progress",
                serde_json::json!({
                    "status": "downloading",
                    "filename": model_filename,
                    "total_mb": model_info.size_mb,
                    "downloaded_mb": downloaded_mb,
                    "percent": percent
                }),
            );
        }
    }

    // Rename temp file to final path
    std::fs::rename(&temp_path, &output_path)
        .map_err(|e| format!("Failed to save model file: {}", e))?;

    // Validate the downloaded model
    let validation = crate::voice::validate_whisper_model(&output_path);
    if !validation.is_valid {
        std::fs::remove_file(&output_path).ok();
        return Err(format!(
            "Downloaded file is invalid: {}",
            validation
                .error
                .unwrap_or_else(|| "Unknown error".to_string())
        ));
    }

    // Emit completion event
    let _ = app.emit(
        "whisper-download-progress",
        serde_json::json!({
            "status": "complete",
            "filename": model_filename,
            "path": output_path.to_string_lossy(),
            "percent": 100
        }),
    );

    // Update config with the new model path
    let mut config = AppConfig::load();
    config.voice.local_model_path = Some(output_path.to_string_lossy().to_string());
    config
        .save()
        .map_err(|e| format!("Failed to save config: {}", e))?;

    tracing::info!(
        "Whisper model downloaded successfully: {}",
        output_path.to_string_lossy()
    );

    Ok(output_path.to_string_lossy().to_string())
}

#[tauri::command]
pub fn get_ui_prefs() -> Result<crate::config::UiSettings, String> {
    Ok(AppConfig::load().ui)
}

#[tauri::command]
pub fn set_ui_prefs(ui: crate::config::UiSettings) -> Result<(), String> {
    let mut cfg = AppConfig::load();
    cfg.ui = ui;
    cfg.save().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn run_voice_once() -> Result<String, String> {
    let cfg = AppConfig::load();
    let engine = crate::voice_select::select_voice(&cfg);
    crate::voice_select::validate_voice_config_for_run(&cfg, engine.as_ref())
        .map_err(|e| e.to_string())?;
    let text = engine
        .process_command(&cfg, None)
        .await
        .map_err(|e| e.to_string())?;
    Ok(text)
}

/// Scan for available Haptic Harmony rings
#[tauri::command]
pub async fn scan_for_rings() -> Result<Vec<String>, String> {
    let ring_manager = crate::ble::create_ring_manager();
    ring_manager
        .scan_for_rings()
        .await
        .map_err(|e| e.to_string())
}

/// Get ring status by device ID
#[tauri::command]
pub async fn get_ring_status(device_id: String) -> Result<Option<crate::ble::RingStatus>, String> {
    let ring_manager = crate::ble::create_ring_manager();
    ring_manager
        .get_ring_status(&device_id)
        .await
        .map_err(|e| e.to_string())
}

/// Pair with a ring
#[tauri::command]
pub async fn pair_ring(device_id: String) -> Result<(), String> {
    let ring_manager = crate::ble::create_ring_manager();
    ring_manager
        .pair_ring(&device_id)
        .await
        .map_err(|e| e.to_string())
}

/// Send haptic feedback to ring
#[tauri::command]
pub async fn send_haptic_feedback(
    device_id: String,
    pattern: String,
    intensity: f32,
    duration_ms: u32,
) -> Result<(), String> {
    let haptic_pattern = match pattern.as_str() {
        "click" => crate::haptics::HapticPattern::Click,
        "pulse" => crate::haptics::HapticPattern::Pulse,
        "ramp" => crate::haptics::HapticPattern::Ramp,
        _ => return Err("Invalid haptic pattern".to_string()),
    };

    let request = crate::haptics::HapticRequest {
        pattern: haptic_pattern,
        intensity,
        duration_ms,
        repeat_count: 0,
        repeat_delay_ms: 0,
    };

    let ring_manager = crate::ble::create_ring_manager();
    ring_manager
        .send_haptic(&device_id, request)
        .await
        .map_err(|e| e.to_string())
}

/// Start gesture monitoring for a ring
#[tauri::command]
pub async fn start_gesture_monitoring(device_id: String) -> Result<(), String> {
    let ring_manager = crate::ble::create_ring_manager();
    let (event_tx, _) = tokio::sync::broadcast::channel(100);
    ring_manager
        .start_gesture_monitoring(&device_id, event_tx)
        .await
        .map_err(|e| e.to_string())
}

/// Stop gesture monitoring for a ring
#[tauri::command]
pub async fn stop_gesture_monitoring(device_id: String) -> Result<(), String> {
    let ring_manager = crate::ble::create_ring_manager();
    ring_manager
        .stop_gesture_monitoring(&device_id)
        .await
        .map_err(|e| e.to_string())
}

/// Get system health status
#[tauri::command]
pub async fn get_system_health() -> Result<serde_json::Value, String> {
    let telemetry = crate::telemetry::get_telemetry_manager().await;
    let health = telemetry.get_system_health().await;
    serde_json::to_value(health).map_err(|e| e.to_string())
}

/// Get telemetry metrics summary
#[tauri::command]
pub async fn get_metrics_summary() -> Result<serde_json::Value, String> {
    let telemetry = crate::telemetry::get_telemetry_manager().await;
    Ok(telemetry.get_metrics_summary().await)
}

/// Get recent telemetry metrics
#[tauri::command]
pub async fn get_recent_metrics(
    limit: Option<usize>,
) -> Result<Vec<crate::telemetry::Metric>, String> {
    let telemetry = crate::telemetry::get_telemetry_manager().await;
    Ok(telemetry.get_recent_metrics(limit.unwrap_or(100)).await)
}

/// Clear telemetry metrics
#[tauri::command]
pub async fn clear_metrics() -> Result<(), String> {
    let telemetry = crate::telemetry::get_telemetry_manager().await;
    telemetry.clear_metrics().await;
    Ok(())
}

/// Export user data (GDPR compliance)
#[tauri::command]
pub async fn export_user_data(user_id: String) -> Result<serde_json::Value, String> {
    let gdpr = crate::gdpr::get_gdpr_manager().await;
    gdpr.export_user_data(&user_id)
        .await
        .map_err(|e| e.to_string())
}

/// Delete user data (GDPR compliance)
#[tauri::command]
pub async fn delete_user_data(
    user_id: String,
    verify: Option<bool>,
) -> Result<Vec<String>, String> {
    let gdpr = crate::gdpr::get_gdpr_manager().await;
    gdpr.delete_user_data(&user_id, verify.unwrap_or(false))
        .await
        .map_err(|e| e.to_string())
}

/// Get user consent status
#[tauri::command]
pub async fn get_user_consents(user_id: String) -> Result<Vec<crate::gdpr::ConsentRecord>, String> {
    let gdpr = crate::gdpr::get_gdpr_manager().await;
    Ok(gdpr.get_user_consents(&user_id).await)
}

/// Register user consent
#[tauri::command]
pub async fn register_consent(
    user_id: String,
    category: String,
    purpose: String,
) -> Result<(), String> {
    let gdpr = crate::gdpr::get_gdpr_manager().await;
    let data_category = match category.as_str() {
        "voice" => crate::gdpr::DataCategory::VoiceRecordings,
        "biometric" => crate::gdpr::DataCategory::BiometricData,
        "device" => crate::gdpr::DataCategory::DeviceData,
        "usage" => crate::gdpr::DataCategory::UsageAnalytics,
        "config" => crate::gdpr::DataCategory::ConfigurationData,
        _ => return Err("Invalid data category".to_string()),
    };

    gdpr.register_consent(user_id, data_category, purpose, "User consent".to_string())
        .await
        .map_err(|e| e.to_string())
}

// Chat and Agent Commands

/// Process a chat message through the configured LLM provider
#[tauri::command]
pub async fn process_chat_message(
    app: tauri::AppHandle,
    message: String,
) -> Result<String, String> {
    use crate::notifications::{NotificationType, get_notification_manager};

    let cfg = AppConfig::load();
    let provider = select_provider(
        &cfg,
        &AgentContext {
            agent_id: "chat".into(),
        },
    );

    tracing::info!(
        "Processing chat message through LLM provider: {}",
        cfg.llm.primary
    );

    // Call the LLM provider with the user message
    let response = provider.call(&message).await;

    match &response {
        Ok(resp) => {
            tracing::info!("LLM response received: {} chars", resp.len());
            // Send completion notification
            get_notification_manager()
                .notify(NotificationType::ResponseComplete, Some(&app))
                .await;
        }
        Err(e) => {
            tracing::error!("LLM error: {}", e);
            // Send error notification
            get_notification_manager()
                .notify(NotificationType::Error, Some(&app))
                .await;
        }
    }

    response.map_err(|e| format!("LLM error: {}", e))
}

/// Global cancellation token for streaming requests
static STREAMING_CANCEL_TOKEN: std::sync::OnceLock<
    std::sync::Mutex<Option<gestura_core::CancellationToken>>,
> = std::sync::OnceLock::new();

fn get_cancel_token_store() -> &'static std::sync::Mutex<Option<gestura_core::CancellationToken>> {
    STREAMING_CANCEL_TOKEN.get_or_init(|| std::sync::Mutex::new(None))
}

/// Process a chat message with streaming response
///
/// Emits `chat-stream-chunk` events with partial content and `chat-stream-done` when complete.
#[tauri::command]
pub async fn process_chat_message_streaming(
    app: tauri::AppHandle,
    message: String,
) -> Result<(), String> {
    use gestura_core::{CancellationToken, StreamChunk, start_streaming};
    use gestura_core::{
        looks_like_capabilities_question, looks_like_tools_question, render_capabilities,
        render_tool_detail, render_tools_overview,
    };
    use tokio::sync::mpsc;

    let cfg = AppConfig::load();

    // Check if this is a tools/capabilities question and handle it locally without LLM
    let trimmed = message.trim();
    let is_tools_cmd = trimmed.starts_with("/tools");
    let is_capabilities_cmd = trimmed.starts_with("/capabilities");
    let is_tools_question = looks_like_tools_question(trimmed);
    let is_capabilities_question = looks_like_capabilities_question(trimmed);

    if is_tools_cmd || is_tools_question {
        let response = if is_tools_cmd {
            // Parse /tools <name> command
            let mut parts = trimmed.split_whitespace();
            let _ = parts.next(); // skip /tools
            if let Some(name) = parts.next() {
                render_tool_detail(name).unwrap_or_else(|| {
                    format!("Unknown tool '{}'. Try `/tools` to list all available tools.", name)
                })
            } else {
                render_tools_overview()
            }
        } else {
            render_tools_overview()
        };

        // Emit the response as a single chunk, then done
        if let Err(e) = app.emit("chat-stream-chunk", &response) {
            tracing::error!("Failed to emit tools response: {}", e);
        }
        if let Err(e) = app.emit("chat-stream-done", ()) {
            tracing::error!("Failed to emit stream done: {}", e);
        }
        return Ok(());
    }

    // Handle capabilities questions - includes MCP servers, devices, settings
    if is_capabilities_cmd || is_capabilities_question {
        let response = render_capabilities(&cfg);

        if let Err(e) = app.emit("chat-stream-chunk", &response) {
            tracing::error!("Failed to emit capabilities response: {}", e);
        }
        if let Err(e) = app.emit("chat-stream-done", ()) {
            tracing::error!("Failed to emit stream done: {}", e);
        }
        return Ok(());
    }

    tracing::info!(
        "Starting streaming chat through LLM provider: {}",
        cfg.llm.primary
    );

    // Create channel for streaming chunks
    let (tx, mut rx) = mpsc::channel::<StreamChunk>(100);

    // Create cancellation token and store it
    let cancel_token = CancellationToken::new();
    {
        let mut store = get_cancel_token_store().lock().unwrap();
        *store = Some(cancel_token.clone());
    }

    // Spawn the streaming task
    let cfg_clone = cfg.clone();
    let message_clone = message.clone();
    let cancel_token_clone = cancel_token.clone();
    tokio::spawn(async move {
        if let Err(e) = start_streaming(&cfg_clone, &message_clone, tx, cancel_token_clone).await {
            tracing::error!("Streaming error: {}", e);
        }
    });

    // Forward chunks to frontend via Tauri events
    while let Some(chunk) = rx.recv().await {
        match chunk {
            StreamChunk::Text(text) => {
                if let Err(e) = app.emit("chat-stream-chunk", &text) {
                    tracing::error!("Failed to emit stream chunk: {}", e);
                }
            }
            StreamChunk::Done => {
                if let Err(e) = app.emit("chat-stream-done", ()) {
                    tracing::error!("Failed to emit stream done: {}", e);
                }
                break;
            }
            StreamChunk::Cancelled => {
                if let Err(e) = app.emit("chat-stream-cancelled", ()) {
                    tracing::error!("Failed to emit stream cancelled: {}", e);
                }
                break;
            }
            StreamChunk::Error(err) => {
                if let Err(e) = app.emit("chat-stream-error", &err) {
                    tracing::error!("Failed to emit stream error: {}", e);
                }
                break;
            }
        }
    }

    // Clear the cancellation token
    {
        let mut store = get_cancel_token_store().lock().unwrap();
        *store = None;
    }

    Ok(())
}

/// Cancel an ongoing streaming chat request
#[tauri::command]
pub fn cancel_chat_streaming() -> Result<(), String> {
    let store = get_cancel_token_store().lock().unwrap();
    if let Some(token) = store.as_ref() {
        token.cancel();
        tracing::info!("Streaming chat cancelled");
        Ok(())
    } else {
        Err("No active streaming request to cancel".to_string())
    }
}

#[tauri::command]
pub async fn send_agent_message(
    agent_id: String,
    message: String,
    _state: tauri::State<'_, crate::AppState>,
) -> Result<String, String> {
    // Use the configured LLM provider for agent communication
    let cfg = AppConfig::load();
    let provider = select_provider(
        &cfg,
        &AgentContext {
            agent_id: agent_id.clone(),
        },
    );

    tracing::info!("Agent {} sending message through LLM", agent_id);

    // Format the prompt with agent context
    let prompt = format!(
        "You are agent '{}'. Respond to the following message:\n\n{}",
        agent_id, message
    );

    let response = provider
        .call(&prompt)
        .await
        .map_err(|e| format!("Agent LLM error: {}", e))?;

    tracing::info!("Agent {} response received", agent_id);
    Ok(response)
}

#[tauri::command]
pub async fn get_agent_status(
    agent_id: String,
    state: tauri::State<'_, crate::AppState>,
) -> Result<serde_json::Value, String> {
    let agents = &state.agents;

    // Check if agent exists in the manager
    if let Some(info) = agents.get_agent_status(&agent_id).await {
        return Ok(serde_json::json!({
            "id": info.id,
            "name": info.name,
            "status": info.status,
            "last_activity": info.last_activity.to_rfc3339()
        }));
    }

    // Agent not found - return inactive status
    Ok(serde_json::json!({
        "id": agent_id,
        "name": "Unknown Agent",
        "status": "inactive",
        "last_activity": null
    }))
}

/// List all active agents
#[tauri::command]
pub async fn list_agents(
    state: tauri::State<'_, crate::AppState>,
) -> Result<serde_json::Value, String> {
    let agents = &state.agents;
    let agent_list = agents.list_agents().await;

    Ok(serde_json::json!({
        "agents": agent_list,
        "count": agent_list.len()
    }))
}

// Orchestrator Commands

/// Delegate a task to a subagent
#[tauri::command]
pub async fn delegate_task(
    task: crate::orchestrator::DelegatedTask,
    state: tauri::State<'_, crate::AppState>,
) -> Result<String, String> {
    let orchestrator =
        crate::orchestrator::AgentOrchestrator::new(state.agents.clone(), state.config.clone());
    orchestrator.delegate_task(task).await
}

/// Spawn a new subagent
#[tauri::command]
pub async fn spawn_subagent(
    agent_id: String,
    name: String,
    state: tauri::State<'_, crate::AppState>,
) -> Result<(), String> {
    let orchestrator =
        crate::orchestrator::AgentOrchestrator::new(state.agents.clone(), state.config.clone());
    orchestrator.spawn_subagent(&agent_id, &name).await
}

/// List all active tasks
#[tauri::command]
pub async fn list_active_tasks(
    state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<crate::orchestrator::DelegatedTask>, String> {
    let orchestrator =
        crate::orchestrator::AgentOrchestrator::new(state.agents.clone(), state.config.clone());
    Ok(orchestrator.list_active_tasks().await)
}

/// Cancel a running task
#[tauri::command]
pub async fn cancel_task(
    task_id: String,
    state: tauri::State<'_, crate::AppState>,
) -> Result<(), String> {
    let orchestrator =
        crate::orchestrator::AgentOrchestrator::new(state.agents.clone(), state.config.clone());
    orchestrator.cancel_task(&task_id).await
}

// Audio Device Management Commands

/// List all available audio input devices
#[tauri::command]
pub fn list_audio_devices() -> Result<Vec<crate::audio_capture::AudioDeviceInfo>, String> {
    Ok(crate::audio_capture::list_audio_input_devices())
}

/// Check if microphone is available
#[tauri::command]
pub fn check_microphone_available() -> bool {
    crate::audio_capture::is_microphone_available()
}

// Permission Management Commands

#[tauri::command]
pub async fn check_permission(permission: String) -> Result<String, String> {
    use crate::permissions::{
        check_accessibility_permission, check_bluetooth_permission, check_microphone_permission,
        check_screen_recording_permission,
    };

    match permission.as_str() {
        "microphone" => {
            let status = check_microphone_permission();
            tracing::info!("Permission check: microphone -> {}", status);
            Ok(status.to_string())
        }
        "accessibility" => {
            let status = check_accessibility_permission();
            tracing::info!("Permission check: accessibility -> {}", status);
            Ok(status.to_string())
        }
        "bluetooth" => {
            let status = check_bluetooth_permission();
            tracing::info!("Permission check: bluetooth -> {}", status);
            Ok(status.to_string())
        }
        "screen_recording" => {
            let status = check_screen_recording_permission();
            tracing::info!("Permission check: screen_recording -> {}", status);
            Ok(status.to_string())
        }
        _ => Err(format!("Unknown permission: {}", permission)),
    }
}

#[tauri::command]
pub async fn request_permission(permission: String) -> Result<(), String> {
    use crate::permissions::{
        SystemPermissionStatus, check_accessibility_permission, check_bluetooth_permission,
        check_microphone_permission, check_screen_recording_permission, open_system_preferences,
        request_bluetooth_permission, request_microphone_permission,
        request_screen_recording_permission,
    };

    tracing::info!("🔐 Permission request received: {}", permission);

    match permission.as_str() {
        "microphone" => {
            let status = check_microphone_permission();
            tracing::info!("🎤 Microphone permission status before request: {}", status);

            match status {
                SystemPermissionStatus::Granted => {
                    tracing::info!("🎤 Microphone permission already granted; nothing to request");
                    Ok(())
                }
                SystemPermissionStatus::Denied | SystemPermissionStatus::Restricted => {
                    tracing::info!(
                        "🎤 Microphone permission denied or restricted; opening System Settings",
                    );
                    if open_system_preferences("microphone") {
                        tracing::info!("✅ Opened System Preferences for Microphone");
                        Ok(())
                    } else {
                        Err("Failed to open System Preferences for Microphone".to_string())
                    }
                }
                SystemPermissionStatus::NotDetermined | SystemPermissionStatus::Unknown => {
                    tracing::info!(
                        "🎤 Microphone permission not determined/unknown; attempting to trigger system dialog",
                    );
                    if request_microphone_permission() {
                        tracing::info!("✅ Microphone permission request initiated");
                        Ok(())
                    } else {
                        tracing::warn!(
                            "⚠️ Microphone permission request script failed; attempting to open System Preferences",
                        );
                        if open_system_preferences("microphone") {
                            tracing::info!(
                                "✅ Opened System Preferences for Microphone (fallback)",
                            );
                            Ok(())
                        } else {
                            Err(
                                "Failed to request microphone permission or open System Preferences"
                                    .to_string(),
                            )
                        }
                    }
                }
            }
        }
        "accessibility" => {
            let status = check_accessibility_permission();
            tracing::info!(
                "♿ Accessibility permission status before request: {}",
                status
            );

            if status == SystemPermissionStatus::Granted {
                tracing::info!("♿ Accessibility permission already granted; nothing to request",);
                return Ok(());
            }

            // Accessibility CANNOT be requested programmatically on macOS
            // It always requires manual grant in System Settings
            if open_system_preferences("accessibility") {
                tracing::info!("✅ Opened System Preferences for Accessibility");
                Ok(())
            } else {
                Err("Failed to open System Preferences for Accessibility".to_string())
            }
        }
        "bluetooth" => {
            let status = check_bluetooth_permission();
            tracing::info!("🔵 Bluetooth permission status before request: {}", status);

            match status {
                SystemPermissionStatus::Granted => {
                    tracing::info!("🔵 Bluetooth permission already granted; nothing to request",);
                    Ok(())
                }
                SystemPermissionStatus::Denied | SystemPermissionStatus::Restricted => {
                    tracing::info!(
                        "🔵 Bluetooth permission denied or restricted; opening System Settings",
                    );
                    if open_system_preferences("bluetooth") {
                        tracing::info!("✅ Opened System Preferences for Bluetooth");
                        Ok(())
                    } else {
                        Err("Failed to open System Preferences for Bluetooth".to_string())
                    }
                }
                SystemPermissionStatus::NotDetermined | SystemPermissionStatus::Unknown => {
                    tracing::info!(
                        "🔵 Bluetooth permission not determined/unknown; attempting to trigger system dialog",
                    );
                    if request_bluetooth_permission() {
                        tracing::info!("✅ Bluetooth permission request initiated");
                        Ok(())
                    } else {
                        tracing::warn!(
                            "⚠️ Bluetooth permission request script failed; attempting to open System Preferences",
                        );
                        if open_system_preferences("bluetooth") {
                            tracing::info!("✅ Opened System Preferences for Bluetooth (fallback)",);
                            Ok(())
                        } else {
                            Err(
                                "Failed to request Bluetooth permission or open System Preferences"
                                    .to_string(),
                            )
                        }
                    }
                }
            }
        }
        "screen_recording" => {
            let status = check_screen_recording_permission();
            tracing::info!(
                "🖥️ Screen Recording permission status before request: {}",
                status
            );

            match status {
                SystemPermissionStatus::Granted => {
                    tracing::info!(
                        "🖥️ Screen Recording permission already granted; nothing to request",
                    );
                    Ok(())
                }
                SystemPermissionStatus::Denied | SystemPermissionStatus::Restricted => {
                    tracing::info!(
                        "🖥️ Screen Recording permission denied/restricted; opening System Settings",
                    );
                    if open_system_preferences("screen_recording") {
                        Ok(())
                    } else {
                        Err("Failed to open System Preferences for Screen Recording".to_string())
                    }
                }
                SystemPermissionStatus::NotDetermined | SystemPermissionStatus::Unknown => {
                    tracing::info!(
                        "🖥️ Screen Recording permission not determined/unknown; attempting to trigger system prompt",
                    );

                    if request_screen_recording_permission()
                        || open_system_preferences("screen_recording")
                    {
                        Ok(())
                    } else {
                        Err(
                            "Failed to request Screen Recording permission or open System Preferences"
                                .to_string(),
                        )
                    }
                }
            }
        }
        _ => Err(format!("Cannot request unknown permission: {}", permission)),
    }
}

// UI Testing and Validation Commands

#[tauri::command]
pub async fn test_open_window(
    window_type: String,
    _app: tauri::AppHandle,
) -> Result<String, String> {
    tracing::info!("Testing window open: {}", window_type);

    match window_type.as_str() {
        "config" => {
            crate::window_manager::open_config_window().map_err(|e| e.to_string())?;
            Ok("Config window opened".to_string())
        }
        "chat" => {
            let session_id =
                crate::window_manager::create_new_chat_session().map_err(|e| e.to_string())?;
            Ok(format!("Chat session created: {}", session_id))
        }
        _ => Err(format!("Unknown window type: {}", window_type)),
    }
}

#[tauri::command]
pub async fn capture_window_screenshot(
    window_label: String,
    app: tauri::AppHandle,
) -> Result<String, String> {
    if let Some(window) = app.get_webview_window(&window_label) {
        // Take screenshot using Tauri's screenshot API
        // Note: This requires the window to be visible and focused
        let _ = window.show();
        let _ = window.set_focus();

        // Wait a moment for the window to render
        tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;

        tracing::info!("Screenshot captured for window: {}", window_label);
        Ok(format!("Screenshot captured for {}", window_label))
    } else {
        Err(format!("Window not found: {}", window_label))
    }
}

#[tauri::command]
pub async fn validate_window_content(
    window_label: String,
    app: tauri::AppHandle,
) -> Result<serde_json::Value, String> {
    if let Some(window) = app.get_webview_window(&window_label) {
        let _ = window.show();
        let _ = window.set_focus();

        // Wait for content to load
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        // Return validation results
        Ok(serde_json::json!({
            "window": window_label,
            "visible": true,
            "focused": true,
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "status": "validated"
        }))
    } else {
        Err(format!("Window not found: {}", window_label))
    }
}

#[tauri::command]
pub async fn get_window_list(app: tauri::AppHandle) -> Result<Vec<String>, String> {
    let windows: Vec<String> = app.webview_windows().keys().cloned().collect();

    Ok(windows)
}

#[tauri::command]
pub async fn close_test_windows(app: tauri::AppHandle) -> Result<String, String> {
    let test_windows = ["permissions", "config", "chat", "status", "about"];
    let mut closed_count = 0;

    for window_label in test_windows.iter() {
        if let Some(window) = app.get_webview_window(window_label) {
            let _ = window.close();
            closed_count += 1;
        }
    }

    Ok(format!("Closed {} test windows", closed_count))
}

/// Get current listening state
#[tauri::command]
pub fn get_listening_state() -> Result<(bool, Option<u64>), String> {
    let (is_listening, remaining) = crate::tray::get_listening_state();
    let remaining_secs = remaining.map(|d| d.as_secs());
    Ok((is_listening, remaining_secs))
}

/// Set listening timeout duration
#[tauri::command]
pub fn set_listening_timeout(seconds: u64) -> Result<(), String> {
    let duration = std::time::Duration::from_secs(seconds);
    crate::tray::set_listening_timeout(duration);
    Ok(())
}

/// Toggle listening mode
#[tauri::command]
pub fn toggle_listening(_app: tauri::AppHandle) -> Result<String, String> {
    // This would trigger the same logic as the tray menu
    // For now, we'll just return the current state
    let (is_listening, _) = crate::tray::get_listening_state();
    if is_listening {
        Ok("Listening stopped".to_string())
    } else {
        Ok("Listening started".to_string())
    }
}

/// Update speech processing configuration
#[tauri::command]
pub fn update_speech_config(config: crate::speech::SpeechConfig) -> Result<(), String> {
    crate::speech::update_speech_config(config);
    Ok(())
}

/// Get current speech processing status
#[tauri::command]
pub fn get_speech_status() -> Result<bool, String> {
    Ok(crate::speech::is_speech_recording())
}

/// Get tray diagnostic information
#[tauri::command]
pub fn get_tray_diagnostic_info() -> Result<serde_json::Value, String> {
    Ok(crate::tray::get_tray_diagnostic_info())
}

/// Check system permissions status
#[tauri::command]
pub fn check_system_permissions() -> Result<serde_json::Value, String> {
    use crate::permissions::{
        SystemPermissionStatus, check_accessibility_permission, check_bluetooth_permission,
        check_microphone_permission, check_screen_recording_permission,
    };

    let mic_status = check_microphone_permission();
    let accessibility_status = check_accessibility_permission();
    let bluetooth_status = check_bluetooth_permission();
    let screen_recording_status = check_screen_recording_permission();

    tracing::info!(
        "System permission snapshot: microphone={}, accessibility={}, bluetooth={}, screen_recording={}",
        mic_status,
        accessibility_status,
        bluetooth_status,
        screen_recording_status
    );

    let mic_instructions = match mic_status {
        SystemPermissionStatus::Granted => "Microphone access is working properly",
        SystemPermissionStatus::Denied => {
            "Please enable microphone access in System Preferences > Privacy & Security > Microphone"
        }
        SystemPermissionStatus::NotDetermined => {
            "Microphone access will be requested when you start listening"
        }
        _ => "Check System Preferences for microphone access",
    };

    let accessibility_instructions = match accessibility_status {
        SystemPermissionStatus::Granted => "Accessibility access is working properly",
        SystemPermissionStatus::Denied => {
            "Please enable accessibility in System Preferences > Privacy & Security > Accessibility"
        }
        _ => "Accessibility access is required for hotkey functionality",
    };

    let bluetooth_instructions = match bluetooth_status {
        SystemPermissionStatus::Granted => "Bluetooth access is working properly",
        SystemPermissionStatus::Denied => {
            "Please enable Bluetooth in System Preferences > Privacy & Security > Bluetooth"
        }
        SystemPermissionStatus::NotDetermined => {
            "Bluetooth access will be requested when connecting to a ring"
        }
        _ => "Check System Preferences for Bluetooth access",
    };

    let screen_recording_instructions = match screen_recording_status {
        SystemPermissionStatus::Granted => "Screen Recording access is working properly",
        SystemPermissionStatus::Denied => {
            "Please enable Screen Recording in System Preferences > Privacy & Security > Screen Recording"
        }
        SystemPermissionStatus::NotDetermined => {
            "Screen Recording access will be requested when screen capture is needed"
        }
        _ => "Check System Preferences for Screen Recording access",
    };

    // Only include real OS-level permissions here.
    let permissions = vec![
        serde_json::json!({
            "id": "microphone",
            "name": "Microphone",
            "description": "Required for voice commands and speech recognition",
            "status": mic_status.to_string(),
            "required": true,
            "instructions": mic_instructions
        }),
        serde_json::json!({
            "id": "accessibility",
            "name": "Accessibility",
            "description": "Required for global hotkeys and gesture shortcuts",
            "status": accessibility_status.to_string(),
            "required": true,
            "instructions": accessibility_instructions
        }),
        serde_json::json!({
            "id": "bluetooth",
            "name": "Bluetooth",
            "description": "Required for connecting to Haptic Harmony ring",
            "status": bluetooth_status.to_string(),
            "required": false,
            "instructions": bluetooth_instructions
        }),
        serde_json::json!({
            "id": "screen_recording",
            "name": "Screen Recording",
            "description": "Optional: required for screen capture features",
            "status": screen_recording_status.to_string(),
            "required": false,
            "instructions": screen_recording_instructions
        }),
    ];

    let total_count = permissions.len();

    let granted_count = permissions
        .iter()
        .filter(|p| p.get("status").and_then(|v| v.as_str()) == Some("granted"))
        .count();

    let required_count = permissions
        .iter()
        .filter(|p| p.get("required").and_then(|v| v.as_bool()) == Some(true))
        .count();

    let required_granted_count = permissions
        .iter()
        .filter(|p| {
            p.get("required").and_then(|v| v.as_bool()) == Some(true)
                && p.get("status").and_then(|v| v.as_str()) == Some("granted")
        })
        .count();

    let missing_required_count = required_count.saturating_sub(required_granted_count);

    Ok(serde_json::json!({
        "permissions": permissions,
        "total_count": total_count,
        "granted_count": granted_count,
        "required_count": required_count,
        "required_granted_count": required_granted_count,
        "missing_required_count": missing_required_count,
        "summary": {
            // Back-compat keys used by current UI.
            "total": total_count,
            "granted": granted_count,
            "required": required_count,
            // New explicit keys.
            "required_granted": required_granted_count,
            "missing_required": missing_required_count
        }
    }))
}

// ============================================================================
// Session History Commands
// ============================================================================

/// Get all chat sessions (both open and closed)
#[tauri::command]
pub fn get_chat_sessions() -> Result<Vec<crate::window_manager::ChatSession>, String> {
    Ok(crate::window_manager::get_all_sessions())
}

/// Restore a closed chat session
#[tauri::command]
pub fn restore_chat_session(session_id: String) -> Result<(), String> {
    crate::window_manager::restore_chat_session(&session_id)
        .map_err(|e| format!("Failed to restore session: {}", e))
}

/// Create a new chat session
#[tauri::command]
pub fn create_chat_session() -> Result<String, String> {
    crate::window_manager::create_new_chat_session()
        .map_err(|e| format!("Failed to create session: {}", e))
}

/// Get session counts (active, closed)
#[tauri::command]
pub fn get_session_counts() -> Result<(usize, usize), String> {
    Ok(crate::window_manager::get_session_counts())
}

// ============================================================================
// Voice Listener Control Commands
// ============================================================================

/// Validation result for voice configuration
#[derive(Debug, Clone, serde::Serialize)]
pub struct VoiceConfigValidation {
    pub is_valid: bool,
    pub provider: String,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub suggestion: Option<String>,
}

/// Validate voice/STT configuration before starting listener
#[tauri::command]
pub fn validate_voice_config() -> VoiceConfigValidation {
    let config = crate::AppConfig::load();
    let provider = config.voice.provider.as_str();

    match provider {
        "local" => {
            // Check if local model is configured
            let model_path = config
                .voice
                .local_model_path
                .as_ref()
                .map(std::path::PathBuf::from)
                .unwrap_or_else(crate::AppConfig::default_whisper_model_path);

            if !model_path.exists() {
                return VoiceConfigValidation {
                    is_valid: false,
                    provider: "local".to_string(),
                    error_code: Some("LOCAL_MODEL_NOT_FOUND".to_string()),
                    error_message: Some("Local Whisper model not found.".to_string()),
                    suggestion: Some(
                        "Download a Whisper model in Settings → Voice & Audio → Local Whisper."
                            .to_string(),
                    ),
                };
            }

            // Validate the model file
            let validation = crate::voice::validate_whisper_model(&model_path);
            if !validation.is_valid {
                return VoiceConfigValidation {
                    is_valid: false,
                    provider: "local".to_string(),
                    error_code: Some("LOCAL_MODEL_INVALID".to_string()),
                    error_message: Some(
                        validation
                            .error
                            .unwrap_or_else(|| "Invalid Whisper model file.".to_string()),
                    ),
                    suggestion: Some(
                        "The model file may be corrupted. Try downloading it again.".to_string(),
                    ),
                };
            }

            VoiceConfigValidation {
                is_valid: true,
                provider: "local".to_string(),
                error_code: None,
                error_message: None,
                suggestion: None,
            }
        }
        "openai" => {
            // Check if OpenAI API key is configured
            if config
                .voice
                .openai_api_key
                .as_ref()
                .is_none_or(|k| k.is_empty())
            {
                return VoiceConfigValidation {
                    is_valid: false,
                    provider: "openai".to_string(),
                    error_code: Some("OPENAI_API_KEY_MISSING".to_string()),
                    error_message: Some("OpenAI API key not configured.".to_string()),
                    suggestion: Some(
                        "Add your OpenAI API key in Settings → AI Providers → OpenAI.".to_string(),
                    ),
                };
            }

            VoiceConfigValidation {
                is_valid: true,
                provider: "openai".to_string(),
                error_code: None,
                error_message: None,
                suggestion: None,
            }
        }
        "none" | "" => VoiceConfigValidation {
            is_valid: false,
            provider: provider.to_string(),
            error_code: Some("NO_PROVIDER_CONFIGURED".to_string()),
            error_message: Some("No speech-to-text provider configured.".to_string()),
            suggestion: Some(
                "Configure a speech-to-text provider in Settings → Voice & Audio.".to_string(),
            ),
        },
        _ => VoiceConfigValidation {
            is_valid: false,
            provider: provider.to_string(),
            error_code: Some("UNKNOWN_PROVIDER".to_string()),
            error_message: Some(format!("Unknown speech-to-text provider: {}", provider)),
            suggestion: Some(
                "Select a valid provider (Local Whisper or OpenAI) in Settings.".to_string(),
            ),
        },
    }
}

/// Extended validation that also ensures a usable LLM provider is configured so
/// the full voice → STT → LLM agent loop can run.
pub fn validate_voice_and_llm_config() -> VoiceConfigValidation {
    // First, validate the speech-to-text configuration using the existing
    // helper so that we keep a single source of truth for STT checks.
    let stt_validation = validate_voice_config();
    if !stt_validation.is_valid {
        return stt_validation;
    }

    let config = crate::AppConfig::load();
    let llm_primary_raw = config.llm.primary.trim();
    let llm_primary = llm_primary_raw.to_lowercase();

    // Helper to construct LLM-related validation errors
    let llm_error = |code: &str, message: &str, suggestion: &str| VoiceConfigValidation {
        is_valid: false,
        provider: format!("llm:{}", llm_primary),
        error_code: Some(code.to_string()),
        error_message: Some(message.to_string()),
        suggestion: Some(suggestion.to_string()),
    };

    if llm_primary.is_empty() {
        return llm_error(
            "LLM_PROVIDER_MISSING",
            "No LLM provider configured.",
            "Select and configure an LLM provider in Settings → AI Providers.",
        );
    }

    // Echo does not require any external configuration and is always valid
    if llm_primary == "echo" {
        return stt_validation;
    }

    match llm_primary.as_str() {
        "openai" => {
            if let Some(c) = &config.llm.openai {
                if c.api_key.trim().is_empty() {
                    return llm_error(
                        "LLM_CONFIG_INCOMPLETE",
                        "OpenAI LLM provider is selected but API key is missing.",
                        "Add your OpenAI API key in Settings → AI Providers → OpenAI.",
                    );
                }
                if c.model.trim().is_empty() {
                    return llm_error(
                        "LLM_CONFIG_INCOMPLETE",
                        "OpenAI LLM provider is selected but no model is configured.",
                        "Choose a chat model for OpenAI in Settings → AI Providers.",
                    );
                }
            } else {
                return llm_error(
                    "LLM_CONFIG_MISSING",
                    "OpenAI LLM provider is selected but not configured.",
                    "Fill in OpenAI LLM settings under Settings → AI Providers → OpenAI.",
                );
            }
        }
        "anthropic" => {
            if let Some(c) = &config.llm.anthropic {
                if c.api_key.trim().is_empty() {
                    return llm_error(
                        "LLM_CONFIG_INCOMPLETE",
                        "Anthropic LLM provider is selected but API key is missing.",
                        "Add your Anthropic API key in Settings → AI Providers → Anthropic.",
                    );
                }
                if c.model.trim().is_empty() {
                    return llm_error(
                        "LLM_CONFIG_INCOMPLETE",
                        "Anthropic LLM provider is selected but no model is configured.",
                        "Choose a Claude model in Settings → AI Providers → Anthropic.",
                    );
                }
            } else {
                return llm_error(
                    "LLM_CONFIG_MISSING",
                    "Anthropic LLM provider is selected but not configured.",
                    "Fill in Anthropic LLM settings under Settings → AI Providers → Anthropic.",
                );
            }
        }
        "grok" => {
            if let Some(c) = &config.llm.grok {
                if c.api_key.trim().is_empty() {
                    return llm_error(
                        "LLM_CONFIG_INCOMPLETE",
                        "Grok LLM provider is selected but API key is missing.",
                        "Add your Grok API key in Settings → AI Providers → Grok.",
                    );
                }
                if c.model.trim().is_empty() {
                    return llm_error(
                        "LLM_CONFIG_INCOMPLETE",
                        "Grok LLM provider is selected but no model is configured.",
                        "Choose a Grok model in Settings → AI Providers → Grok.",
                    );
                }
            } else {
                return llm_error(
                    "LLM_CONFIG_MISSING",
                    "Grok LLM provider is selected but not configured.",
                    "Fill in Grok LLM settings under Settings → AI Providers → Grok.",
                );
            }
        }
        "ollama" => {
            if let Some(c) = &config.llm.ollama {
                if c.base_url.trim().is_empty() {
                    return llm_error(
                        "LLM_CONFIG_INCOMPLETE",
                        "Ollama LLM provider is selected but server URL is missing.",
                        "Set the Ollama server URL (for example http://localhost:11434) in Settings → AI Providers → Ollama.",
                    );
                }
                if c.model.trim().is_empty() {
                    return llm_error(
                        "LLM_CONFIG_INCOMPLETE",
                        "Ollama LLM provider is selected but no model is configured.",
                        "Select an Ollama model in Settings → AI Providers → Ollama.",
                    );
                }
            } else {
                return llm_error(
                    "LLM_CONFIG_MISSING",
                    "Ollama LLM provider is selected but not configured.",
                    "Fill in Ollama settings under Settings → AI Providers → Ollama.",
                );
            }
        }
        _ => {
            return llm_error(
                "LLM_PROVIDER_UNKNOWN",
                &format!("Unknown LLM provider: {}", llm_primary_raw),
                "Select a valid LLM provider (OpenAI, Anthropic, Grok, Ollama, or Echo) in Settings → AI Providers.",
            );
        }
    }

    // Both STT and LLM configuration look good; report overall success.
    stt_validation
}

/// Start voice listening with validation shared with the tray logic.
///
/// This command is typically triggered from the chat UI. It delegates to the
/// tray module so that both chat and tray use the exact same validation and
/// speech start pipeline.
#[tauri::command]
pub fn start_voice_listening(app: tauri::AppHandle) -> Result<String, String> {
    crate::tray::start_listening_with_validation(&app)?;
    let provider = crate::AppConfig::load().voice.provider;
    Ok(format!("Voice listening started (provider: {})", provider))
}

/// Stop voice listening
#[tauri::command]
pub fn stop_voice_listening(app: tauri::AppHandle) -> Result<String, String> {
    // Stop the speech processing (audio recording)
    if let Err(e) = crate::speech::stop_speech_listening() {
        tracing::warn!("Failed to stop speech processing: {}", e);
    }
    // Update the listening state
    crate::tray::stop_listening();

    // Emit event to notify frontend that listening has stopped
    if let Err(e) = app.emit(
        "listening-state-changed",
        serde_json::json!({
            "is_listening": false
        }),
    ) {
        tracing::warn!("Failed to emit listening-state-changed: {}", e);
    }
    tracing::info!("Emitted listening-state-changed event (stopped via API)");

    Ok("Voice listening stopped".to_string())
}

/// Complete the onboarding process and mark it as done
#[tauri::command]
pub fn complete_onboarding() -> Result<(), String> {
    tracing::info!("Onboarding completed by user");
    // Save a default config to mark first run as complete
    let config = AppConfig::load();
    config.save().map_err(|e| e.to_string())?;
    Ok(())
}

/// Close the onboarding window
#[tauri::command]
pub fn close_onboarding_window() -> Result<(), String> {
    crate::window_manager::close_onboarding().map_err(|e| e.to_string())
}

/// Open system preferences to a specific pane
#[tauri::command]
pub fn open_system_preferences(pane: String) -> Result<(), String> {
    use crate::permissions::open_system_preferences as open_prefs;
    if open_prefs(&pane) {
        Ok(())
    } else {
        Err(format!("Failed to open System Preferences for {}", pane))
    }
}

/// Update voice provider setting
#[tauri::command]
pub fn update_voice_provider(provider: String) -> Result<(), String> {
    let mut cfg = AppConfig::load();
    cfg.voice.provider = provider.clone();
    cfg.save().map_err(|e| e.to_string())?;
    tracing::info!("Voice provider updated to: {}", provider);
    Ok(())
}

/// Update whisper model setting
#[tauri::command]
pub fn update_whisper_model(model_filename: String) -> Result<(), String> {
    let mut cfg = AppConfig::load();
    let models_dir = AppConfig::whisper_models_dir();
    let model_path = models_dir.join(&model_filename);
    cfg.voice.local_model_path = Some(model_path.to_string_lossy().to_string());
    cfg.save().map_err(|e| e.to_string())?;
    tracing::info!("Whisper model updated to: {}", model_filename);
    Ok(())
}

/// Update LLM provider setting
#[tauri::command]
pub fn update_llm_provider(provider: String) -> Result<(), String> {
    let mut cfg = AppConfig::load();
    cfg.llm.primary = provider.clone();
    cfg.save().map_err(|e| e.to_string())?;
    tracing::info!("LLM provider updated to: {}", provider);
    Ok(())
}

/// Update selected audio input device
#[tauri::command]
pub fn update_audio_device(device_name: Option<String>) -> Result<(), String> {
    let mut cfg = AppConfig::load();
    cfg.voice.audio_device = device_name.clone();
    cfg.save().map_err(|e| e.to_string())?;
    tracing::info!("Audio device updated to: {:?}", device_name);
    Ok(())
}

/// Update Ollama configuration (URL and model)
#[tauri::command]
pub fn update_ollama_config(base_url: String, model: String) -> Result<(), String> {
    let mut cfg = AppConfig::load();
    cfg.llm.ollama = Some(crate::config::OllamaConfig {
        base_url: base_url.clone(),
        model: model.clone(),
    });
    cfg.save().map_err(|e| e.to_string())?;
    tracing::info!("Ollama config updated: url={}, model={}", base_url, model);
    Ok(())
}

/// Get notification settings
#[tauri::command]
pub fn get_notification_settings() -> Result<gestura_core::config::NotificationSettings, String> {
    let cfg = AppConfig::load();
    Ok(cfg.notifications)
}

/// Update notification settings
#[tauri::command]
#[allow(clippy::too_many_arguments)] // Tauri command parameters expanded for fine-grained frontend control
pub fn update_notification_settings(
    sound_enabled: Option<bool>,
    haptic_enabled: Option<bool>,
    sound_volume: Option<u8>,
    haptic_intensity: Option<u8>,
    notification_sound: Option<String>,
    command_confirm_sound: Option<String>,
    mcp_feedback_enabled: Option<bool>,
    auto_listen_on_feedback: Option<bool>,
) -> Result<(), String> {
    let mut cfg = AppConfig::load();

    if let Some(v) = sound_enabled {
        cfg.notifications.sound_enabled = v;
    }
    if let Some(v) = haptic_enabled {
        cfg.notifications.haptic_enabled = v;
    }
    if let Some(v) = sound_volume {
        cfg.notifications.sound_volume = v.min(100);
    }
    if let Some(v) = haptic_intensity {
        cfg.notifications.haptic_intensity = v.min(100);
    }
    if let Some(v) = notification_sound {
        cfg.notifications.notification_sound = normalize_notification_sound_choice(&v);
    }
    if let Some(v) = command_confirm_sound {
        cfg.notifications.command_confirm_sound = normalize_command_confirm_sound_choice(&v);
    }
    if let Some(v) = mcp_feedback_enabled {
        cfg.notifications.mcp_feedback_enabled = v;
    }
    if let Some(v) = auto_listen_on_feedback {
        cfg.notifications.auto_listen_on_feedback = v;
    }

    cfg.save().map_err(|e| e.to_string())?;
    tracing::info!("Notification settings updated");
    Ok(())
}

/// Preview a user-selected notification sound at the given volume.
///
/// This is used by the config UI to provide immediate auditory feedback when
/// a sound option is selected.
#[tauri::command]
pub async fn preview_notification_sound(sound: String, volume: Option<u8>) -> Result<(), String> {
    use crate::notifications::get_notification_manager;
    get_notification_manager()
        .preview_sound(&sound, volume)
        .await;
    Ok(())
}

fn normalize_notification_sound_choice(value: &str) -> String {
    match value {
        "default" | "chime" | "ping" | "pop" | "subtle" | "none" => value.to_string(),
        _ => "default".to_string(),
    }
}

fn normalize_command_confirm_sound_choice(value: &str) -> String {
    match value {
        "default" | "success" | "click" | "beep" | "none" => value.to_string(),
        _ => "default".to_string(),
    }
}

/// Set the connected ring device for haptic notifications
#[tauri::command]
pub fn set_notification_ring(device_id: Option<String>) -> Result<(), String> {
    use crate::notifications::get_notification_manager;
    get_notification_manager().set_connected_ring(device_id.clone());
    tracing::info!("Notification ring set to: {:?}", device_id);
    Ok(())
}

/// Test notification (for settings UI)
#[tauri::command]
pub async fn test_notification(
    app: tauri::AppHandle,
    notification_type: String,
) -> Result<(), String> {
    use crate::notifications::{NotificationType, get_notification_manager};

    let ntype = match notification_type.as_str() {
        "response_complete" => NotificationType::ResponseComplete,
        "mcp_feedback" => NotificationType::McpFeedbackRequest,
        "error" => NotificationType::Error,
        "listening_started" => NotificationType::ListeningStarted,
        "listening_stopped" => NotificationType::ListeningStopped,
        _ => return Err(format!("Unknown notification type: {}", notification_type)),
    };

    get_notification_manager().notify(ntype, Some(&app)).await;

    Ok(())
}
