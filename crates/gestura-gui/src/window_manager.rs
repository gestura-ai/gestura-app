//! Window and session management for Gestura
//! Handles chat sessions, window lifecycle, and session restoration

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};
use uuid::Uuid;

/// Returns the path for storing GUI session history: ~/.gestura/gui_sessions.json
fn sessions_file_path() -> PathBuf {
    gestura_core::config::AppConfig::data_dir().join("gui_sessions.json")
}

/// Persisted session data (excludes window state which is ephemeral)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct PersistedSessions {
    /// All sessions (open flag is reset on load since windows close on app exit)
    sessions: Vec<ChatSession>,
    /// Version for future migration support
    version: u32,
}

/// A message in the conversation history
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationMessage {
    /// Message role: "user", "assistant", or "tool"
    pub role: String,
    /// Message content
    pub content: String,
    /// Tool call ID (for tool messages)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Thinking content (for extended thinking)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
    /// Message timestamp
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// Source of the message: "text" or "voice"
    pub source: MessageSource,
}

/// Source of a message
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MessageSource {
    /// Text input from chat
    Text,
    /// Voice input (transcribed)
    Voice,
    /// System-generated
    System,
}

impl Default for MessageSource {
    fn default() -> Self {
        Self::Text
    }
}

/// Tool call record for session history
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionToolCall {
    /// Tool call ID
    pub id: String,
    /// Tool name
    pub name: String,
    /// Tool arguments (JSON string)
    pub arguments: String,
    /// Tool result
    pub result: String,
    /// Whether the call succeeded
    pub success: bool,
    /// Duration in milliseconds
    pub duration_ms: u64,
    /// Timestamp
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Session-scoped LLM configuration
/// When set, overrides the global config for this session only
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionLlmConfig {
    /// Override LLM provider for this session (e.g., "openai", "anthropic", "ollama")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Override model for this session
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// Permission level for session tool execution
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SessionPermissionLevel {
    /// Read-only access - no file writes, no shell commands
    Sandbox,
    /// Ask before write operations (default)
    #[default]
    Restricted,
    /// Full access - all operations allowed without confirmation
    Full,
}

/// Session-scoped tool availability settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionToolSettings {
    /// Permission level for this session
    #[serde(default)]
    pub permission_level: SessionPermissionLevel,
    /// Enabled tools for this session (tool name -> enabled)
    #[serde(default)]
    pub enabled_tools: std::collections::HashMap<String, bool>,
}

impl Default for SessionToolSettings {
    fn default() -> Self {
        let mut enabled_tools = std::collections::HashMap::new();
        // Default enabled tools
        enabled_tools.insert("file_read".to_string(), true);
        enabled_tools.insert("file_write".to_string(), true);
        enabled_tools.insert("shell".to_string(), true);
        enabled_tools.insert("web_search".to_string(), false);
        enabled_tools.insert("code_analysis".to_string(), true);
        Self {
            permission_level: SessionPermissionLevel::default(),
            enabled_tools,
        }
    }
}

/// Unified session state for both voice and text inputs
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionState {
    /// Conversation history (shared between voice and text)
    pub messages: Vec<ConversationMessage>,
    /// Tool call history
    pub tool_calls: Vec<SessionToolCall>,
    /// Total tokens used in this session
    pub total_tokens: u64,
    /// Last context cache key (for smart context reduction)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_cache_key: Option<String>,
    /// Workspace directory for sandboxed file/shell operations
    /// All file operations and tool calls are scoped to this directory
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_dir: Option<PathBuf>,
    /// Session-scoped LLM configuration (overrides global config when set)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm_config: Option<SessionLlmConfig>,
    /// Session-scoped tool and permission settings
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_settings: Option<SessionToolSettings>,
}

impl SessionState {
    /// Create a new session state with a workspace directory
    pub fn with_workspace(workspace_dir: PathBuf) -> Self {
        Self {
            workspace_dir: Some(workspace_dir),
            ..Default::default()
        }
    }

    /// Set the workspace directory
    pub fn set_workspace(&mut self, workspace_dir: PathBuf) {
        self.workspace_dir = Some(workspace_dir);
    }

    /// Get the workspace directory
    pub fn get_workspace(&self) -> Option<&PathBuf> {
        self.workspace_dir.as_ref()
    }
}

impl SessionState {
    /// Add a user message
    pub fn add_user_message(&mut self, content: &str, source: MessageSource) {
        self.messages.push(ConversationMessage {
            role: "user".to_string(),
            content: content.to_string(),
            tool_call_id: None,
            thinking: None,
            timestamp: chrono::Utc::now(),
            source,
        });
    }

    /// Add an assistant message
    pub fn add_assistant_message(&mut self, content: &str, thinking: Option<String>) {
        self.messages.push(ConversationMessage {
            role: "assistant".to_string(),
            content: content.to_string(),
            tool_call_id: None,
            thinking,
            timestamp: chrono::Utc::now(),
            source: MessageSource::System,
        });
    }

    /// Add a tool result message
    pub fn add_tool_message(&mut self, tool_call_id: &str, content: &str) {
        self.messages.push(ConversationMessage {
            role: "tool".to_string(),
            content: content.to_string(),
            tool_call_id: Some(tool_call_id.to_string()),
            thinking: None,
            timestamp: chrono::Utc::now(),
            source: MessageSource::System,
        });
    }

    /// Record a tool call
    pub fn record_tool_call(&mut self, call: SessionToolCall) {
        self.tool_calls.push(call);
    }

    /// Get recent messages for LLM context
    pub fn get_recent_messages(&self, limit: usize) -> Vec<&ConversationMessage> {
        let start = self.messages.len().saturating_sub(limit);
        self.messages.iter().skip(start).collect()
    }

    /// Convert to Message format for pipeline
    pub fn to_pipeline_messages(&self) -> Vec<gestura_core::Message> {
        self.messages
            .iter()
            .map(|m| gestura_core::Message {
                role: m.role.clone(),
                content: m.content.clone(),
                tool_call_id: m.tool_call_id.clone(),
                thinking: m.thinking.clone(),
            })
            .collect()
    }
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
        let path = sessions_file_path();
        if !path.exists() {
            tracing::info!("No persisted sessions file found at {:?}", path);
            return;
        }

        match std::fs::read_to_string(&path) {
            Ok(json) => match serde_json::from_str::<PersistedSessions>(&json) {
                Ok(persisted) => {
                    let mut sessions = self.sessions.lock().unwrap();
                    for mut session in persisted.sessions {
                        // Mark all loaded sessions as closed (windows don't survive app restart)
                        session.is_open = false;
                        session.window_label = None;
                        sessions.insert(session.id.clone(), session);
                    }
                    let count = sessions.len();
                    drop(sessions);

                    tracing::info!("Loaded {} persisted sessions from {:?}", count, path);

                    // Notify tray to rebuild menu with loaded sessions
                    let _ = self.app.emit("sessions-changed", ());
                }
                Err(e) => {
                    tracing::warn!("Failed to parse persisted sessions: {}", e);
                }
            },
            Err(e) => {
                tracing::warn!("Failed to read persisted sessions file: {}", e);
            }
        }
    }

    /// Save all sessions to disk for persistence across app restarts
    pub fn save_sessions_to_disk(&self) {
        let sessions = self.sessions.lock().unwrap();
        let session_list: Vec<ChatSession> = sessions.values().cloned().collect();
        drop(sessions);

        if session_list.is_empty() {
            // Don't create empty file; remove existing one if present
            let path = sessions_file_path();
            if path.exists() {
                let _ = std::fs::remove_file(&path);
                tracing::debug!("Removed empty sessions file");
            }
            return;
        }

        let persisted = PersistedSessions {
            sessions: session_list,
            version: 1,
        };

        let path = sessions_file_path();
        if let Some(parent) = path.parent()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            tracing::error!("Failed to create sessions directory: {}", e);
            return;
        }

        match serde_json::to_string_pretty(&persisted) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&path, json) {
                    tracing::error!("Failed to write sessions file: {}", e);
                } else {
                    tracing::info!("Saved {} sessions to {:?}", persisted.sessions.len(), path);
                }
            }
            Err(e) => {
                tracing::error!("Failed to serialize sessions: {}", e);
            }
        }
    }

    /// Create a new chat session and window
    pub fn create_chat_session(&self) -> tauri::Result<String> {
        let session_id = Uuid::new_v4().to_string();
        let window_label = format!("chat-{}", session_id);

        tracing::info!("Creating new chat session: {}", session_id);

        // Default to a reasonable workspace so file tools can answer questions like
        // "what's in the project directory" without requiring an explicit workspace pick.
        // Users can still override this via pick_workspace_directory.
        //
        // Priority order:
        // 1. Detected project directory (has .git, Cargo.toml, etc.)
        // 2. User's home directory
        // 3. System temp directory (last resort)
        let default_workspace = get_project_directory()
            .or_else(dirs::home_dir)
            .or_else(|| std::env::temp_dir().canonicalize().ok());

        let session_state = if let Some(ref workspace) = default_workspace {
            tracing::info!(
                session_id = %session_id,
                workspace = %workspace.display(),
                "Session initialized with default workspace"
            );
            SessionState::with_workspace(workspace.clone())
        } else {
            tracing::warn!(
                session_id = %session_id,
                "No default workspace available - session will have no working directory"
            );
            SessionState::default()
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
            drop(sessions);

            // Persist the restored session state
            self.save_sessions_to_disk();

            // Emit event to notify tray that sessions changed
            let _ = self.app.emit("sessions-changed", ());
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
            session.state.set_workspace(workspace_dir);
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

/// Set the session LLM model (overrides global config for this session)
pub fn set_session_llm_model(session_id: &str, model: String) {
    tracing::info!(
        session_id = %session_id,
        model = %model,
        "set_session_llm_model called"
    );

    if let Some(manager) = get_window_manager() {
        let mut sessions = manager.sessions.lock().unwrap();
        if let Some(session) = sessions.get_mut(session_id) {
            let llm_config = session
                .state
                .llm_config
                .get_or_insert_with(Default::default);
            llm_config.model = Some(model.clone());
            tracing::info!(
                session_id = %session_id,
                model = %model,
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

/// Get the session tool settings for a session
pub fn get_session_tool_settings(session_id: &str) -> SessionToolSettings {
    get_session_state(session_id)
        .and_then(|s| s.tool_settings)
        .unwrap_or_default()
}

/// Set the session permission level
pub fn set_session_permission_level(session_id: &str, level: SessionPermissionLevel) {
    if let Some(manager) = get_window_manager() {
        let mut sessions = manager.sessions.lock().unwrap();
        if let Some(session) = sessions.get_mut(session_id) {
            let tool_settings = session
                .state
                .tool_settings
                .get_or_insert_with(Default::default);
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
                .get_or_insert_with(Default::default);
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
    match settings.permission_level {
        SessionPermissionLevel::Sandbox => !is_write_operation,
        SessionPermissionLevel::Restricted => true, // Will prompt for confirmation
        SessionPermissionLevel::Full => true,
    }
}

/// Check if confirmation is required for an action
pub fn requires_confirmation(session_id: &str, is_write_operation: bool) -> bool {
    let settings = get_session_tool_settings(session_id);
    match settings.permission_level {
        SessionPermissionLevel::Sandbox => false, // Blocked, no confirmation needed
        SessionPermissionLevel::Restricted => is_write_operation,
        SessionPermissionLevel::Full => false,
    }
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
