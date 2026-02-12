//! MCP Lifecycle Management
//! Handles initialize/initialized handshake, ping, and shutdown.

use super::types::{
    ClientCapabilities, ClientInfo, InitializeParams, InitializeResult, LoggingCapability,
    PROTOCOL_VERSION, PingResult, PromptsCapability, ResourcesCapability, ServerCapabilities,
    ServerInfo, SessionState, ToolsCapability,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

/// Session manager for MCP lifecycle
#[derive(Debug)]
pub struct SessionManager {
    state: AtomicU8,
    client_info: std::sync::RwLock<Option<ClientInfo>>,
    client_capabilities: std::sync::RwLock<Option<ClientCapabilities>>,
    server_info: ServerInfo,
    server_capabilities: ServerCapabilities,
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionManager {
    /// Create a new session manager
    pub fn new() -> Self {
        Self {
            state: AtomicU8::new(SessionState::Uninitialized as u8),
            client_info: std::sync::RwLock::new(None),
            client_capabilities: std::sync::RwLock::new(None),
            server_info: ServerInfo::default(),
            server_capabilities: Self::default_capabilities(),
        }
    }

    /// Get default server capabilities
    fn default_capabilities() -> ServerCapabilities {
        ServerCapabilities {
            tools: Some(ToolsCapability { list_changed: true }),
            resources: Some(ResourcesCapability {
                subscribe: false,
                list_changed: true,
            }),
            prompts: Some(PromptsCapability { list_changed: true }),
            logging: Some(LoggingCapability {}),
        }
    }

    /// Get current session state
    pub fn state(&self) -> SessionState {
        match self.state.load(Ordering::SeqCst) {
            0 => SessionState::Uninitialized,
            1 => SessionState::Initializing,
            2 => SessionState::Ready,
            3 => SessionState::ShuttingDown,
            4 => SessionState::Closed,
            _ => SessionState::Uninitialized,
        }
    }

    /// Set session state
    fn set_state(&self, state: SessionState) {
        self.state.store(state as u8, Ordering::SeqCst);
    }

    /// Check if session is ready for requests
    pub fn is_ready(&self) -> bool {
        self.state() == SessionState::Ready
    }

    /// Handle initialize request
    pub fn initialize(&self, params: InitializeParams) -> Result<InitializeResult, String> {
        let current_state = self.state();
        if current_state != SessionState::Uninitialized {
            return Err(format!(
                "Cannot initialize: session is in {:?} state",
                current_state
            ));
        }

        // Validate protocol version
        if params.protocol_version != PROTOCOL_VERSION {
            tracing::warn!(
                "Client protocol version {} differs from server version {}",
                params.protocol_version,
                PROTOCOL_VERSION
            );
        }

        // Store client info
        if let Ok(mut info) = self.client_info.write() {
            *info = Some(params.client_info.clone());
        }

        // Store client capabilities
        if let Ok(mut caps) = self.client_capabilities.write() {
            *caps = Some(params.capabilities);
        }

        self.set_state(SessionState::Initializing);

        tracing::info!(
            "MCP session initializing with client: {} v{}",
            params.client_info.name,
            params.client_info.version
        );

        Ok(InitializeResult {
            protocol_version: PROTOCOL_VERSION.to_string(),
            capabilities: self.server_capabilities.clone(),
            server_info: self.server_info.clone(),
            instructions: Some(
                "Gestura is a voice-first AI assistant with haptic feedback support.".to_string(),
            ),
        })
    }

    /// Handle initialized notification (completes handshake)
    pub fn initialized(&self) -> Result<(), String> {
        let current_state = self.state();
        if current_state != SessionState::Initializing {
            return Err(format!(
                "Cannot complete initialization: session is in {:?} state",
                current_state
            ));
        }

        self.set_state(SessionState::Ready);
        tracing::info!("MCP session initialized and ready");
        Ok(())
    }

    /// Handle ping request
    pub fn ping(&self) -> PingResult {
        PingResult {}
    }

    /// Handle shutdown request
    pub fn shutdown(&self) -> Result<(), String> {
        let current_state = self.state();
        if current_state == SessionState::Closed {
            return Err("Session already closed".to_string());
        }

        self.set_state(SessionState::ShuttingDown);
        tracing::info!("MCP session shutting down");

        // Clean up resources
        if let Ok(mut info) = self.client_info.write() {
            *info = None;
        }
        if let Ok(mut caps) = self.client_capabilities.write() {
            *caps = None;
        }

        self.set_state(SessionState::Closed);
        tracing::info!("MCP session closed");
        Ok(())
    }

    /// Get client info if available
    pub fn client_info(&self) -> Option<ClientInfo> {
        self.client_info.read().ok()?.clone()
    }

    /// Get client capabilities if available
    pub fn client_capabilities(&self) -> Option<ClientCapabilities> {
        self.client_capabilities.read().ok()?.clone()
    }

    /// Get server info
    pub fn server_info(&self) -> &ServerInfo {
        &self.server_info
    }

    /// Get server capabilities
    pub fn server_capabilities(&self) -> &ServerCapabilities {
        &self.server_capabilities
    }
}

/// Create a shared session manager
pub fn create_session_manager() -> Arc<SessionManager> {
    Arc::new(SessionManager::new())
}
