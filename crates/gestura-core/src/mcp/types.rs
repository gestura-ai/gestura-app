//! MCP Protocol Types (Version 2025-11-25)
//! Defines all types for MCP protocol compliance including capabilities,
//! lifecycle messages, prompts, and notifications.

use serde::{Deserialize, Serialize};

/// MCP Protocol version
pub const PROTOCOL_VERSION: &str = "2025-11-25";

// ============================================================================
// Server Information
// ============================================================================

/// Server information sent during initialization
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
}

impl Default for ServerInfo {
    fn default() -> Self {
        Self {
            name: "gestura".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }
}

/// Client information received during initialization
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientInfo {
    pub name: String,
    pub version: String,
}

// ============================================================================
// Capabilities
// ============================================================================

/// Server capabilities advertised during initialization
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerCapabilities {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<ToolsCapability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resources: Option<ResourcesCapability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompts: Option<PromptsCapability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logging: Option<LoggingCapability>,
}

/// Client capabilities received during initialization
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientCapabilities {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sampling: Option<SamplingCapability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roots: Option<RootsCapability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elicitation: Option<ElicitationCapability>,
}

/// Tools capability configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolsCapability {
    /// Whether the server supports tool list changes
    #[serde(default)]
    pub list_changed: bool,
}

/// Resources capability configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourcesCapability {
    /// Whether the server supports resource subscriptions
    #[serde(default)]
    pub subscribe: bool,
    /// Whether the server supports resource list changes
    #[serde(default)]
    pub list_changed: bool,
}

/// Prompts capability configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptsCapability {
    /// Whether the server supports prompt list changes
    #[serde(default)]
    pub list_changed: bool,
}

/// Logging capability configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoggingCapability {}

/// Sampling capability (client feature)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SamplingCapability {}

/// Roots capability (client feature)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RootsCapability {
    /// Whether the client supports roots list changes
    #[serde(default)]
    pub list_changed: bool,
}

/// Elicitation capability (client feature)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ElicitationCapability {}

// ============================================================================
// Lifecycle Messages
// ============================================================================

/// Initialize request parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeParams {
    pub protocol_version: String,
    pub capabilities: ClientCapabilities,
    pub client_info: ClientInfo,
}

/// Initialize response result
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResult {
    pub protocol_version: String,
    pub capabilities: ServerCapabilities,
    pub server_info: ServerInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
}

/// Ping request (empty params)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PingParams {}

/// Ping response (empty result)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PingResult {}

// ============================================================================
// Prompts
// ============================================================================

/// Prompt definition
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Prompt {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<Vec<PromptArgument>>,
}

/// Prompt argument definition
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptArgument {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub required: bool,
}

/// Prompt message content
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptMessage {
    pub role: PromptRole,
    pub content: PromptContent,
}

/// Prompt role
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PromptRole {
    User,
    Assistant,
}

/// Prompt content (text or embedded resource)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PromptContent {
    Text(TextContent),
    Resource(EmbeddedResource),
}

impl PromptContent {
    /// Get as text content if this is a text variant
    pub fn as_text(&self) -> Option<&TextContent> {
        match self {
            PromptContent::Text(t) => Some(t),
            _ => None,
        }
    }

    /// Get as embedded resource if this is a resource variant
    pub fn as_resource(&self) -> Option<&EmbeddedResource> {
        match self {
            PromptContent::Resource(r) => Some(r),
            _ => None,
        }
    }
}

/// Text content
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextContent {
    #[serde(rename = "type")]
    pub content_type: String,
    pub text: String,
}

impl TextContent {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            content_type: "text".to_string(),
            text: text.into(),
        }
    }
}

/// Embedded resource content
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddedResource {
    #[serde(rename = "type")]
    pub content_type: String,
    pub resource: ResourceReference,
}

/// Resource reference
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceReference {
    pub uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// prompts/list request params
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptsListParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

/// prompts/list response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptsListResult {
    pub prompts: Vec<Prompt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// prompts/get request params
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptsGetParams {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<std::collections::HashMap<String, String>>,
}

/// prompts/get response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptsGetResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub messages: Vec<PromptMessage>,
}

// ============================================================================
// Notifications
// ============================================================================

/// Progress notification params
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressNotification {
    /// Progress token identifying the operation
    pub progress_token: ProgressToken,
    /// Current progress value
    pub progress: f64,
    /// Total expected value (if known)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<f64>,
    /// Optional message describing current state
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Progress token (string or integer)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ProgressToken {
    String(String),
    Integer(i64),
}

impl From<String> for ProgressToken {
    fn from(s: String) -> Self {
        ProgressToken::String(s)
    }
}

impl From<i64> for ProgressToken {
    fn from(i: i64) -> Self {
        ProgressToken::Integer(i)
    }
}

/// Log level for logging notifications
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Debug,
    Info,
    Notice,
    Warning,
    Error,
    Critical,
    Alert,
    Emergency,
}

/// Logging message notification params
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoggingMessage {
    pub level: LogLevel,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logger: Option<String>,
    pub data: serde_json::Value,
}

/// Cancelled notification params
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelledNotification {
    /// The request ID that was cancelled
    pub request_id: serde_json::Value,
    /// Optional reason for cancellation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

// ============================================================================
// Tools (enhanced from existing)
// ============================================================================

/// Tool definition per MCP spec
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tool {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub input_schema: serde_json::Value,
    /// Annotations for tool behavior hints
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<ToolAnnotations>,
}

/// Tool annotations for behavior hints
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolAnnotations {
    /// Human-readable title
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Whether this tool may perform destructive actions
    #[serde(default)]
    pub destructive_hint: bool,
    /// Whether this tool is idempotent
    #[serde(default)]
    pub idempotent_hint: bool,
    /// Whether this tool opens external resources
    #[serde(default)]
    pub open_world_hint: bool,
}

/// tools/list request params
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolsListParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

/// tools/list response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolsListResult {
    pub tools: Vec<Tool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// tools/call request params
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolsCallParams {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<serde_json::Value>,
}

/// tools/call response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolsCallResult {
    pub content: Vec<ToolResultContent>,
    #[serde(default)]
    pub is_error: bool,
}

/// Tool result content
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ToolResultContent {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image")]
    Image { data: String, mime_type: String },
    #[serde(rename = "resource")]
    Resource { resource: ResourceReference },
}

// ============================================================================
// Resources (enhanced from existing)
// ============================================================================

/// Resource definition per MCP spec
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Resource {
    pub uri: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<ResourceAnnotations>,
}

/// Resource annotations
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceAnnotations {
    /// Intended audience
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audience: Option<Vec<String>>,
    /// Priority hint (0.0 to 1.0)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<f64>,
}

/// resources/list request params
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourcesListParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

/// resources/list response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourcesListResult {
    pub resources: Vec<Resource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// resources/read request params
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourcesReadParams {
    pub uri: String,
}

/// resources/read response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourcesReadResult {
    pub contents: Vec<ResourceContent>,
}

/// Resource content
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceContent {
    pub uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blob: Option<String>,
}

// ============================================================================
// JSON-RPC Error Codes (per MCP spec)
// ============================================================================

/// Standard JSON-RPC error codes
pub mod error_codes {
    /// Parse error - Invalid JSON
    pub const PARSE_ERROR: i32 = -32700;
    /// Invalid Request - Not a valid Request object
    pub const INVALID_REQUEST: i32 = -32600;
    /// Method not found
    pub const METHOD_NOT_FOUND: i32 = -32601;
    /// Invalid params
    pub const INVALID_PARAMS: i32 = -32602;
    /// Internal error
    pub const INTERNAL_ERROR: i32 = -32603;
}

/// MCP-specific error codes (reserved range: -32000 to -32099)
pub mod mcp_error_codes {
    /// Connection not initialized
    pub const NOT_INITIALIZED: i32 = -32002;
    /// Already initialized
    pub const ALREADY_INITIALIZED: i32 = -32003;
    /// Resource not found
    pub const RESOURCE_NOT_FOUND: i32 = -32004;
    /// Tool not found
    pub const TOOL_NOT_FOUND: i32 = -32005;
    /// Prompt not found
    pub const PROMPT_NOT_FOUND: i32 = -32006;
    /// Request cancelled
    pub const REQUEST_CANCELLED: i32 = -32007;
}

// ============================================================================
// Session State
// ============================================================================

/// MCP session state
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SessionState {
    /// Not yet initialized
    #[default]
    Uninitialized,
    /// Initialize request received, waiting for initialized notification
    Initializing,
    /// Fully initialized and ready
    Ready,
    /// Shutdown requested
    ShuttingDown,
    /// Session closed
    Closed,
}
