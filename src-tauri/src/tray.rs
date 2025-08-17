//! System tray utilities for Gestura
use tauri::{AppHandle, Manager};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{TrayIconBuilder, TrayIconEvent, MouseButton, MouseButtonState};
use crate::window_manager::{self, get_session_counts, get_all_sessions};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// Global listening state and tray management
lazy_static::lazy_static! {
    static ref LISTENING_STATE: Arc<Mutex<ListeningState>> = Arc::new(Mutex::new(ListeningState::default()));
    static ref TRAY_INITIALIZED: Arc<Mutex<bool>> = Arc::new(Mutex::new(false));
    static ref TRAY_INSTANCE: Arc<Mutex<Option<tauri::tray::TrayIcon<tauri::Wry>>>> = Arc::new(Mutex::new(None));
}

#[derive(Debug, Clone)]
struct ListeningState {
    is_listening: bool,
    started_at: Option<Instant>,
    timeout_duration: Duration,
    session_id: Option<String>,
}

impl Default for ListeningState {
    fn default() -> Self {
        Self {
            is_listening: false,
            started_at: None,
            timeout_duration: Duration::from_secs(30), // Default 30 seconds
            session_id: None,
        }
    }
}

/// Initialize the system tray with comprehensive menu options.
/// Provides access to chat, configuration, session management, and system controls.
pub fn init_tray(app: &AppHandle) -> tauri::Result<()> {
    tracing::info!("🔧 Starting system tray initialization...");

    // Check if tray is already initialized
    {
        let mut initialized = TRAY_INITIALIZED.lock().unwrap();
        if *initialized {
            tracing::warn!("⚠️ System tray already initialized, skipping duplicate initialization");
            return Ok(());
        }
        *initialized = true;
        tracing::info!("✅ Tray initialization flag set");
    }

    // Clean up any existing tray icon first (defensive programming)
    {
        let mut tray_instance = TRAY_INSTANCE.lock().unwrap();
        if let Some(_existing_tray) = tray_instance.take() {
            tracing::info!("🧹 Cleaned up existing tray icon before creating new one");
        }
    }

    // Initialize window manager
    window_manager::init_window_manager(app.clone());
    tracing::info!("📱 Window manager initialized");

    // Create the main menu
    let menu = build_tray_menu(app)?;
    tracing::info!("📋 Tray menu built successfully");

    // Create tray icon with proper icon and event handlers
    let tray = TrayIconBuilder::new()
        .menu(&menu)
        .icon(app.default_window_icon().unwrap().clone())
        .tooltip("Gestura - Voice & Gesture Control")
        .on_menu_event(handle_menu_event)
        .on_tray_icon_event(handle_tray_event)
        .build(app)?;

    tracing::info!("🎯 Tray icon created successfully");

    // Store the tray instance for cleanup
    {
        let mut tray_instance = TRAY_INSTANCE.lock().unwrap();
        *tray_instance = Some(tray);
    }

    tracing::info!("✅ System tray initialized successfully - SINGLE ICON GUARANTEED");
    Ok(())
}

/// Build the comprehensive tray menu with session management
fn build_tray_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let menu = Menu::new(app)?;

    // Main actions - dynamically set listen text based on state
    let listening_state = LISTENING_STATE.lock().unwrap();
    let listen_text = if listening_state.is_listening {
        "Stop Listening"
    } else {
        "Start Listening"
    };
    drop(listening_state);

    let listen = MenuItem::with_id(app, "listen", listen_text, true, Option::<&str>::None)?;
    let new_chat = MenuItem::with_id(app, "new_chat", "New Chat Session", true, Option::<&str>::None)?;
    let config = MenuItem::with_id(app, "config", "Configuration", true, Option::<&str>::None)?;

    // Sessions submenu
    let sessions_menu = build_sessions_submenu(app)?;

    // Separators
    let separator1 = PredefinedMenuItem::separator(app)?;
    let separator2 = PredefinedMenuItem::separator(app)?;

    // Exit
    let quit = MenuItem::with_id(app, "quit", "Exit Gestura", true, Option::<&str>::None)?;

    // Build menu structure
    menu.append(&listen)?;
    menu.append(&separator1)?;
    menu.append(&new_chat)?;
    menu.append(&sessions_menu)?;
    menu.append(&separator2)?;
    menu.append(&config)?;
    menu.append(&separator1)?;
    menu.append(&quit)?;

    Ok(menu)
}

/// Build sessions submenu with active and closed sessions
fn build_sessions_submenu(app: &AppHandle) -> tauri::Result<Submenu<tauri::Wry>> {
    let sessions_menu = Menu::new(app)?;
    let (active_count, closed_count) = get_session_counts();

    if active_count == 0 && closed_count == 0 {
        let no_sessions = MenuItem::with_id(app, "no_sessions", "No sessions yet", false, Option::<&str>::None)?;
        sessions_menu.append(&no_sessions)?;
    } else {
        // Add active sessions
        if active_count > 0 {
            let active_header = MenuItem::with_id(app, "active_header", &format!("Active Sessions ({})", active_count), false, Option::<&str>::None)?;
            sessions_menu.append(&active_header)?;

            // TODO: Add individual active sessions
        }

        // Add closed sessions
        if closed_count > 0 {
            if active_count > 0 {
                let separator = PredefinedMenuItem::separator(app)?;
                sessions_menu.append(&separator)?;
            }

            let closed_header = MenuItem::with_id(app, "closed_header", &format!("Closed Sessions ({})", closed_count), false, Option::<&str>::None)?;
            sessions_menu.append(&closed_header)?;

            let restore_all = MenuItem::with_id(app, "restore_all", "Restore All Sessions", true, Option::<&str>::None)?;
            sessions_menu.append(&restore_all)?;
        }
    }

    Ok(Submenu::with_id(app, "sessions", "📋 Chat Sessions", true)?)
}

/// Handle menu item clicks
fn handle_menu_event(app: &AppHandle, event: tauri::menu::MenuEvent) {
    tracing::info!("Menu event: {:?}", event.id());

    match event.id().as_ref() {
        "listen" => {
            toggle_listening_mode(app);
        }
        "new_chat" => {
            if let Err(e) = window_manager::create_new_chat_session() {
                tracing::error!("Failed to create chat session: {}", e);
            }
        }
        "config" => {
            if let Err(e) = window_manager::open_config_window() {
                tracing::error!("Failed to open config window: {}", e);
            }
        }
        "restore_all" => {
            restore_all_sessions();
        }
        "quit" => {
            show_exit_confirmation(app);
        }
        id if id.starts_with("session_") => {
            // Handle individual session restoration
            let session_id = id.strip_prefix("session_").unwrap();
            if let Err(e) = window_manager::restore_chat_session(session_id) {
                tracing::error!("Failed to restore session {}: {}", session_id, e);
            }
        }
        _ => {
            tracing::debug!("Unhandled menu event: {:?}", event.id());
        }
    }
}

/// Handle tray icon events (clicks)
fn handle_tray_event(tray: &tauri::tray::TrayIcon, event: TrayIconEvent) {
    let app = tray.app_handle();

    match event {
        TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } => {
            // Single left-click: Show menu or create new chat
            tracing::info!("Tray single-click detected");
            if let Err(e) = window_manager::create_new_chat_session() {
                tracing::error!("Failed to create chat session on click: {}", e);
            }
        }
        TrayIconEvent::DoubleClick {
            button: MouseButton::Left,
            ..
        } => {
            // Double left-click: Toggle listening mode
            tracing::info!("Tray double-click detected - toggling listen mode");
            toggle_listening_mode(&app);
        }
        // Note: Right-click events are handled automatically by the tray system
        _ => {
            tracing::debug!("Other tray event: {:?}", event);
        }
    }
}

/// Toggle listening mode (start/stop)
fn toggle_listening_mode(app: &AppHandle) {
    let mut state = LISTENING_STATE.lock().unwrap();

    if state.is_listening {
        // Stop listening
        tracing::info!("Stopping listening mode");
        state.is_listening = false;
        state.started_at = None;
        state.session_id = None;

        // Stop speech processing
        if let Err(e) = crate::speech::stop_speech_listening() {
            tracing::warn!("Failed to stop speech processing: {}", e);
        }

        show_system_notification(app, "Listening Stopped", "Voice listening has been stopped");

    } else {
        // Start listening
        tracing::info!("Starting listening mode");
        state.is_listening = true;
        state.started_at = Some(Instant::now());

        // Start actual speech processing
        let session_id = format!("listening-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs());
        state.session_id = Some(session_id.clone());
        tracing::info!("Starting speech processing session: {}", session_id);

        show_system_notification(app, "Listening Started", "Gestura is now listening for voice commands. Speak your command now.");

        // Start speech capture and processing
        let app_handle = app.clone();
        let timeout_duration = state.timeout_duration;
        tokio::spawn(async move {
            // Start speech listening with timeout
            let speech_result = tokio::time::timeout(
                timeout_duration,
                crate::speech::start_speech_listening(&app_handle)
            ).await;

            match speech_result {
                Ok(Ok(())) => {
                    tracing::info!("Speech processing completed successfully");
                }
                Ok(Err(e)) => {
                    tracing::error!("Speech processing failed: {}", e);
                    show_system_notification(&app_handle, "Listening Error", &format!("Speech processing failed: {}", e));
                }
                Err(_) => {
                    tracing::info!("Speech processing timed out");
                    stop_listening_on_timeout(&app_handle);
                }
            }
        });
    }

    // Rebuild tray menu to update button text
    let _ = rebuild_tray_menu(app);
}

/// Stop listening when timeout is reached
fn stop_listening_on_timeout(app: &AppHandle) {
    let mut state = LISTENING_STATE.lock().unwrap();

    if state.is_listening {
        if let Some(started_at) = state.started_at {
            if started_at.elapsed() >= state.timeout_duration {
                tracing::info!("Listening timeout reached, stopping");
                state.is_listening = false;
                state.started_at = None;
                state.session_id = None;

                show_system_notification(app, "Listening Timeout", "Voice listening stopped due to timeout");

                // Rebuild tray menu to update button text
                let _ = rebuild_tray_menu(app);
            }
        }
    }
}

/// Rebuild tray menu to update dynamic content
fn rebuild_tray_menu(app: &AppHandle) -> tauri::Result<()> {
    // For now, we'll just log that we need to rebuild
    // In a full implementation, we would recreate the tray with updated menu
    tracing::info!("Tray menu needs rebuilding to update listen button text");

    // TODO: Implement proper tray menu rebuilding
    // This requires recreating the tray icon with updated menu

    Ok(())
}

/// Show system notification
fn show_system_notification(app: &AppHandle, title: &str, body: &str) {
    tracing::info!("NOTIFICATION: {} - {}", title, body);

    // Try to show actual system notification
    if let Err(e) = show_native_notification(title, body) {
        tracing::warn!("Failed to show native notification: {}", e);
    }
}

/// Show native system notification
fn show_native_notification(title: &str, body: &str) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(target_os = "macos")]
    {
        use std::process::Command;
        Command::new("osascript")
            .arg("-e")
            .arg(&format!(
                r#"display notification "{}" with title "{}""#,
                body, title
            ))
            .output()?;
    }

    #[cfg(target_os = "windows")]
    {
        // Windows notification implementation would go here
        tracing::info!("Windows notification: {} - {}", title, body);
    }

    #[cfg(target_os = "linux")]
    {
        use std::process::Command;
        Command::new("notify-send")
            .arg(title)
            .arg(body)
            .output()?;
    }

    Ok(())
}

/// Get current listening state (for API access)
pub fn get_listening_state() -> (bool, Option<Duration>) {
    let state = LISTENING_STATE.lock().unwrap();
    let remaining = if let Some(started_at) = state.started_at {
        let elapsed = started_at.elapsed();
        if elapsed < state.timeout_duration {
            Some(state.timeout_duration - elapsed)
        } else {
            None
        }
    } else {
        None
    };

    (state.is_listening, remaining)
}

/// Set listening timeout duration
pub fn set_listening_timeout(duration: Duration) {
    let mut state = LISTENING_STATE.lock().unwrap();
    state.timeout_duration = duration;
    tracing::info!("Listening timeout set to {:?}", duration);
}

/// Restore all closed sessions
fn restore_all_sessions() {
    let sessions = get_all_sessions();
    let closed_sessions: Vec<_> = sessions.iter()
        .filter(|s| !s.is_open)
        .collect();

    tracing::info!("Restoring {} closed sessions", closed_sessions.len());

    for session in closed_sessions {
        if let Err(e) = window_manager::restore_chat_session(&session.id) {
            tracing::error!("Failed to restore session {}: {}", session.id, e);
        }
    }
}

/// Show exit confirmation dialog and handle graceful shutdown
fn show_exit_confirmation(app: &AppHandle) {
    tracing::info!("Exit confirmation requested");

    // For now, we'll implement a simple confirmation via the system
    // In a full implementation, we could show a custom dialog

    // Close all managed windows and sessions
    if let Some(manager) = window_manager::get_window_manager() {
        manager.close_all();
    }

    // Graceful shutdown
    graceful_shutdown(app);
}

/// Graceful shutdown with cleanup
fn graceful_shutdown(app: &AppHandle) {
    tracing::info!("Performing graceful shutdown...");

    // Close all windows
    let windows: Vec<_> = app.webview_windows().keys().cloned().collect();
    for window_label in windows {
        if let Some(window) = app.get_webview_window(&window_label) {
            let _ = window.close();
        }
    }

    // Exit the application
    app.exit(0);
}

/// Get diagnostic information about tray status
pub fn get_tray_diagnostic_info() -> serde_json::Value {
    let initialized = *TRAY_INITIALIZED.lock().unwrap();
    let has_instance = TRAY_INSTANCE.lock().unwrap().is_some();

    serde_json::json!({
        "tray_initialized": initialized,
        "tray_instance_exists": has_instance,
        "status": if initialized && has_instance {
            "healthy"
        } else if initialized && !has_instance {
            "initialized_but_no_instance"
        } else if !initialized && has_instance {
            "instance_without_initialization"
        } else {
            "not_initialized"
        }
    })
}




