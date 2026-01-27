//! MCP (Model Context Protocol) Module
//!
//! This module provides full MCP protocol compliance (Version 2025-11-25) including:
//! - **Types**: All MCP protocol types (capabilities, messages, etc.)
//! - **Lifecycle**: Initialize/initialized handshake, ping, shutdown
//! - **Prompts**: Templated messages and workflows
//! - **Notifications**: Progress, logging, and cancellation
//! - **Integrator**: Token management and tool exposure
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                     MCP Server (Gestura)                     │
//! ├─────────────────────────────────────────────────────────────┤
//! │  Lifecycle    │  Tools      │  Resources  │  Prompts        │
//! │  - initialize │  - list     │  - list     │  - list         │
//! │  - ping       │  - call     │  - read     │  - get          │
//! │  - shutdown   │             │  - subscribe│                 │
//! ├─────────────────────────────────────────────────────────────┤
//! │                    Notifications                             │
//! │  - progress   │  - logging  │  - cancelled │  - list_changed│
//! ├─────────────────────────────────────────────────────────────┤
//! │                    Transport Layer                           │
//! │  - STDIO      │  - HTTP/SSE (planned)                       │
//! └─────────────────────────────────────────────────────────────┘
//! ```

pub mod discovery;
pub mod integrator;
pub mod lifecycle;
pub mod notifications;
pub mod prompts;
pub mod server;
pub mod types;

// Re-export commonly used types
pub use discovery::{
    CacheStats as McpCacheStats, CachedTool, McpDiscoveryManager, McpServerConfig,
    ServerInfo as McpServerInfo, ServerState,
};
pub use integrator::{LocalMcp, McpIntegrator, MdhResource, TokenInfo, get_mcp, mdh_translate};
pub use lifecycle::{SessionManager, create_session_manager};
pub use notifications::{
    McpLogger, McpNotification, NotificationReceiver, NotificationSender, OperationProgress,
    ProgressTracker, create_notification_channel,
};
pub use prompts::{PromptRegistry, RegisteredPrompt};
pub use server::{
    JsonRpcError, JsonRpcRequest, JsonRpcResponse, McpRequestContext, McpResourceHandler,
    McpServer, McpToolHandler,
};
pub use types::{
    // Notifications
    CancelledNotification,
    // Capabilities
    ClientCapabilities,
    ClientInfo,
    // Text content
    EmbeddedResource,
    // Lifecycle
    InitializeParams,
    InitializeResult,
    // Logging
    LogLevel,
    LoggingCapability,
    LoggingMessage,
    // Protocol version
    PROTOCOL_VERSION,
    PingParams,
    PingResult,
    ProgressNotification,
    ProgressToken,
    // Prompts
    Prompt,
    PromptArgument,
    PromptContent,
    PromptMessage,
    PromptRole,
    PromptsCapability,
    PromptsGetParams,
    PromptsGetResult,
    PromptsListParams,
    PromptsListResult,
    // Resources
    Resource,
    ResourceAnnotations,
    ResourceContent,
    ResourceReference,
    ResourcesCapability,
    ResourcesListParams,
    ResourcesListResult,
    ResourcesReadParams,
    ResourcesReadResult,
    ServerCapabilities,
    ServerInfo,
    // Session
    SessionState,
    TextContent,
    // Tools
    Tool,
    ToolAnnotations,
    ToolResultContent,
    ToolsCallParams,
    ToolsCallResult,
    ToolsCapability,
    ToolsListParams,
    ToolsListResult,
    // Error codes
    error_codes,
    mcp_error_codes,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_protocol_version() {
        assert_eq!(PROTOCOL_VERSION, "2025-11-25");
    }

    #[test]
    fn test_server_info_default() {
        let info = ServerInfo::default();
        assert_eq!(info.name, "gestura");
        assert!(!info.version.is_empty());
    }

    #[test]
    fn test_session_lifecycle() {
        let session = SessionManager::new();
        assert_eq!(session.state(), SessionState::Uninitialized);

        let params = InitializeParams {
            protocol_version: PROTOCOL_VERSION.to_string(),
            capabilities: ClientCapabilities::default(),
            client_info: ClientInfo {
                name: "test-client".to_string(),
                version: "1.0.0".to_string(),
            },
        };

        let result = session.initialize(params).unwrap();
        assert_eq!(result.protocol_version, PROTOCOL_VERSION);
        assert_eq!(session.state(), SessionState::Initializing);

        session.initialized().unwrap();
        assert_eq!(session.state(), SessionState::Ready);
        assert!(session.is_ready());

        session.shutdown().unwrap();
        assert_eq!(session.state(), SessionState::Closed);
    }

    #[test]
    fn test_prompt_registry() {
        let registry = PromptRegistry::new();
        let prompts = registry.list();
        assert!(!prompts.is_empty());
        assert!(registry.contains("voice-command"));

        let mut args = std::collections::HashMap::new();
        args.insert("command".to_string(), "hello world".to_string());
        args.insert("context".to_string(), "testing".to_string());

        let result = registry.get("voice-command", Some(&args)).unwrap();
        assert!(
            result.messages[0]
                .content
                .as_text()
                .map(|t| t.text.contains("hello world"))
                .unwrap_or(false)
        );
    }

    #[test]
    fn test_progress_tracker() {
        let (sender, _receiver) = create_notification_channel();
        let tracker = ProgressTracker::new(sender);

        let id = tracker.start_operation("test-op".to_string(), Some(100.0));
        assert!(!tracker.is_cancelled(&id));

        tracker.update_progress(&id, 50.0, Some("Halfway".to_string()));
        tracker.cancel_operation(&id, Some("User cancelled".to_string()));
        assert!(tracker.is_cancelled(&id));
    }
}
