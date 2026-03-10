//! Model Context Protocol implementation for Gestura.
//!
//! This crate owns Gestura's MCP protocol layer for protocol version
//! `2025-11-25`, including the protocol data model, client/server plumbing,
//! discovery and caching, connection lifecycle, notifications, prompt registry,
//! and local integration helpers.
//!
//! ## Responsibilities
//!
//! - MCP client and server implementations
//! - service discovery, caching, and registry helpers
//! - session lifecycle and notification delivery
//! - prompt resources and prompt registry access
//! - MCP-specific configuration, errors, and inspection helpers
//!
//! ## Boundary with `gestura-core`
//!
//! Higher-level agent orchestration stays in `gestura-core`. This crate focuses
//! on implementing the MCP protocol surface itself so it can evolve as a
//! cohesive domain.
//!
//! Most application code should import MCP items through `gestura_core::mcp::*`
//! unless it specifically needs to depend on this domain crate directly.

pub mod client;
pub mod cmd_utils;
pub mod config;
pub mod discovery;
pub mod integrator;
pub mod lifecycle;
pub mod notifications;
pub mod prompts;
pub mod provision;
pub mod registry;
pub mod server;
pub mod types;

// Compatibility-style local re-exports used by this domain.
// (These keep internal module paths stable while the workspace is modularized.)
pub mod error;
pub mod execution_mode;
pub mod tool_inspection;

// Re-export commonly used types (mirrors the historical `gestura_core::mcp::*` surface).
pub use client::{McpClient, McpClientRegistry, get_mcp_client_registry};
pub use config::{
    McpJsonFile, McpScope, McpServerEntry, McpTool, McpTransportType, import_claude_desktop_servers,
};
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
pub use provision::{ProvisionResult, ProvisionStatus, provision_mcp_server};
pub use registry::{
    PopularMcpServer, RegistryBrowseEntry, RegistryBrowsePage, browse_mcp_registry,
    list_popular_mcp_servers, normalize_mcp_server_name,
};
pub use server::{
    JsonRpcError, JsonRpcRequest, JsonRpcResponse, McpRequestContext, McpResourceHandler,
    McpServer, McpToolHandler,
};
pub use types::{
    CancelledNotification, ClientCapabilities, ClientInfo, EmbeddedResource, InitializeParams,
    InitializeResult, LogLevel, LoggingCapability, LoggingMessage, PROTOCOL_VERSION, PingParams,
    PingResult, ProgressNotification, ProgressToken, Prompt, PromptArgument, PromptContent,
    PromptMessage, PromptRole, PromptsCapability, PromptsGetParams, PromptsGetResult,
    PromptsListParams, PromptsListResult, Resource, ResourceAnnotations, ResourceContent,
    ResourceReference, ResourcesCapability, ResourcesListParams, ResourcesListResult,
    ResourcesReadParams, ResourcesReadResult, ServerCapabilities, ServerInfo, SessionState,
    TextContent, Tool, ToolAnnotations, ToolResultContent, ToolsCallParams, ToolsCallResult,
    ToolsCapability, ToolsListParams, ToolsListResult, error_codes, mcp_error_codes,
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
