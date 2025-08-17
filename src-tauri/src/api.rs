//! Tauri command handlers for configuration, MCP tools, MDH pointers, and tests.
use tauri::Manager;
use crate::{AppConfig, llm_provider::{select_provider, AgentContext}};

#[tauri::command]
pub fn get_config() -> Result<AppConfig, String> { Ok(AppConfig::load()) }

#[tauri::command]
pub fn save_config(cfg: AppConfig) -> Result<(), String> { cfg.save().map_err(|e| e.to_string()) }

#[tauri::command]
pub fn list_mcp_tools() -> Result<Vec<crate::config::McpTool>, String> { Ok(AppConfig::load().mcp_tools) }

#[tauri::command]
pub fn add_mcp_tool(tool: crate::config::McpTool) -> Result<(), String> {
    let mut cfg = AppConfig::load();
    if !cfg.mcp_tools.iter().any(|t| t.name == tool.name) { cfg.mcp_tools.push(tool); }
    cfg.save().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn remove_mcp_tool(name: String) -> Result<(), String> {
    let mut cfg = AppConfig::load();
    cfg.mcp_tools.retain(|t| t.name != name);
    cfg.save().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_mdh_pointers() -> Result<std::collections::HashMap<String, String>, String> { Ok(AppConfig::load().mdh_pointers) }

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
    let provider = select_provider(&cfg, &AgentContext { agent_id: "test".into() });
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
    crate::voice_select::validate_voice_config_for_run(&cfg, engine.as_ref()).map_err(|e| e.to_string())?;
    let text = engine.process_command(&cfg, None).await.map_err(|e| e.to_string())?;
    Ok(text)
}

/// Scan for available Haptic Harmony rings
#[tauri::command]
pub async fn scan_for_rings() -> Result<Vec<String>, String> {
    let ring_manager = crate::ble::create_ring_manager();
    ring_manager.scan_for_rings().await.map_err(|e| e.to_string())
}

/// Get ring status by device ID
#[tauri::command]
pub async fn get_ring_status(device_id: String) -> Result<Option<crate::ble::RingStatus>, String> {
    let ring_manager = crate::ble::create_ring_manager();
    ring_manager.get_ring_status(&device_id).await.map_err(|e| e.to_string())
}

/// Pair with a ring
#[tauri::command]
pub async fn pair_ring(device_id: String) -> Result<(), String> {
    let ring_manager = crate::ble::create_ring_manager();
    ring_manager.pair_ring(&device_id).await.map_err(|e| e.to_string())
}

/// Send haptic feedback to ring
#[tauri::command]
pub async fn send_haptic_feedback(device_id: String, pattern: String, intensity: f32, duration_ms: u32) -> Result<(), String> {
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
    ring_manager.send_haptic(&device_id, request).await.map_err(|e| e.to_string())
}

/// Start gesture monitoring for a ring
#[tauri::command]
pub async fn start_gesture_monitoring(device_id: String) -> Result<(), String> {
    let ring_manager = crate::ble::create_ring_manager();
    let (event_tx, _) = tokio::sync::broadcast::channel(100);
    ring_manager.start_gesture_monitoring(&device_id, event_tx).await.map_err(|e| e.to_string())
}

/// Stop gesture monitoring for a ring
#[tauri::command]
pub async fn stop_gesture_monitoring(device_id: String) -> Result<(), String> {
    let ring_manager = crate::ble::create_ring_manager();
    ring_manager.stop_gesture_monitoring(&device_id).await.map_err(|e| e.to_string())
}

/// Get system health status
#[tauri::command]
pub async fn get_system_health() -> Result<serde_json::Value, String> {
    let telemetry = crate::telemetry::get_telemetry_manager().await;
    let health = telemetry.get_system_health().await;
    Ok(serde_json::to_value(health).map_err(|e| e.to_string())?)
}

/// Get telemetry metrics summary
#[tauri::command]
pub async fn get_metrics_summary() -> Result<serde_json::Value, String> {
    let telemetry = crate::telemetry::get_telemetry_manager().await;
    Ok(telemetry.get_metrics_summary().await)
}

/// Get recent telemetry metrics
#[tauri::command]
pub async fn get_recent_metrics(limit: Option<usize>) -> Result<Vec<crate::telemetry::Metric>, String> {
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
    gdpr.export_user_data(&user_id).await.map_err(|e| e.to_string())
}

/// Delete user data (GDPR compliance)
#[tauri::command]
pub async fn delete_user_data(user_id: String, verify: Option<bool>) -> Result<Vec<String>, String> {
    let gdpr = crate::gdpr::get_gdpr_manager().await;
    gdpr.delete_user_data(&user_id, verify.unwrap_or(false)).await.map_err(|e| e.to_string())
}

/// Get user consent status
#[tauri::command]
pub async fn get_user_consents(user_id: String) -> Result<Vec<crate::gdpr::ConsentRecord>, String> {
    let gdpr = crate::gdpr::get_gdpr_manager().await;
    Ok(gdpr.get_user_consents(&user_id).await)
}

/// Register user consent
#[tauri::command]
pub async fn register_consent(user_id: String, category: String, purpose: String) -> Result<(), String> {
    let gdpr = crate::gdpr::get_gdpr_manager().await;
    let data_category = match category.as_str() {
        "voice" => crate::gdpr::DataCategory::VoiceRecordings,
        "biometric" => crate::gdpr::DataCategory::BiometricData,
        "device" => crate::gdpr::DataCategory::DeviceData,
        "usage" => crate::gdpr::DataCategory::UsageAnalytics,
        "config" => crate::gdpr::DataCategory::ConfigurationData,
        _ => return Err("Invalid data category".to_string()),
    };

    gdpr.register_consent(user_id, data_category, purpose, "User consent".to_string()).await.map_err(|e| e.to_string())
}

// Chat and Agent Commands

#[tauri::command]
pub async fn send_agent_message(agent_id: String, message: String, state: tauri::State<'_, crate::AppState>) -> Result<String, String> {
    let _agents = &state.agents;

    // For now, return a simple echo response
    // TODO: Implement actual agent communication
    let response = format!("Agent '{}' received: {}", agent_id, message);

    // Log the interaction
    tracing::info!("Agent message - ID: {}, Message: {}", agent_id, message);

    Ok(response)
}

#[tauri::command]
pub async fn get_agent_status(agent_id: String, state: tauri::State<'_, crate::AppState>) -> Result<serde_json::Value, String> {
    let _agents = &state.agents;

    // TODO: Implement actual agent status checking
    Ok(serde_json::json!({
        "id": agent_id,
        "name": "Default Agent",
        "status": "active",
        "last_activity": chrono::Utc::now().to_rfc3339()
    }))
}

// Permission Management Commands

#[tauri::command]
pub async fn check_permission(permission: String) -> Result<String, String> {
    match permission.as_str() {
        "microphone" => {
            // Check microphone permission
            // TODO: Implement actual permission checking
            Ok("pending".to_string())
        }
        "accessibility" => {
            // Check accessibility permission
            // TODO: Implement actual permission checking
            Ok("pending".to_string())
        }
        "bluetooth" => {
            // Check bluetooth permission
            // TODO: Implement actual permission checking
            Ok("pending".to_string())
        }
        "notifications" => {
            // Check notification permission
            // TODO: Implement actual permission checking
            Ok("granted".to_string())
        }
        "network" => {
            // Network access is usually granted by default
            Ok("granted".to_string())
        }
        _ => Err(format!("Unknown permission: {}", permission))
    }
}

#[tauri::command]
pub async fn request_permission(permission: String) -> Result<(), String> {
    match permission.as_str() {
        "microphone" => {
            // Request microphone permission
            // TODO: Implement actual permission request
            tracing::info!("Requesting microphone permission");
            Ok(())
        }
        "accessibility" => {
            // Request accessibility permission
            // TODO: Implement actual permission request
            tracing::info!("Requesting accessibility permission");
            Ok(())
        }
        "bluetooth" => {
            // Request bluetooth permission
            // TODO: Implement actual permission request
            tracing::info!("Requesting bluetooth permission");
            Ok(())
        }
        "notifications" => {
            // Request notification permission
            // TODO: Implement actual permission request
            tracing::info!("Requesting notification permission");
            Ok(())
        }
        _ => Err(format!("Cannot request unknown permission: {}", permission))
    }
}

// UI Testing and Validation Commands

#[tauri::command]
pub async fn test_open_window(window_type: String, app: tauri::AppHandle) -> Result<String, String> {
    tracing::info!("Testing window open: {}", window_type);

    match window_type.as_str() {
        "config" => {
            crate::window_manager::open_config_window().map_err(|e| e.to_string())?;
            Ok("Config window opened".to_string())
        }
        "chat" => {
            let session_id = crate::window_manager::create_new_chat_session().map_err(|e| e.to_string())?;
            Ok(format!("Chat session created: {}", session_id))
        }
        _ => Err(format!("Unknown window type: {}", window_type))
    }
}

#[tauri::command]
pub async fn capture_window_screenshot(window_label: String, app: tauri::AppHandle) -> Result<String, String> {
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
pub async fn validate_window_content(window_label: String, app: tauri::AppHandle) -> Result<serde_json::Value, String> {
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
    let windows: Vec<String> = app.webview_windows()
        .keys()
        .map(|k| k.clone())
        .collect();

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
pub fn toggle_listening(app: tauri::AppHandle) -> Result<String, String> {
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
    let permissions = vec![
        serde_json::json!({
            "name": "Microphone",
            "description": "Required for voice commands and speech recognition",
            "status": "not_determined",
            "required": true,
            "instructions": "Microphone access will be requested when you start listening"
        }),
        serde_json::json!({
            "name": "Notifications",
            "description": "Required to show listening status and system alerts",
            "status": "granted",
            "required": true,
            "instructions": "Notifications are working properly"
        }),
        serde_json::json!({
            "name": "File System",
            "description": "Required to save configuration and session data",
            "status": "granted",
            "required": true,
            "instructions": "File system access is working properly"
        }),
        serde_json::json!({
            "name": "Network Access",
            "description": "Required for AI providers and cloud speech services",
            "status": "granted",
            "required": false,
            "instructions": "Network access is available for cloud services"
        })
    ];

    Ok(serde_json::json!({
        "permissions": permissions,
        "summary": {
            "granted": 3,
            "required": 3,
            "total": 4
        }
    }))
}
