//! Window and session management for Gestura
//! Handles chat sessions, window lifecycle, and session restoration

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};
use uuid::Uuid;

use gestura_core::chat_sessions::{ChatSessionStore, FileChatSessionStore, SessionFilter};

/// Returns the path for the legacy GUI session history file: `~/.gestura/gui_sessions.json`.
///
/// The GUI previously persisted all sessions in a single JSON file. As part of the
/// Core-First migration (Phase 2), the GUI now uses the unified core store
/// (`FileChatSessionStore`) which persists one JSON file per session in
/// `AppConfig::data_dir()/chat_sessions/`.
///
/// This path remains only to support one-time migration from the legacy format.
fn legacy_sessions_file_path() -> PathBuf {
    gestura_core::config::AppConfig::data_dir().join("gui_sessions.json")
}

/// Legacy persisted session data (excludes window state which is ephemeral).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct LegacyPersistedSessions {
    /// All sessions (open flag is reset on load since windows close on app exit)
    sessions: Vec<ChatSession>,
    /// Version for future migration support
    version: u32,
}

/// Return the default core-backed chat session store.
///
/// Keeping this as a helper ensures the GUI remains a thin adapter over the
/// unified persistence implementation in `gestura-core`.
fn session_store() -> FileChatSessionStore {
    FileChatSessionStore::new_default()
}

/// Convert the GUI session view-model into the persisted core session model.
///
/// The GUI maintains ephemeral window state (`is_open`, `window_label`) and a
/// derived `message_count` for UI display. Persistence is handled by the core
/// `ChatSession` type only.
fn to_core_session(session: &ChatSession) -> gestura_core::chat_sessions::ChatSession {
    gestura_core::chat_sessions::ChatSession {
        id: session.id.clone(),
        title: session.title.clone(),
        created_at: session.created_at,
        last_active: session.last_active,
        model: session
            .state
            .llm_config
            .as_ref()
            .and_then(|cfg| cfg.model.clone()),
        state: session.state.clone(),
    }
}

/// Convert the persisted core session model into the GUI session view-model.
///
/// Windows do not survive app restarts, so all loaded sessions are marked as
/// closed and their `window_label` is cleared.
fn from_core_session(session: gestura_core::chat_sessions::ChatSession) -> ChatSession {
    let message_count = session.state.messages.len();
    ChatSession {
        id: session.id,
        title: session.title,
        created_at: session.created_at,
        last_active: session.last_active,
        is_open: false,
        window_label: None,
        message_count,
        state: session.state,
    }
}

/// Sanitize potentially invalid persisted (provider, model) overrides.
///
/// This is defensive against historic bug states such as `provider=openai` with
/// a non-OpenAI model value.
fn sanitize_session_llm_override(
    session_id: &str,
    state: &mut SessionState,
    global_llm_provider: &str,
) -> bool {
    let Some(ref mut llm_cfg) = state.llm_config else {
        return false;
    };

    let mut repaired = false;
    if let Some(ref model) = llm_cfg.model {
        let effective_provider = llm_cfg.provider.as_deref().unwrap_or(global_llm_provider);
        if !crate::llm_validation::is_model_compatible_with_provider(effective_provider, model) {
            tracing::warn!(
                session_id = %session_id,
                provider = %effective_provider,
                model = %model,
                "Clearing incompatible persisted session LLM model override"
            );
            llm_cfg.model = None;
            repaired = true;
        }
    }

    // If both fields are now empty, clear the override container.
    if llm_cfg.provider.is_none() && llm_cfg.model.is_none() {
        state.llm_config = None;
    }

    repaired
}

/// One-time migration from the legacy `gui_sessions.json` file into the unified
/// core store directory.
///
/// Returns the loaded core sessions if migration succeeded (even partially).
fn migrate_legacy_gui_sessions_to_core(
    store: &FileChatSessionStore,
    global_llm_provider: &str,
) -> Vec<gestura_core::chat_sessions::ChatSession> {
    let path = legacy_sessions_file_path();
    if !path.exists() {
        return Vec::new();
    }

    let mut migrated = Vec::new();
    match std::fs::read_to_string(&path) {
        Ok(json) => match serde_json::from_str::<LegacyPersistedSessions>(&json) {
            Ok(persisted) => {
                for mut session in persisted.sessions {
                    // Windows don't survive app restart.
                    session.is_open = false;
                    session.window_label = None;
                    session.message_count = session.state.messages.len();

                    // Sanitize LLM overrides before persisting.
                    let _ = sanitize_session_llm_override(
                        &session.id,
                        &mut session.state,
                        global_llm_provider,
                    );

                    let core_session = to_core_session(&session);
                    match store.save(&core_session) {
                        Ok(()) => migrated.push(core_session),
                        Err(e) => tracing::warn!(
                            session_id = %session.id,
                            error = %e,
                            "Failed to migrate legacy GUI session to core store"
                        ),
                    }
                }

                // Best-effort cleanup: remove legacy file after migration.
                if let Err(e) = std::fs::remove_file(&path) {
                    tracing::debug!(error = %e, path = %path.display(), "Failed to remove legacy sessions file");
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, path = %path.display(), "Failed to parse legacy GUI sessions file");
            }
        },
        Err(e) => {
            tracing::warn!(error = %e, path = %path.display(), "Failed to read legacy GUI sessions file");
        }
    }

    migrated
}

/// Core-First: the GUI re-exports the unified chat session model from `gestura-core`.
///
/// This keeps the Tauri layer thin while preserving the existing public module paths
/// (`crate::window_manager::ConversationMessage`, etc.) used by backend commands.
pub use gestura_core::chat_sessions::{
    ConversationMessage, MessageSource, SessionLlmConfig, SessionPermissionLevel, SessionState,
    SessionToolCall, SessionToolSettings, SessionVoiceConfig,
};

/// Default session tool settings derived from the global app configuration.
///
/// This is used in the GUI layer to preserve prior behavior where newly-created
/// sessions inherited default enabled tools and permission level from config.
fn default_session_tool_settings() -> SessionToolSettings {
    let config = gestura_core::config::AppConfig::load();
    SessionToolSettings::from_global_config(&config)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatSession {
    pub id: String,
    pub title: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub last_active: chrono::DateTime<chrono::Utc>,
    pub is_open: bool,
    pub window_label: Option<String>,
    pub message_count: usize,
    /// Unified session state (conversation history, tool calls, etc.)
    #[serde(default)]
    pub state: SessionState,
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

    /// Load persisted sessions from disk
    /// Called during initialization to restore session history
    pub fn load_persisted_sessions(&self) {
        // Load global LLM provider once so we can validate persisted session overrides.
        // This is a startup path; using the synchronous config load here is fine.
        let global_llm_provider = gestura_core::config::AppConfig::load().llm.primary;
        let store = session_store();

        // 1) Prefer loading from the unified core store.
        let mut loaded_core_sessions: Vec<gestura_core::chat_sessions::ChatSession> = Vec::new();
        match store.list(SessionFilter::All) {
            Ok(infos) => {
                for info in infos {
                    match store.load(&info.id) {
                        Ok(session) => loaded_core_sessions.push(session),
                        Err(e) => tracing::warn!(
                            session_id = %info.id,
                            error = %e,
                            "Failed to load persisted session from core store"
                        ),
                    }
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "Failed to list persisted sessions from core store");
            }
        }

        // 2) If the core store is empty, attempt a one-time migration from the legacy file.
        if loaded_core_sessions.is_empty() {
            loaded_core_sessions =
                migrate_legacy_gui_sessions_to_core(&store, global_llm_provider.as_str());
        }

        if loaded_core_sessions.is_empty() {
            tracing::info!("No persisted sessions found");
            return;
        }

        // Sanitize sessions as we load them.
        let mut repaired_any = false;
        let mut sessions_for_ui: Vec<ChatSession> = Vec::new();
        for mut core_session in loaded_core_sessions {
            repaired_any |= sanitize_session_llm_override(
                &core_session.id,
                &mut core_session.state,
                global_llm_provider.as_str(),
            );
            sessions_for_ui.push(from_core_session(core_session));
        }

        // Store in-memory sessions.
        {
            let mut sessions = self.sessions.lock().unwrap();
            for session in sessions_for_ui {
                sessions.insert(session.id.clone(), session);
            }
        }

        let count = self.sessions.lock().unwrap().len();
        tracing::info!("Loaded {} persisted sessions from core store", count);

        if repaired_any {
            // Persist repairs so the invalid state does not reappear on next launch.
            self.save_sessions_to_disk();
            tracing::info!("Persisted session LLM config repairs to disk");
        }

        // Notify tray to rebuild menu with loaded sessions
        let _ = self.app.emit("sessions-changed", ());
    }

    /// Save all sessions to disk for persistence across app restarts
    pub fn save_sessions_to_disk(&self) {
        let store = session_store();
        let sessions = self.sessions.lock().unwrap();
        let session_list: Vec<ChatSession> = sessions.values().cloned().collect();
        drop(sessions);

        // If there are no in-memory sessions, remove all persisted sessions.
        if session_list.is_empty() {
            if let Ok(infos) = store.list(SessionFilter::All) {
                for info in infos {
                    let _ = store.delete(&info.id);
                }
            }
            // Best-effort cleanup: also remove legacy file if present.
            let legacy = legacy_sessions_file_path();
            if legacy.exists() {
                let _ = std::fs::remove_file(&legacy);
            }
            tracing::debug!("Removed all persisted sessions (none in memory)");
            return;
        }

        let mut saved = 0usize;
        for session in &session_list {
            let core_session = to_core_session(session);
            match store.save(&core_session) {
                Ok(()) => saved += 1,
                Err(e) => tracing::error!(
                    session_id = %session.id,
                    error = %e,
                    "Failed to persist session to core store"
                ),
            }
        }

        tracing::info!("Saved {} sessions to core store", saved);
    }

    /// Create a new chat session and window
    pub fn create_chat_session(&self) -> tauri::Result<String> {
        let session_id = Uuid::new_v4().to_string();
        let window_label = format!("chat-{}", session_id);

        tracing::info!("Creating new chat session: {}", session_id);

        // Default to a session-specific workspace directory:
        // ~/.gestura/sessions/{session_uuid}/
        // This ensures each session has its own isolated workspace for file operations.
        // Users can still override this via pick_workspace_directory.
        let session_workspace = dirs::home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join(".gestura")
            .join("sessions")
            .join(&session_id);

        // Create the session workspace directory if it doesn't exist
        if let Err(e) = std::fs::create_dir_all(&session_workspace) {
            tracing::warn!(
                session_id = %session_id,
                error = %e,
                path = %session_workspace.display(),
                "Failed to create session workspace directory"
            );
        }

        let session_state = if session_workspace.exists() {
            tracing::info!(
                session_id = %session_id,
                workspace = %session_workspace.display(),
                "Session initialized with dedicated workspace"
            );
            SessionState::with_workspace(session_workspace)
        } else {
            // Fallback: try project directory, home, or temp
            let fallback_workspace = get_project_directory()
                .or_else(dirs::home_dir)
                .or_else(|| std::env::temp_dir().canonicalize().ok());

            if let Some(ref workspace) = fallback_workspace {
                tracing::info!(
                    session_id = %session_id,
                    workspace = %workspace.display(),
                    "Session initialized with fallback workspace"
                );
                SessionState::with_workspace(workspace.clone())
            } else {
                tracing::warn!(
                    session_id = %session_id,
                    "No workspace available - session will have no working directory"
                );
                SessionState::default()
            }
        };

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
            state: session_state,
        };

        // Store the session
        {
            let mut sessions = self.sessions.lock().unwrap();
            sessions.insert(session_id.clone(), session);
        }

        // Create the window
        self.create_chat_window(&session_id, &window_label)?;

        // Persist sessions to disk
        self.save_sessions_to_disk();

        // Emit event to notify tray that sessions changed
        let _ = self.app.emit("sessions-changed", ());
        tracing::info!("Emitted sessions-changed event (new session created)");

        Ok(session_id)
    }

    /// Create a chat window for a session
    fn create_chat_window(&self, session_id: &str, window_label: &str) -> tauri::Result<()> {
        // Include the session_id in the URL so the frontend can route events/state
        // correctly per window (avoids cross-window event bleed and enables
        // session-scoped history).
        let chat_url = format!("chat.html?session_id={}", session_id);
        let window =
            WebviewWindowBuilder::new(&self.app, window_label, WebviewUrl::App(chat_url.into()))
                .title("Gestura Chat")
                .inner_size(800.0, 600.0)
                .center()
                .resizable(true)
                .decorations(true)
                .visible(true)
                .focused(true) // Ensure window gets focus when created
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

                // Persist sessions to disk (session is now marked as closed)
                save_sessions();

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

    /// Restore a closed chat session or focus an already-open session
    pub fn restore_session(&self, session_id: &str) -> tauri::Result<()> {
        let session = {
            let sessions = self.sessions.lock().unwrap();
            sessions.get(session_id).cloned()
        };

        if let Some(mut session) = session {
            if session.is_open {
                // Session is already open - focus its window
                if let Some(ref window_label) = session.window_label
                    && let Some(window) = self.app.get_webview_window(window_label)
                {
                    let _ = window.show();
                    let _ = window.set_focus();
                    tracing::info!("Focused existing session window: {}", session_id);
                }
            } else {
                // Session is closed - restore it
                let window_label = format!("chat-{}", session_id);
                self.create_chat_window(session_id, &window_label)?;

                // Update session
                session.is_open = true;
                session.window_label = Some(window_label);
                session.last_active = chrono::Utc::now();

                let mut sessions = self.sessions.lock().unwrap();
                sessions.insert(session_id.to_string(), session);
                drop(sessions);

                // Persist the restored session state
                self.save_sessions_to_disk();

                // Emit event to notify tray that sessions changed
                let _ = self.app.emit("sessions-changed", ());
                tracing::info!("Restored session: {}", session_id);
            }
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

    /// Get the session id associated with a given window label.
    ///
    /// This is primarily used by Tauri commands to infer session context from the
    /// calling window (by label) when the frontend omits `sessionId`.
    ///
    /// Resolution order:
    /// 1) The internal `windows` registry (most robust)
    /// 2) Parsing the app's chat label convention: `chat-{session_id}`
    pub fn get_session_id_for_window_label(&self, window_label: &str) -> Option<String> {
        // Prefer the explicit window registry.
        let from_registry = self
            .windows
            .lock()
            .unwrap()
            .get(window_label)
            .and_then(|info| info.session_id.clone());
        if from_registry.is_some() {
            return from_registry;
        }

        // Fallback: parse labels generated by this app's naming convention.
        window_label
            .strip_prefix("chat-")
            .map(|sid| sid.to_string())
    }

    /// Get the session state for a session
    pub fn get_session_state(&self, session_id: &str) -> Option<SessionState> {
        let sessions = self.sessions.lock().unwrap();
        sessions.get(session_id).map(|s| s.state.clone())
    }

    /// Add a user message to a session (from text or voice)
    pub fn add_user_message(&self, session_id: &str, content: &str, source: MessageSource) {
        let mut sessions = self.sessions.lock().unwrap();
        if let Some(session) = sessions.get_mut(session_id) {
            session.state.add_user_message(content, source);
            session.message_count = session.state.messages.len();
            session.last_active = chrono::Utc::now();
        }
    }

    /// Add an assistant message to a session
    pub fn add_assistant_message(&self, session_id: &str, content: &str, thinking: Option<String>) {
        let mut sessions = self.sessions.lock().unwrap();
        if let Some(session) = sessions.get_mut(session_id) {
            session.state.add_assistant_message(content, thinking);
            session.message_count = session.state.messages.len();
            session.last_active = chrono::Utc::now();
        }
        drop(sessions);

        // Persist after assistant response (marks end of a conversation turn)
        self.save_sessions_to_disk();
    }

    /// Add a tool result message to a session
    pub fn add_tool_message(&self, session_id: &str, tool_call_id: &str, content: &str) {
        let mut sessions = self.sessions.lock().unwrap();
        if let Some(session) = sessions.get_mut(session_id) {
            session.state.add_tool_message(tool_call_id, content);
            session.message_count = session.state.messages.len();
            session.last_active = chrono::Utc::now();
        }
    }

    /// Record a tool call in a session
    pub fn record_tool_call(&self, session_id: &str, call: SessionToolCall) {
        let mut sessions = self.sessions.lock().unwrap();
        if let Some(session) = sessions.get_mut(session_id) {
            session.state.record_tool_call(call);
            session.last_active = chrono::Utc::now();
        }
    }

    /// Update token count for a session
    pub fn update_token_count(&self, session_id: &str, tokens: u64) {
        let mut sessions = self.sessions.lock().unwrap();
        if let Some(session) = sessions.get_mut(session_id) {
            session.state.total_tokens += tokens;
        }
    }

    /// Get conversation history for pipeline
    pub fn get_pipeline_messages(&self, session_id: &str) -> Vec<gestura_core::Message> {
        let sessions = self.sessions.lock().unwrap();
        sessions
            .get(session_id)
            .map(|s| s.state.to_pipeline_messages())
            .unwrap_or_default()
    }
}

// Global window manager instance
lazy_static::lazy_static! {
    static ref WINDOW_MANAGER: Mutex<Option<WindowManager>> = Mutex::new(None);
}

/// Initialize the global window manager
pub fn init_window_manager(app: AppHandle) {
    let manager = WindowManager::new(app);

    // Load persisted sessions from disk before making manager available
    manager.load_persisted_sessions();

    // Get count before moving into global
    let session_count = manager.sessions.lock().unwrap().len();

    let mut global_manager = WINDOW_MANAGER.lock().unwrap();
    *global_manager = Some(manager);
    tracing::info!(
        "Window manager initialized with {} persisted sessions",
        session_count
    );
}

/// Save all sessions to disk (public function for use from closures/callbacks)
pub fn save_sessions() {
    if let Some(manager) = get_window_manager() {
        manager.save_sessions_to_disk();
    }
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

/// Try to resolve a session id from a Tauri window label.
///
/// This is a convenience wrapper over [`WindowManager::get_session_id_for_window_label`].
pub fn get_session_id_for_window_label(window_label: &str) -> Option<String> {
    get_window_manager().and_then(|m| m.get_session_id_for_window_label(window_label))
}

/// Get the window label for a session by its ID
pub fn get_session_window_label(session_id: &str) -> Option<String> {
    get_window_manager().and_then(|m| m.get_session_window_label(session_id))
}

/// Get session state for a session
pub fn get_session_state(session_id: &str) -> Option<SessionState> {
    get_window_manager().and_then(|m| m.get_session_state(session_id))
}

/// Get the workspace directory for the current active session
pub fn get_active_session_workspace() -> Option<PathBuf> {
    get_active_chat_for_voice()
        .and_then(|session_id| get_session_state(&session_id))
        .and_then(|state| state.workspace_dir)
}

/// Set the workspace directory for a session
pub fn set_session_workspace(session_id: &str, workspace_dir: PathBuf) {
    if let Some(manager) = get_window_manager() {
        let mut sessions = manager.sessions.lock().unwrap();
        if let Some(session) = sessions.get_mut(session_id) {
            session.state.workspace_dir = Some(workspace_dir);
        }
    }
}

/// Get the session LLM config for a session
pub fn get_session_llm_config(session_id: &str) -> Option<SessionLlmConfig> {
    let result = get_session_state(session_id).and_then(|s| s.llm_config);
    tracing::debug!(
        session_id = %session_id,
        config = ?result,
        "get_session_llm_config returning"
    );
    result
}

/// Set the session LLM provider (overrides global config for this session)
pub fn set_session_llm_provider(session_id: &str, provider: String) {
    tracing::info!(
        session_id = %session_id,
        provider = %provider,
        "set_session_llm_provider called"
    );

    if let Some(manager) = get_window_manager() {
        let mut sessions = manager.sessions.lock().unwrap();
        if let Some(session) = sessions.get_mut(session_id) {
            let llm_config = session
                .state
                .llm_config
                .get_or_insert_with(Default::default);
            llm_config.provider = Some(provider.clone());
            // Provider and model are tightly coupled. If the user switches providers but we keep a
            // session-scoped model override from the previous provider, the next request can
            // silently keep using the stale model (or error) even though the UI shows the new
            // provider.
            //
            // Clearing the model override ensures the effective model falls back to the global
            // default for the selected provider (or the UI can immediately set a new session model).
            llm_config.model = None;
            tracing::info!(
                session_id = %session_id,
                provider = %provider,
                final_config = ?session.state.llm_config,
                "Session LLM provider set successfully"
            );
        } else {
            tracing::warn!(
                session_id = %session_id,
                "Session not found when setting LLM provider"
            );
        }
    } else {
        tracing::error!("Window manager not available when setting session LLM provider");
    }
}

/// Set the session LLM model (overrides global config for this session).
///
/// This performs a best-effort compatibility check between the current effective
/// provider (session override provider if present, otherwise global default) and
/// the requested `model`. If the model appears to belong to a different provider
/// (e.g. `provider=openai` + `model=grok-2`), this returns an error and does not
/// persist the invalid combination.
pub fn set_session_llm_model(session_id: &str, model: String) -> Result<(), String> {
    tracing::info!(
        session_id = %session_id,
        model = %model,
        "set_session_llm_model called"
    );

    if let Some(manager) = get_window_manager() {
        let mut sessions = manager.sessions.lock().unwrap();
        if let Some(session) = sessions.get_mut(session_id) {
            let trimmed_model = model.trim().to_string();
            if trimmed_model.is_empty() {
                // Treat empty model as clearing the override.
                if let Some(ref mut llm_cfg) = session.state.llm_config {
                    llm_cfg.model = None;
                    if llm_cfg.provider.is_none() {
                        session.state.llm_config = None;
                    }
                }
                return Ok(());
            }

            let llm_config = session
                .state
                .llm_config
                .get_or_insert_with(Default::default);

            // Determine effective provider to validate against.
            let effective_provider = llm_config
                .provider
                .as_deref()
                .map(|s| s.to_string())
                .unwrap_or_else(|| gestura_core::config::AppConfig::load().llm.primary);

            crate::llm_validation::validate_model_for_provider(
                &effective_provider,
                &trimmed_model,
            )?;
            llm_config.model = Some(trimmed_model.clone());
            tracing::info!(
                session_id = %session_id,
                model = %trimmed_model,
                final_config = ?session.state.llm_config,
                "Session LLM model set successfully"
            );
        } else {
            tracing::warn!(
                session_id = %session_id,
                "Session not found when setting LLM model"
            );
        }
    } else {
        tracing::error!("Window manager not available when setting session LLM model");
    }

    Ok(())
}

/// Clear session LLM config (revert to global config)
pub fn clear_session_llm_config(session_id: &str) {
    if let Some(manager) = get_window_manager() {
        let mut sessions = manager.sessions.lock().unwrap();
        if let Some(session) = sessions.get_mut(session_id) {
            session.state.llm_config = None;
        }
    }
}

/// Get the session voice config for a session.
///
/// Returns `None` when no session-specific override is set (use global config).
pub fn get_session_voice_config(session_id: &str) -> Option<SessionVoiceConfig> {
    let result = get_session_state(session_id).and_then(|s| s.voice_config);
    tracing::debug!(
        session_id = %session_id,
        config = ?result,
        "get_session_voice_config returning"
    );
    result
}

/// Set the session voice/STT provider (overrides global config for this session).
///
/// Clears any session-scoped STT model override when the provider changes to avoid stale model ids.
pub fn set_session_voice_provider(session_id: &str, provider: String) {
    tracing::info!(
        session_id = %session_id,
        provider = %provider,
        "set_session_voice_provider called"
    );

    if let Some(manager) = get_window_manager() {
        {
            let mut sessions = manager.sessions.lock().unwrap();
            if let Some(session) = sessions.get_mut(session_id) {
                let voice_config = session
                    .state
                    .voice_config
                    .get_or_insert_with(Default::default);
                voice_config.provider = Some(provider.clone());
                voice_config.model = None;
                tracing::info!(
                    session_id = %session_id,
                    provider = %provider,
                    final_config = ?session.state.voice_config,
                    "Session voice provider set successfully"
                );
            } else {
                tracing::warn!(
                    session_id = %session_id,
                    "Session not found when setting voice provider"
                );
            }
        }

        // Persist immediately so provider changes are reflected in the on-disk session config
        // even if the user closes the app before sending another message.
        manager.save_sessions_to_disk();
    } else {
        tracing::error!("Window manager not available when setting session voice provider");
    }
}

/// Set the session STT model (overrides global config for this session).
pub fn set_session_voice_model(session_id: &str, model: String) {
    tracing::info!(
        session_id = %session_id,
        model = %model,
        "set_session_voice_model called"
    );

    if let Some(manager) = get_window_manager() {
        {
            let mut sessions = manager.sessions.lock().unwrap();
            if let Some(session) = sessions.get_mut(session_id) {
                let voice_config = session
                    .state
                    .voice_config
                    .get_or_insert_with(Default::default);
                voice_config.model = Some(model.clone());
                tracing::info!(
                    session_id = %session_id,
                    model = %model,
                    final_config = ?session.state.voice_config,
                    "Session voice model set successfully"
                );
            } else {
                tracing::warn!(
                    session_id = %session_id,
                    "Session not found when setting voice model"
                );
            }
        }

        manager.save_sessions_to_disk();
    } else {
        tracing::error!("Window manager not available when setting session voice model");
    }
}

/// Clear session voice config (revert to global config for this session).
pub fn clear_session_voice_config(session_id: &str) {
    if let Some(manager) = get_window_manager() {
        {
            let mut sessions = manager.sessions.lock().unwrap();
            if let Some(session) = sessions.get_mut(session_id) {
                session.state.voice_config = None;
            }
        }

        manager.save_sessions_to_disk();
    }
}

/// Get the session tool settings for a session
pub fn get_session_tool_settings(session_id: &str) -> SessionToolSettings {
    get_session_state(session_id)
        .and_then(|s| s.tool_settings)
        .unwrap_or_else(default_session_tool_settings)
}

/// Set the session permission level
pub fn set_session_permission_level(session_id: &str, level: SessionPermissionLevel) {
    if let Some(manager) = get_window_manager() {
        let mut sessions = manager.sessions.lock().unwrap();
        if let Some(session) = sessions.get_mut(session_id) {
            let tool_settings = session
                .state
                .tool_settings
                .get_or_insert_with(default_session_tool_settings);
            tool_settings.permission_level = level;
        }
    }
}

/// Set whether a tool is enabled for a session
pub fn set_session_tool_enabled(session_id: &str, tool_name: &str, enabled: bool) {
    if let Some(manager) = get_window_manager() {
        let mut sessions = manager.sessions.lock().unwrap();
        if let Some(session) = sessions.get_mut(session_id) {
            let tool_settings = session
                .state
                .tool_settings
                .get_or_insert_with(default_session_tool_settings);
            tool_settings
                .enabled_tools
                .insert(tool_name.to_string(), enabled);
        }
    }
}

/// Check if a tool is enabled for a session
pub fn is_session_tool_enabled(session_id: &str, tool_name: &str) -> bool {
    get_session_tool_settings(session_id)
        .enabled_tools
        .get(tool_name)
        .copied()
        .unwrap_or(true) // Default to enabled if not explicitly set
}

/// Check if an action is allowed based on session permission level
pub fn is_action_allowed(session_id: &str, is_write_operation: bool) -> bool {
    let settings = get_session_tool_settings(session_id);
    let permission_level = settings.permission_level.to_pipeline();
    gestura_core::tools::policy::is_action_allowed(permission_level, is_write_operation)
}

/// Check if confirmation is required for an action
pub fn requires_confirmation(session_id: &str, is_write_operation: bool) -> bool {
    let settings = get_session_tool_settings(session_id);
    let permission_level = settings.permission_level.to_pipeline();
    gestura_core::tools::policy::requires_confirmation(permission_level, is_write_operation)
}

/// Add a user message to a session (from text or voice)
pub fn add_user_message(session_id: &str, content: &str, source: MessageSource) {
    if let Some(manager) = get_window_manager() {
        manager.add_user_message(session_id, content, source);
    }
}

/// Add an assistant message to a session
pub fn add_assistant_message(session_id: &str, content: &str, thinking: Option<String>) {
    if let Some(manager) = get_window_manager() {
        manager.add_assistant_message(session_id, content, thinking);
    }
}

/// Add a tool result message to a session
pub fn add_tool_message(session_id: &str, tool_call_id: &str, content: &str) {
    if let Some(manager) = get_window_manager() {
        manager.add_tool_message(session_id, tool_call_id, content);
    }
}

/// Record a tool call in a session
pub fn record_tool_call(session_id: &str, call: SessionToolCall) {
    if let Some(manager) = get_window_manager() {
        manager.record_tool_call(session_id, call);
    }
}

/// Update token count for a session
pub fn update_token_count(session_id: &str, tokens: u64) {
    if let Some(manager) = get_window_manager() {
        manager.update_token_count(session_id, tokens);
    }
}

/// Get conversation history for pipeline
pub fn get_pipeline_messages(session_id: &str) -> Vec<gestura_core::Message> {
    get_window_manager()
        .map(|m| m.get_pipeline_messages(session_id))
        .unwrap_or_default()
}

/// Open a sandboxed shell session with Gestura CLI access
///
/// This opens a terminal window with:
/// - Working directory set to the user's home or project directory
/// - Gestura environment configured
/// - Access to `gestura` CLI commands
pub fn open_shell_session() -> Result<(), crate::shell_session::ShellSessionError> {
    use crate::shell_session::{ShellSessionConfig, open_shell_session as spawn_shell};

    let mut config = ShellSessionConfig::default().with_env("GESTURA_SHELL", "1");

    // Prefer a real project directory, then the user's home directory.
    // Avoid forcing `/` as a working directory; shell_session will create a safe
    // per-session directory if we cannot find a usable path.
    if let Some(working_dir) = get_project_directory().or_else(dirs::home_dir) {
        config = config.with_working_directory(working_dir);
    }

    spawn_shell(config)
}

/// Get the current Gestura project directory
///
/// Checks for common project indicators like .gestura/, Cargo.toml, package.json
fn get_project_directory() -> Option<std::path::PathBuf> {
    // Start from current directory
    let cwd = std::env::current_dir().ok()?;

    // If the current directory is the filesystem root, treat it as unusable.
    #[cfg(unix)]
    if cwd == std::path::PathBuf::from("/") {
        return None;
    }

    // Check if current directory is a Gestura project
    if is_gestura_project(&cwd) {
        return Some(cwd);
    }

    // Check parent directories (up to 5 levels)
    let mut dir = cwd.clone();
    for _ in 0..5 {
        if let Some(parent) = dir.parent() {
            if is_gestura_project(parent) {
                return Some(parent.to_path_buf());
            }
            dir = parent.to_path_buf();
        } else {
            break;
        }
    }

    None
}

/// Check if a directory is a Gestura project
fn is_gestura_project(dir: &std::path::Path) -> bool {
    // Look for common project indicators
    let indicators = [".gestura", "Cargo.toml", "package.json", ".git"];

    indicators.iter().any(|ind| dir.join(ind).exists())
}
