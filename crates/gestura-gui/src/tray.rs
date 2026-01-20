//! System tray utilities for Gestura
use crate::window_manager::{self, get_all_sessions};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Listener, Manager};

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
            // 3 minutes timeout: 2 min max recording + 1 min for transcription/LLM processing
            timeout_duration: Duration::from_secs(180),
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

    // Load tray icon from bundled resources
    // Detect system appearance and load appropriate icon (black for light mode, white for dark mode)
    let icon = {
        // Detect dark mode on macOS
        let is_dark_mode = is_system_dark_mode();
        let icon_variant = if is_dark_mode { "white" } else { "black" };
        let icon_filename = format!("icons/tray/icon-{}@2x.png", icon_variant);

        tracing::info!(
            "System dark mode: {}, loading {} icon",
            is_dark_mode,
            icon_variant
        );

        // Try to load from bundled resources first (works in production builds)
        let resource_icon = app
            .path()
            .resolve(&icon_filename, tauri::path::BaseDirectory::Resource)
            .ok()
            .and_then(|path| {
                tracing::info!("Attempting to load tray icon from: {:?}", path);
                std::fs::read(&path).ok().and_then(|bytes| {
                    image::load_from_memory(&bytes).ok().map(|img| {
                        let rgba = img.to_rgba8();
                        let (width, height) = rgba.dimensions();
                        Image::new_owned(rgba.into_raw(), width, height)
                    })
                })
            });

        match resource_icon {
            Some(icon) => {
                tracing::info!("Loaded tray icon from bundled resources");
                icon
            }
            None => {
                // Fall back to embedded icon for development builds
                tracing::info!("Falling back to embedded tray icon ({})", icon_variant);
                // Use black icon as default embedded fallback
                // Convert to slices to handle different array sizes
                let icon_bytes: &[u8] = if is_dark_mode {
                    include_bytes!("../icons/tray/icon-white@2x.png").as_slice()
                } else {
                    include_bytes!("../icons/tray/icon-black@2x.png").as_slice()
                };
                match image::load_from_memory(icon_bytes) {
                    Ok(img) => {
                        let rgba = img.to_rgba8();
                        let (width, height) = rgba.dimensions();
                        Image::new_owned(rgba.into_raw(), width, height)
                    }
                    Err(e) => {
                        tracing::warn!("Failed to load embedded tray icon: {}, using fallback", e);
                        // Create a simple 22x22 colored icon as fallback (blue circle)
                        let size = 22u32;
                        let mut rgba = vec![0u8; (size * size * 4) as usize];
                        for i in 0..(size * size) as usize {
                            rgba[i * 4] = 66; // R
                            rgba[i * 4 + 1] = 133; // G
                            rgba[i * 4 + 2] = 244; // B
                            rgba[i * 4 + 3] = 255; // A
                        }
                        Image::new_owned(rgba, size, size)
                    }
                }
            }
        }
    };

    // Create tray icon with proper icon and event handlers
    // Use icon_as_template(true) for macOS - this allows the system to automatically
    // adjust the icon colors for light/dark menu bar appearance
    let tray = TrayIconBuilder::new()
        .menu(&menu)
        .icon(icon)
        .icon_as_template(true)
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

    // Listen for sessions-changed events to rebuild the tray menu
    let app_handle = app.clone();
    app.listen("sessions-changed", move |_event| {
        tracing::info!("Received sessions-changed event, rebuilding tray menu");
        if let Err(e) = rebuild_tray_menu(&app_handle) {
            tracing::error!("Failed to rebuild tray menu after sessions change: {}", e);
        }
    });

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
    let new_chat = MenuItem::with_id(
        app,
        "new_chat",
        "New Chat Session",
        true,
        Option::<&str>::None,
    )?;
    let config = MenuItem::with_id(app, "config", "Configuration", true, Option::<&str>::None)?;
    let open_shell = MenuItem::with_id(
        app,
        "open_shell",
        "Open In Shell",
        true,
        Option::<&str>::None,
    )?;

    // DevTools entries are debug-only so release builds don't surface internal tooling.
    #[cfg(debug_assertions)]
    let devtools_config = MenuItem::with_id(
        app,
        "devtools_config",
        "Open Config DevTools",
        true,
        Option::<&str>::None,
    )?;
    #[cfg(debug_assertions)]
    let devtools_last_chat = MenuItem::with_id(
        app,
        "devtools_last_chat",
        "Open Last Chat DevTools",
        true,
        Option::<&str>::None,
    )?;

    // Sessions submenu
    let sessions_menu = build_sessions_submenu(app)?;

    // Separators
    let separator1 = PredefinedMenuItem::separator(app)?;
    let separator2 = PredefinedMenuItem::separator(app)?;
    #[cfg(debug_assertions)]
    let separator3 = PredefinedMenuItem::separator(app)?;

    // Exit
    let quit = MenuItem::with_id(app, "quit", "Exit Gestura", true, Option::<&str>::None)?;

    // Build menu structure
    menu.append(&listen)?;
    menu.append(&separator1)?;
    menu.append(&new_chat)?;
    menu.append(&sessions_menu)?;
    menu.append(&open_shell)?;
    menu.append(&separator2)?;
    menu.append(&config)?;

    #[cfg(debug_assertions)]
    {
        menu.append(&devtools_config)?;
        menu.append(&devtools_last_chat)?;
        menu.append(&separator3)?;
    }
    menu.append(&quit)?;

    Ok(menu)
}

/// Build sessions submenu with active and closed sessions
fn build_sessions_submenu(app: &AppHandle) -> tauri::Result<Submenu<tauri::Wry>> {
    use tauri::menu::SubmenuBuilder;

    let all_sessions = get_all_sessions();

    tracing::info!(
        "Building sessions submenu: {} total sessions retrieved",
        all_sessions.len()
    );

    let active_sessions: Vec<_> = all_sessions.iter().filter(|s| s.is_open).collect();
    let closed_sessions: Vec<_> = all_sessions.iter().filter(|s| !s.is_open).collect();

    tracing::info!(
        "Session breakdown: {} active, {} closed",
        active_sessions.len(),
        closed_sessions.len()
    );

    // Use SubmenuBuilder to properly create submenu with items
    let mut builder = SubmenuBuilder::with_id(app, "sessions", "📋 Chat Sessions");

    if active_sessions.is_empty() && closed_sessions.is_empty() {
        let no_sessions = MenuItem::with_id(
            app,
            "no_sessions",
            "No sessions yet",
            false,
            Option::<&str>::None,
        )?;
        builder = builder.item(&no_sessions);
    } else {
        // Add active sessions
        if !active_sessions.is_empty() {
            let active_header = MenuItem::with_id(
                app,
                "active_header",
                format!("── Active ({}) ──", active_sessions.len()),
                false,
                Option::<&str>::None,
            )?;
            builder = builder.item(&active_header);

            // Add individual active sessions (limit to 10)
            for session in active_sessions.iter().take(10) {
                let label = format!(
                    "💬 {} ({})",
                    session.title,
                    session.last_active.format("%H:%M")
                );
                let session_item = MenuItem::with_id(
                    app,
                    format!("session_{}", session.id),
                    label,
                    true,
                    Option::<&str>::None,
                )?;
                builder = builder.item(&session_item);
            }
        }

        // Add closed sessions
        if !closed_sessions.is_empty() {
            if !active_sessions.is_empty() {
                builder = builder.separator();
            }

            let closed_header = MenuItem::with_id(
                app,
                "closed_header",
                format!("── Closed ({}) ──", closed_sessions.len()),
                false,
                Option::<&str>::None,
            )?;
            builder = builder.item(&closed_header);

            // Add individual closed sessions (limit to 5)
            for session in closed_sessions.iter().take(5) {
                let label = format!(
                    "📁 {} ({})",
                    session.title,
                    session.last_active.format("%b %d")
                );
                let session_item = MenuItem::with_id(
                    app,
                    format!("session_{}", session.id),
                    label,
                    true,
                    Option::<&str>::None,
                )?;
                builder = builder.item(&session_item);
            }

            // Add "Restore All" option if more than one closed session
            if closed_sessions.len() > 1 {
                builder = builder.separator();

                let restore_all = MenuItem::with_id(
                    app,
                    "restore_all",
                    "🔄 Restore All Sessions",
                    true,
                    Option::<&str>::None,
                )?;
                builder = builder.item(&restore_all);
            }
        }
    }

    builder.build()
}

/// Helper to open DevTools for a specific window label
#[cfg(debug_assertions)]
fn open_window_devtools(app: &AppHandle, window_label: &str) {
    if let Some(window) = app.get_webview_window(window_label) {
        // Ensure window is visible and focused before opening DevTools
        if let Err(e) = window.show() {
            tracing::warn!(
                "Failed to show window '{}' before opening DevTools: {}",
                window_label,
                e
            );
        }
        if let Err(e) = window.set_focus() {
            tracing::warn!(
                "Failed to focus window '{}' before opening DevTools: {}",
                window_label,
                e
            );
        }
        // In Tauri v2, the devtools APIs are only available in debug builds by default.
        // The `open_devtools` method itself is gated behind `debug_assertions`, so we
        // must also guard our call to avoid compile errors in release (signed) builds.
        #[cfg(debug_assertions)]
        {
            window.open_devtools();
            tracing::info!("Opened DevTools for window '{}'", window_label);
        }
        #[cfg(not(debug_assertions))]
        {
            tracing::info!(
                "DevTools open requested for window '{}' but debug assertions are disabled; \
	                 DevTools can only be opened programmatically in dev/debug builds.",
                window_label
            );
        }
    } else {
        tracing::warn!(
            "Requested DevTools for window '{}', but no such window exists",
            window_label
        );
    }
}

/// Helper to open DevTools for the most recently active open chat session
#[cfg(debug_assertions)]
fn open_last_chat_devtools(app: &AppHandle) {
    let sessions = get_all_sessions();
    let maybe_session = sessions
        .into_iter()
        .filter(|s| s.is_open)
        .max_by_key(|s| s.last_active);

    if let Some(session) = maybe_session {
        if let Some(label) = session.window_label.as_deref() {
            open_window_devtools(app, label);
        } else {
            tracing::warn!(
                "Open chat session '{}' has no associated window label; cannot open DevTools",
                session.id
            );
        }
    } else {
        tracing::warn!("No open chat sessions available to open DevTools for");
    }
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
        "open_shell" => {
            tracing::info!("Opening shell session from tray menu");
            match window_manager::open_shell_session() {
                Ok(()) => {
                    tracing::info!("Shell session opened successfully");
                    show_system_notification(
                        app,
                        "Shell Session",
                        "Gestura shell session opened in terminal",
                    );
                }
                Err(e) => {
                    tracing::error!("Failed to open shell session: {}", e);
                    show_system_notification(
                        app,
                        "Shell Session Error",
                        &format!("Failed to open shell: {}", e),
                    );
                }
            }
        }
        #[cfg(debug_assertions)]
        "devtools_config" => {
            tracing::info!("Opening DevTools for config window from tray menu");
            if let Err(e) = window_manager::open_config_window() {
                tracing::error!("Failed to open config window before DevTools: {}", e);
            }
            open_window_devtools(app, "config");
        }
        #[cfg(debug_assertions)]
        "devtools_last_chat" => {
            tracing::info!("Opening DevTools for last active chat window from tray menu");
            open_last_chat_devtools(app);
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
            toggle_listening_mode(app);
        }
        // Note: Right-click events are handled automatically by the tray system
        _ => {
            tracing::debug!("Other tray event: {:?}", event);
        }
    }
}

/// Start listening with shared validation logic used by both the tray and
/// chat UI entry points.
///
/// This ensures we always run the same configuration checks (provider
/// selected, OpenAI key present, local Whisper model available, etc.) before
/// starting the speech pipeline.
pub fn start_listening_with_validation(app: &AppHandle) -> Result<(), String> {
    // Validate configuration first so that tray-initiated starts behave the
    // same way as the chat UI and return user-friendly error messages.
    let validation = crate::api::validate_voice_and_llm_config_sync();
    if !validation.is_valid {
        let error_msg = format!(
            "{} {}",
            validation
                .error_message
                .unwrap_or_else(|| "Configuration error".to_string()),
            validation.suggestion.unwrap_or_default()
        );
        return Err(error_msg);
    }

    // Scope the lock to avoid deadlock when calling rebuild_tray_menu
    let timeout_duration = {
        let mut state = LISTENING_STATE.lock().unwrap();
        if state.is_listening {
            return Err("Voice listening is already active.".to_string());
        }

        tracing::info!("Starting listening mode");
        state.is_listening = true;
        state.started_at = Some(Instant::now());

        // Start actual speech processing
        let session_id = format!(
            "listening-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
        );
        state.session_id = Some(session_id.clone());
        let timeout_duration = state.timeout_duration;
        tracing::info!("Starting speech processing session: {}", session_id);

        timeout_duration
        // Lock is dropped here at end of scope
    };

    show_system_notification(
        app,
        "Listening Started",
        "Gestura is now listening for voice commands. Speak your command now.",
    );

    // Start speech capture and processing
    // Use tauri::async_runtime::spawn instead of tokio::spawn because menu
    // event handlers run on the main thread outside of a tokio async context.
    let app_handle = app.clone();
    tauri::async_runtime::spawn(async move {
        // Start speech listening with timeout
        let speech_result = tokio::time::timeout(
            timeout_duration,
            crate::speech::start_speech_listening(&app_handle),
        )
        .await;

        match speech_result {
            Ok(Ok(())) => {
                tracing::info!("Speech processing completed successfully");
                // Reset listening state after successful completion
                reset_listening_state(&app_handle);
            }
            Ok(Err(e)) => {
                tracing::error!("Speech processing failed: {}", e);
                show_system_notification(
                    &app_handle,
                    "Listening Error",
                    &format!("Speech processing failed: {}", e),
                );
                // Reset listening state after failure
                reset_listening_state(&app_handle);
            }
            Err(_) => {
                tracing::info!("Speech processing timed out");
                stop_listening_on_timeout(&app_handle);
            }
        }
    });

    // Rebuild tray menu to update button text
    // Note: Lock must be released before this call to avoid deadlock
    let _ = rebuild_tray_menu(app);

    // Emit event to notify frontend that listening has started
    let _ = app.emit(
        "listening-state-changed",
        serde_json::json!({
            "is_listening": true
        }),
    );
    tracing::info!("Emitted listening-state-changed event (started)");

    Ok(())
}

/// Toggle listening mode (start/stop)
fn toggle_listening_mode(app: &AppHandle) {
    // Check current state and update if stopping
    let was_listening = {
        let mut state = LISTENING_STATE.lock().unwrap();
        if state.is_listening {
            // Stop listening
            tracing::info!("Stopping listening mode");
            state.is_listening = false;
            state.started_at = None;
            state.session_id = None;
            true
        } else {
            false
        }
        // Lock is dropped here
    };

    if was_listening {
        // Stop speech processing (outside of lock)
        if let Err(e) = crate::speech::stop_speech_listening() {
            tracing::warn!("Failed to stop speech processing: {}", e);
        }

        show_system_notification(app, "Listening Stopped", "Voice listening has been stopped");

        // Rebuild tray menu to update button text (lock is not held)
        let _ = rebuild_tray_menu(app);

        // Emit event to notify frontend that listening has stopped
        let _ = app.emit(
            "listening-state-changed",
            serde_json::json!({
                "is_listening": false
            }),
        );
        tracing::info!("Emitted listening-state-changed event (stopped via toggle)");
    } else {
        // Start listening (lock already released)
        if let Err(err) = start_listening_with_validation(app) {
            tracing::warn!("Failed to start listening from tray: {}", err);
            show_system_notification(app, "Listening Error", &err);
        }
    }
}

/// Stop listening when timeout is reached
fn stop_listening_on_timeout(app: &AppHandle) {
    // Check and update state within a scoped lock to avoid deadlock
    let should_rebuild = {
        let mut state = LISTENING_STATE.lock().unwrap();

        if state.is_listening
            && let Some(started_at) = state.started_at
            && started_at.elapsed() >= state.timeout_duration
        {
            tracing::info!("Listening timeout reached, stopping");
            state.is_listening = false;
            state.started_at = None;
            state.session_id = None;
            true
        } else {
            false
        }
        // Lock is dropped here
    };

    if should_rebuild {
        show_system_notification(
            app,
            "Listening Timeout",
            "Voice listening stopped due to timeout",
        );

        // Rebuild tray menu to update button text (lock is not held)
        let _ = rebuild_tray_menu(app);

        // Emit event to notify frontend that listening has stopped
        let _ = app.emit(
            "listening-state-changed",
            serde_json::json!({
                "is_listening": false
            }),
        );
        tracing::info!("Emitted listening-state-changed event (timeout)");
    }
}

/// Reset listening state after speech processing completes (success or failure)
fn reset_listening_state(app: &AppHandle) {
    // Check and update state within a scoped lock to avoid deadlock
    let should_rebuild = {
        let mut state = LISTENING_STATE.lock().unwrap();

        if state.is_listening {
            tracing::info!("Resetting listening state after speech processing completed");
            state.is_listening = false;
            state.started_at = None;
            state.session_id = None;
            true
        } else {
            false
        }
        // Lock is dropped here
    };

    if should_rebuild {
        // Rebuild tray menu to update button text (lock is not held)
        let _ = rebuild_tray_menu(app);

        // Emit event to notify frontend that listening has stopped
        let _ = app.emit(
            "listening-state-changed",
            serde_json::json!({
                "is_listening": false
            }),
        );
        tracing::info!("Emitted listening-state-changed event (stopped)");
    }
}

/// Rebuild tray menu to update dynamic content
fn rebuild_tray_menu(app: &AppHandle) -> tauri::Result<()> {
    tracing::info!("Rebuilding tray menu to update listen button text");

    // Build a new menu with updated state
    let new_menu = build_tray_menu(app)?;

    // Get the tray instance and update its menu
    let tray_instance = TRAY_INSTANCE.lock().unwrap();
    if let Some(tray) = tray_instance.as_ref() {
        tray.set_menu(Some(new_menu))?;
        tracing::info!("Tray menu rebuilt successfully");
    } else {
        tracing::warn!("No tray instance found to rebuild menu");
    }

    Ok(())
}

/// Show system notification
fn show_system_notification(_app: &AppHandle, title: &str, body: &str) {
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
            .arg(format!(
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
        Command::new("notify-send").arg(title).arg(body).output()?;
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

/// Start voice listening
pub fn start_listening() {
    let mut state = LISTENING_STATE.lock().unwrap();
    if !state.is_listening {
        state.is_listening = true;
        state.started_at = Some(Instant::now());
        state.session_id = Some(uuid::Uuid::new_v4().to_string());
        tracing::info!("Voice listening started");
    }
}

/// Stop voice listening
pub fn stop_listening() {
    let mut state = LISTENING_STATE.lock().unwrap();
    if state.is_listening {
        state.is_listening = false;
        state.started_at = None;
        state.session_id = None;
        tracing::info!("Voice listening stopped");
    }
}

/// Restore all closed sessions
fn restore_all_sessions() {
    let sessions = get_all_sessions();
    let closed_sessions: Vec<_> = sessions.iter().filter(|s| !s.is_open).collect();

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

/// Detect if the system is using dark mode
#[cfg(target_os = "macos")]
fn is_system_dark_mode() -> bool {
    use std::process::Command;

    // Query macOS for the current appearance setting
    let output = Command::new("defaults")
        .args(["read", "-g", "AppleInterfaceStyle"])
        .output();

    match output {
        Ok(output) => {
            let result = String::from_utf8_lossy(&output.stdout);
            result.trim().eq_ignore_ascii_case("dark")
        }
        Err(_) => {
            // If the command fails or the key doesn't exist, assume light mode
            false
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn is_system_dark_mode() -> bool {
    // Default to light mode on non-macOS platforms
    false
}
