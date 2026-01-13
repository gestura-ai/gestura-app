//! Window and session management for Gestura
//! Handles chat sessions, window lifecycle, and session restoration

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatSession {
    pub id: String,
    pub title: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub last_active: chrono::DateTime<chrono::Utc>,
    pub is_open: bool,
    pub window_label: Option<String>,
    pub message_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowInfo {
    pub label: String,
    pub window_type: String,
    pub session_id: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub is_visible: bool,
}

pub struct WindowManager {
    sessions: Arc<Mutex<HashMap<String, ChatSession>>>,
    windows: Arc<Mutex<HashMap<String, WindowInfo>>>,
    app: AppHandle,
}

impl WindowManager {
    pub fn new(app: AppHandle) -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            windows: Arc::new(Mutex::new(HashMap::new())),
            app,
        }
    }

    /// Create a new chat session and window
    pub fn create_chat_session(&self) -> tauri::Result<String> {
        let session_id = Uuid::new_v4().to_string();
        let window_label = format!("chat-{}", session_id);

        tracing::info!("Creating new chat session: {}", session_id);

        // Create the session with user-friendly title
        let now = chrono::Utc::now();
        let session = ChatSession {
            id: session_id.clone(),
            title: format!("Chat {}", now.format("%b %d, %H:%M")),
            created_at: now,
            last_active: now,
            is_open: true,
            window_label: Some(window_label.clone()),
            message_count: 0,
        };

        // Store the session
        {
            let mut sessions = self.sessions.lock().unwrap();
            sessions.insert(session_id.clone(), session);
        }

        // Create the window
        self.create_chat_window(&session_id, &window_label)?;

        // Emit event to notify tray that sessions changed
        let _ = self.app.emit("sessions-changed", ());
        tracing::info!("Emitted sessions-changed event (new session created)");

        Ok(session_id)
    }

    /// Create a chat window for a session
    fn create_chat_window(&self, session_id: &str, window_label: &str) -> tauri::Result<()> {
        let window =
            WebviewWindowBuilder::new(&self.app, window_label, WebviewUrl::App("chat.html".into()))
                .title("Gestura Chat")
                .inner_size(800.0, 600.0)
                .center()
                .resizable(true)
                .decorations(true)
                .visible(true)
                .focused(true)  // Ensure window gets focus when created
                .devtools(true)
                .build()?;

        // Make sure window is shown and focused
        let _ = window.show();
        let _ = window.set_focus();

        // Store window info
        let window_info = WindowInfo {
            label: window_label.to_string(),
            window_type: "chat".to_string(),
            session_id: Some(session_id.to_string()),
            created_at: chrono::Utc::now(),
            is_visible: true,
        };

        {
            let mut windows = self.windows.lock().unwrap();
            windows.insert(window_label.to_string(), window_info);
        }

        // Set up window close handler
        let sessions = Arc::clone(&self.sessions);
        let windows = Arc::clone(&self.windows);
        let app_handle = self.app.clone();
        let session_id = session_id.to_string();
        let window_label_clone = window_label.to_string();

        window.on_window_event(move |event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                tracing::info!("Chat window closing: {}", window_label_clone);

                // Mark session as closed but don't delete it
                if let Ok(mut sessions) = sessions.lock()
                    && let Some(session) = sessions.get_mut(&session_id)
                {
                    session.is_open = false;
                    session.window_label = None;
                    session.last_active = chrono::Utc::now();
                }

                // Remove window info
                if let Ok(mut windows) = windows.lock() {
                    windows.remove(&window_label_clone);
                }

                // Emit event to notify tray that sessions changed
                let _ = app_handle.emit("sessions-changed", ());
                tracing::info!("Emitted sessions-changed event (session closed)");
            }
        });

        tracing::info!("Created chat window: {}", window_label);
        Ok(())
    }

    /// Create configuration window
    pub fn create_config_window(&self) -> tauri::Result<()> {
        let window_label = "config";

        // Check if window already exists
        if self.app.get_webview_window(window_label).is_some()
            && let Some(window) = self.app.get_webview_window(window_label)
        {
            let _ = window.show();
            let _ = window.set_focus();
            return Ok(());
        }

        let _window = WebviewWindowBuilder::new(
            &self.app,
            window_label,
            WebviewUrl::App("config.html".into()),
        )
        .title("Gestura Configuration")
        .inner_size(700.0, 500.0)
        .center()
        .resizable(true)
        .decorations(true)
        .visible(true)
        .devtools(true)
        .build()?;

        tracing::info!("Created config window");
        Ok(())
    }

    /// Create onboarding window for first-time users
    pub fn create_onboarding_window(&self) -> tauri::Result<()> {
        let window_label = "onboarding";

        // Check if window already exists
        if self.app.get_webview_window(window_label).is_some()
            && let Some(window) = self.app.get_webview_window(window_label)
        {
            let _ = window.show();
            let _ = window.set_focus();
            return Ok(());
        }

        let _window = WebviewWindowBuilder::new(
            &self.app,
            window_label,
            WebviewUrl::App("onboarding.html".into()),
        )
        .title("Welcome to Gestura")
        .inner_size(720.0, 580.0)
        .center()
        .resizable(true)
        .decorations(true)
        .visible(true)
        .transparent(false) // Ensure opaque background
        .devtools(true)
        .build()?;

        tracing::info!("Created onboarding window");
        Ok(())
    }

    /// Close the onboarding window
    pub fn close_onboarding_window(&self) -> tauri::Result<()> {
        if let Some(window) = self.app.get_webview_window("onboarding") {
            let _ = window.close();
            tracing::info!("Closed onboarding window");
        }
        Ok(())
    }

    /// Restore a closed chat session
    pub fn restore_session(&self, session_id: &str) -> tauri::Result<()> {
        let session = {
            let sessions = self.sessions.lock().unwrap();
            sessions.get(session_id).cloned()
        };

        if let Some(mut session) = session
            && !session.is_open
        {
            let window_label = format!("chat-{}", session_id);
            self.create_chat_window(session_id, &window_label)?;

            // Update session
            session.is_open = true;
            session.window_label = Some(window_label);
            session.last_active = chrono::Utc::now();

            let mut sessions = self.sessions.lock().unwrap();
            sessions.insert(session_id.to_string(), session);

            tracing::info!("Restored session: {}", session_id);
        }

        Ok(())
    }

    /// Get all sessions (open and closed)
    pub fn get_sessions(&self) -> Vec<ChatSession> {
        let sessions = self.sessions.lock().unwrap();
        sessions.values().cloned().collect()
    }

    /// Get active (open) sessions
    pub fn get_active_sessions(&self) -> Vec<ChatSession> {
        let sessions = self.sessions.lock().unwrap();
        sessions.values().filter(|s| s.is_open).cloned().collect()
    }

    /// Get closed sessions that can be restored
    pub fn get_closed_sessions(&self) -> Vec<ChatSession> {
        let sessions = self.sessions.lock().unwrap();
        sessions.values().filter(|s| !s.is_open).cloned().collect()
    }

    /// Get the focused chat session, if any
    /// Returns the session ID of the currently focused chat window
    pub fn get_focused_chat_session(&self) -> Option<String> {
        let sessions = self.sessions.lock().unwrap();

        // Find active sessions with window labels
        for session in sessions.values().filter(|s| s.is_open) {
            if let Some(ref window_label) = session.window_label
                && let Some(window) = self.app.get_webview_window(window_label)
                && window.is_focused().unwrap_or(false)
            {
                tracing::info!("Found focused chat session: {}", session.id);
                return Some(session.id.clone());
            }
        }

        tracing::info!("No focused chat session found");
        None
    }

    /// Get the most recently active open chat session
    /// Falls back to this if no window is focused
    pub fn get_most_recent_active_session(&self) -> Option<String> {
        let sessions = self.sessions.lock().unwrap();

        sessions
            .values()
            .filter(|s| s.is_open && s.window_label.is_some())
            .max_by_key(|s| s.last_active)
            .map(|s| {
                tracing::info!("Found most recent active session: {}", s.id);
                s.id.clone()
            })
    }

    /// Close all windows and sessions
    pub fn close_all(&self) {
        let windows = {
            let windows = self.windows.lock().unwrap();
            windows.keys().cloned().collect::<Vec<_>>()
        };

        for window_label in windows {
            if let Some(window) = self.app.get_webview_window(&window_label) {
                let _ = window.close();
            }
        }

        // Clear all data
        self.sessions.lock().unwrap().clear();
        self.windows.lock().unwrap().clear();

        tracing::info!("Closed all windows and sessions");
    }

    /// Get session count for menu display
    pub fn get_session_counts(&self) -> (usize, usize) {
        let sessions = self.sessions.lock().unwrap();
        let active = sessions.values().filter(|s| s.is_open).count();
        let closed = sessions.values().filter(|s| !s.is_open).count();
        (active, closed)
    }

    /// Get the window label for a session by its ID
    pub fn get_session_window_label(&self, session_id: &str) -> Option<String> {
        let sessions = self.sessions.lock().unwrap();
        sessions
            .get(session_id)
            .and_then(|s| s.window_label.clone())
    }
}

// Global window manager instance
lazy_static::lazy_static! {
    static ref WINDOW_MANAGER: Mutex<Option<WindowManager>> = Mutex::new(None);
}

/// Initialize the global window manager
pub fn init_window_manager(app: AppHandle) {
    let manager = WindowManager::new(app);
    let mut global_manager = WINDOW_MANAGER.lock().unwrap();
    *global_manager = Some(manager);
    tracing::info!("Window manager initialized");
}

/// Get the global window manager
pub fn get_window_manager() -> Option<WindowManager> {
    let manager = WINDOW_MANAGER.lock().unwrap();
    manager.as_ref().map(|m| WindowManager {
        sessions: Arc::clone(&m.sessions),
        windows: Arc::clone(&m.windows),
        app: m.app.clone(),
    })
}

/// Convenience functions for tray usage
pub fn create_new_chat_session() -> tauri::Result<String> {
    if let Some(manager) = get_window_manager() {
        manager.create_chat_session()
    } else {
        Err(tauri::Error::FailedToReceiveMessage)
    }
}

pub fn open_config_window() -> tauri::Result<()> {
    if let Some(manager) = get_window_manager() {
        manager.create_config_window()
    } else {
        Err(tauri::Error::FailedToReceiveMessage)
    }
}

pub fn restore_chat_session(session_id: &str) -> tauri::Result<()> {
    if let Some(manager) = get_window_manager() {
        manager.restore_session(session_id)
    } else {
        Err(tauri::Error::FailedToReceiveMessage)
    }
}

pub fn get_all_sessions() -> Vec<ChatSession> {
    if let Some(manager) = get_window_manager() {
        manager.get_sessions()
    } else {
        Vec::new()
    }
}

pub fn get_session_counts() -> (usize, usize) {
    if let Some(manager) = get_window_manager() {
        manager.get_session_counts()
    } else {
        (0, 0)
    }
}

pub fn open_onboarding_window() -> tauri::Result<()> {
    if let Some(manager) = get_window_manager() {
        manager.create_onboarding_window()
    } else {
        Err(tauri::Error::FailedToReceiveMessage)
    }
}

pub fn close_onboarding() -> tauri::Result<()> {
    if let Some(manager) = get_window_manager() {
        manager.close_onboarding_window()
    } else {
        Err(tauri::Error::FailedToReceiveMessage)
    }
}

/// Get the focused chat session, if any
pub fn get_focused_chat_session() -> Option<String> {
    get_window_manager().and_then(|m| m.get_focused_chat_session())
}

/// Get the most recently active open chat session
pub fn get_most_recent_active_session() -> Option<String> {
    get_window_manager().and_then(|m| m.get_most_recent_active_session())
}

/// Get an active chat session to use for voice input
/// Priority: 1) Focused chat window, 2) Most recently active chat, 3) None (create new)
pub fn get_active_chat_for_voice() -> Option<String> {
    // First try to get the focused chat window
    if let Some(session_id) = get_focused_chat_session() {
        return Some(session_id);
    }

    // Fall back to most recently active session
    get_most_recent_active_session()
}

/// Get the window label for a session by its ID
pub fn get_session_window_label(session_id: &str) -> Option<String> {
    get_window_manager().and_then(|m| m.get_session_window_label(session_id))
}
